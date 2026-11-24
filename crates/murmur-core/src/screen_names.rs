//! Name-like candidates from the text on screen (`FieldContext::screen_words`),
//! for speech-recognition hints (D107).
//!
//! A shell sends what it reads on screen: Android ("Send on-screen words")
//! sends whole chat bubbles, top to bottom; Windows the window title's
//! capitalised words. As ASR hints (Parakeet / sherpa hotwords, whisper's
//! initial prompt) a bubble boosts only a decode that starts the whole
//! sentence, and the oldest bubbles fill the 900-char prompt budget, so
//! the names further down were cut. [`name_candidates`] keeps the names,
//! newest first. The core derives them where it builds an ASR request, for
//! mid-sentence casing and for the cleanup prompt's SCREEN NAMES line
//! (D108, only those the transcript comes near); the context keeps the raw
//! screen words, which the Hebrew script check still uses.
//!
//! Rules, generic only (no list of real names):
//! * entries are read bottom to top (newest messages first);
//! * an entry of at most [`MAX_CANDIDATE_WORDS`] words (a sender or header
//!   label) is kept, less dropped words at its ends; a label written all
//!   in capitals ("May", "Will Ashby") keeps its common words;
//! * from a longer entry: `@mentions` (without the `@`, "@orin_vale" →
//!   "orin vale", as it is said) and runs of capitalised words, emitted in
//!   pairs ("Alder Fenwick Corin Malloy" → "Alder Fenwick", "Corin Malloy");
//!   Hebrew and lower-case words of a long entry are not candidates;
//! * a word is dropped when it is a never-name label (`data/screen-labels-en.txt`:
//!   pronouns such as "I" and "You", weekdays, app labels, one-word replies), or common
//!   (`data/common-words-{en,he}.txt`) and written in lower case, in Hebrew
//!   (after its one-letter prefixes ו ה ב ל ש מ כ), or capitalised only
//!   because it starts a sentence; "May" mid-sentence is kept;
//! * tokens with digits ("10:42", "ב-10:42") are never words;
//! * case-insensitive dedupe, at most [`MAX_CANDIDATES`].

use std::collections::HashSet;
use std::sync::OnceLock;

/// Cap on candidates.
pub const MAX_CANDIDATES: usize = 50;
/// Cap on words per candidate.
pub const MAX_CANDIDATE_WORDS: usize = 3;

const COMMON_EN: &str = include_str!("../data/common-words-en.txt");
const COMMON_HE: &str = include_str!("../data/common-words-he.txt");
const LABELS_EN: &str = include_str!("../data/screen-labels-en.txt");
/// One-letter Hebrew prefixes (and, the, in, to, that, from, as).
const HE_PREFIXES: [char; 7] = ['ו', 'ה', 'ב', 'ל', 'ש', 'מ', 'כ'];

fn word_set(lists: &[&'static str]) -> HashSet<String> {
    lists
        .iter()
        .flat_map(|l| l.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_lowercase)
        .collect()
}

fn common() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| word_set(&[COMMON_EN, COMMON_HE, LABELS_EN]))
}

fn labels() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| word_set(&[LABELS_EN]))
}

fn key(w: &str) -> String {
    w.to_lowercase().replace('’', "'")
}

/// Whether `w` is in the built-in common-word lists (any case); a Hebrew
/// word also with up to two one-letter prefixes removed.
pub fn is_common(w: &str) -> bool {
    let mut k = key(w);
    for _ in 0..3 {
        if common().contains(&k) {
            return true;
        }
        let mut cs = k.chars();
        match cs.next() {
            Some(c) if HE_PREFIXES.contains(&c) && cs.clone().count() >= 2 => {
                k = cs.collect();
            }
            _ => return false,
        }
    }
    false
}

fn is_label(w: &str) -> bool {
    labels().contains(&key(w))
}

/// One word of an entry.
struct Token {
    /// One word, or a mention's words ("@orin_vale" → "orin vale").
    text: String,
    mention: bool,
    /// Digits or no letters ("10:42"): not a word, and a run stops here.
    gap: bool,
    /// Followed by `,` `.` `;` `:` `!` `?` or a closing bracket.
    breaks: bool,
    /// Followed by `.` `!` `?`: the next word starts a sentence.
    ends_sentence: bool,
}

impl Token {
    fn capitalised(&self) -> bool {
        self.text.chars().next().is_some_and(char::is_uppercase)
    }

    /// Written all in capitals ("OK", "LOL"), more than one letter.
    fn shouted(&self) -> bool {
        self.text.chars().filter(|c| c.is_alphabetic()).count() > 1
            && !self.text.chars().any(char::is_lowercase)
    }
}

fn tokens(entry: &str) -> Vec<Token> {
    entry
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            let tail = raw.trim_end_matches(['"', '\'', '’', '”', '*', '_']);
            let breaks = tail.ends_with([',', '.', ';', ':', '!', '?', ')', ']']);
            let ends_sentence = tail.ends_with(['.', '!', '?']);
            let mention = raw.starts_with('@');
            let core = raw.trim_matches(|c: char| !c.is_alphanumeric());
            let core = ["'s", "’s"]
                .iter()
                .find_map(|s| core.strip_suffix(s))
                .unwrap_or(core);
            let text = if raw == "&" {
                "and".to_string()
            } else if mention {
                core.replace(['_', '.'], " ")
                    .split_whitespace()
                    .take(MAX_CANDIDATE_WORDS)
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                core.to_string()
            };
            let gap =
                text.chars().any(|c| c.is_ascii_digit()) || !text.chars().any(char::is_alphabetic);
            Token {
                text,
                mention,
                gap,
                breaks,
                ends_sentence,
            }
        })
        .collect()
}

/// Whether `t` is no name: see the module docs. `starts` = it starts a
/// sentence; `label` = it stands in a short all-capitals label.
fn dropped(t: &Token, starts: bool, label: bool) -> bool {
    if t.gap || is_label(&t.text) {
        return true;
    }
    if t.capitalised() && !t.shouted() {
        return starts && !label && is_common(&t.text);
    }
    is_common(&t.text)
}

/// A short entry (≤ [`MAX_CANDIDATE_WORDS`] words), less dropped words at
/// its ends; `None` when nothing is left.
fn short_entry(toks: &[&Token]) -> Option<String> {
    let cased = toks.iter().filter(|t| {
        t.text
            .chars()
            .next()
            .is_some_and(|c| c.is_uppercase() || c.is_lowercase())
    });
    let label = cased.clone().count() > 0 && cased.clone().all(|t| t.capitalised());
    let keep: Vec<bool> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| !dropped(t, i == 0, label))
        .collect();
    let start = keep.iter().position(|&k| k)?;
    let end = keep.iter().rposition(|&k| k)?;
    Some(
        toks[start..=end]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The names in a long entry: mentions and capitalised runs.
fn long_entry(toks: &[Token], out: &mut Vec<String>) {
    let mut run: Vec<&str> = Vec::new();
    let flush = |run: &mut Vec<&str>, out: &mut Vec<String>| {
        for pair in run.chunks(2) {
            out.push(pair.join(" "));
        }
        run.clear();
    };
    let mut starts = true;
    for t in toks {
        let starts_here = starts;
        starts = t.ends_sentence || (starts && t.gap);
        if t.mention && !t.gap {
            flush(&mut run, out);
            if !is_label(&t.text) && !is_common(&t.text) {
                out.push(t.text.clone());
            }
            continue;
        }
        if t.capitalised() && !dropped(t, starts_here, false) {
            run.push(&t.text);
            if t.breaks {
                flush(&mut run, out);
            }
            continue;
        }
        flush(&mut run, out);
    }
    flush(&mut run, out);
}

/// Name-like candidates from `entries` (on-screen texts, top to bottom),
/// newest (bottom) first: see the module docs.
pub fn name_candidates(entries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in entries.iter().rev() {
        let toks = tokens(entry);
        let words: Vec<&Token> = toks.iter().filter(|t| !t.gap).collect();
        if words.is_empty() {
            continue;
        }
        let n: usize = words.iter().map(|t| t.text.split(' ').count()).sum();
        if n <= MAX_CANDIDATE_WORDS {
            out.extend(short_entry(&words));
        } else {
            long_entry(&toks, &mut out);
        }
    }
    let mut seen = HashSet::new();
    out.into_iter()
        .filter(|c| seen.insert(c.to_lowercase()))
        .take(MAX_CANDIDATES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn one(s: &str) -> Vec<String> {
        name_candidates(&v(&[s]))
    }

    #[test]
    fn a_bubble_gives_its_names_not_the_sentence() {
        assert_eq!(
            one("Hey Maren, did Tobiah Quell send the deck to the Lantern Works team yet?"),
            v(&["Maren", "Tobiah Quell", "Lantern Works"])
        );
    }

    #[test]
    fn mentions_lose_the_at_sign_and_plain_words_are_not_candidates() {
        assert_eq!(
            one("thanks for the review @orin_vale, merging now"),
            v(&["orin vale"])
        );
        // An underscore alone is not a mention.
        assert!(one("see the file build_notes before the review today").is_empty());
    }

    #[test]
    fn short_labels_are_kept_less_dropped_words() {
        let got = name_candidates(&v(&[
            "Online",
            "10:42",
            "Type a message",
            "Reply to Kestrel",
            "Ysolde Brannick",
        ]));
        assert_eq!(got, v(&["Ysolde Brannick", "Kestrel"]));
    }

    #[test]
    fn newest_entries_come_first() {
        let got = name_candidates(&v(&["Wren Hollis", "Pellam Oakes", "Ottoline Reyes"]));
        assert_eq!(got, v(&["Ottoline Reyes", "Pellam Oakes", "Wren Hollis"]));
    }

    #[test]
    fn long_capitalised_runs_come_in_pairs() {
        assert_eq!(
            one("minutes from Alder Fenwick Corin Malloy today"),
            v(&["Alder Fenwick", "Corin Malloy"])
        );
        assert_eq!(
            one("notes from Sab Teague Ilse came in"),
            v(&["Sab Teague", "Ilse"])
        );
    }

    #[test]
    fn ampersand_is_said_and() {
        assert_eq!(one("Harrow & Vane"), v(&["Harrow and Vane"]));
    }

    #[test]
    fn common_word_names_are_kept_mid_sentence_and_as_labels() {
        assert_eq!(one("I asked May about the venue for Saturday"), v(&["May"]));
        assert_eq!(
            one("then Will said the June draft is fine"),
            v(&["Will", "June"])
        );
        assert_eq!(
            name_candidates(&v(&["Hope", "Will Ashby"])),
            v(&["Will Ashby", "Hope"])
        );
        // Sentence-initial and lower-case common words are not names.
        assert!(one("Will you send it? May be late, hope not.").is_empty());
        assert!(one("Good morning").is_empty());
    }

    #[test]
    fn windows_title_words_drop_labels_but_keep_names() {
        // Windows sends a title's capitalised words, one per entry.
        let got = name_candidates(&v(&[
            "Inbox", "Document", "May", "Falkner", "Outlook", "Thursday",
        ]));
        assert_eq!(got, v(&["Outlook", "Falkner", "May"]));
    }

    #[test]
    fn hebrew_keeps_short_labels_with_an_uncommon_word() {
        let got = name_candidates(&v(&[
            "נויה טלמור",
            "מה קורה?",
            "היום ב-10:42",
            "תודה רבה",
            "ובסדר",
            "מישהו יכול להביא את המטען מחר בבוקר?",
        ]));
        assert_eq!(got, v(&["נויה טלמור"]));
    }

    #[test]
    fn hebrew_prefixes_are_stripped_for_the_lookup() {
        assert!(is_common("והיום"));
        assert!(is_common("שבסדר"));
        assert!(!is_common("טלמור"));
    }

    #[test]
    fn the_list_is_capped() {
        let names: Vec<String> = (0..60u8)
            .map(|i| {
                format!(
                    "Zq{}{} Yv",
                    (b'a' + i / 26) as char,
                    (b'a' + i % 26) as char
                )
            })
            .collect();
        let got = name_candidates(&names);
        assert_eq!(got.len(), MAX_CANDIDATES);
        assert_eq!(got[0], names[59]);
    }

    #[test]
    fn dedupe_is_case_insensitive() {
        let got = name_candidates(&v(&["Wren Hollis", "wren hollis", "WREN HOLLIS"]));
        assert_eq!(got, v(&["WREN HOLLIS"]));
    }

    #[test]
    fn possessives_and_punctuation_are_stripped() {
        assert_eq!(
            one("I think Brisa's (the new lead) is out until Thursday."),
            v(&["Brisa"])
        );
    }
}
