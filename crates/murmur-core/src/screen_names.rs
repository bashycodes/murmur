//! Name-like candidates from the text on screen (`FieldContext::screen_words`).
//!
//! A shell sends what it reads on screen: Android ("Send on-screen words")
//! sends whole chat bubbles, Windows the window title's capitalised words.
//! Everything downstream (Parakeet / sherpa hotwords, whisper's initial
//! prompt, the cleanup prompt's DICTIONARY line, mid-sentence casing)
//! wants the names in that text, not sentences: a bubble spent as one
//! hotword boosts only a decode that starts the whole sentence, and a few
//! bubbles fill the 900-char prompt budget. [`name_candidates`] keeps the
//! names; the core applies it where screen words enter
//! (`MurmurCore::set_context`, and the router's `set_context` so a hybrid
//! phone sends the short list to its server). It is idempotent, so a list
//! that went through it once comes back unchanged.
//!
//! Rules, generic only (no list of real names):
//! * an entry of at most [`MAX_CANDIDATE_WORDS`] words (a sender or
//!   header label) is kept whole, less common words at its ends; this is
//!   the only rule for Hebrew and other scripts without capitals;
//! * from a longer entry: `@mentions` and `user_names` (`@orin_vale`
//!   gives "orin vale", as it is said), runs of capitalised words (split
//!   into [`MAX_CANDIDATE_WORDS`]-word pieces, common words trimmed from
//!   their ends, so "Hey Maya" gives "Maya"), and, after all of those,
//!   lower-case Latin words of at least 4 letters that are not common;
//! * case-insensitive dedupe, at most [`MAX_CANDIDATES`].
//!
//! "Common" is the built-in list in `data/common-words-en.txt` (function
//! words, frequent chat words, weekdays, months, app labels) and
//! `data/common-words-he.txt` (very common Hebrew words and labels).

use std::collections::HashSet;
use std::sync::OnceLock;

/// Cap on candidates sent on.
pub const MAX_CANDIDATES: usize = 50;
/// Cap on words per candidate.
pub const MAX_CANDIDATE_WORDS: usize = 3;
/// Shortest lower-case word kept as a "not common" candidate.
const MIN_PLAIN_CHARS: usize = 4;

const COMMON_EN: &str = include_str!("../data/common-words-en.txt");
const COMMON_HE: &str = include_str!("../data/common-words-he.txt");

fn common() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        COMMON_EN
            .lines()
            .chain(COMMON_HE.lines())
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_lowercase)
            .collect()
    })
}

/// Whether `w` is in the built-in common-word lists (any case).
pub fn is_common(w: &str) -> bool {
    common().contains(&w.to_lowercase().replace('’', "'"))
}

/// One word of an entry.
struct Token {
    /// One word, or a handle's words ("@orin_vale" → "orin vale", as it
    /// is said; at most [`MAX_CANDIDATE_WORDS`]).
    text: String,
    /// An `@mention` or a `user_name`: a name whatever its case.
    handle: bool,
    /// Followed by `,` `.` `;` `:` `!` `?` or a closing bracket: a run of
    /// names stops here.
    breaks: bool,
}

impl Token {
    fn capitalised(&self) -> bool {
        self.text.chars().next().is_some_and(char::is_uppercase)
    }

    fn lower_latin(&self) -> bool {
        self.text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c == '\'')
    }
}

fn tokens(entry: &str) -> Vec<Token> {
    entry
        .split_whitespace()
        .filter_map(|raw| {
            let breaks = raw
                .trim_end_matches(['"', '\'', '’', '”', '*', '_'])
                .ends_with([',', '.', ';', ':', '!', '?', ')', ']']);
            let core = raw.trim_matches(|c: char| !c.is_alphanumeric());
            let core = ["'s", "’s"]
                .iter()
                .find_map(|s| core.strip_suffix(s))
                .unwrap_or(core);
            // A word has a letter; "10:42" and "3" are not words.
            if !core.chars().any(char::is_alphabetic) {
                return None;
            }
            let handle = raw.starts_with('@') || core.contains('_');
            let text = if handle {
                let said = core.replace(['_', '.'], " ");
                let words: Vec<&str> = said.split_whitespace().take(MAX_CANDIDATE_WORDS).collect();
                words.join(" ")
            } else {
                core.to_string()
            };
            Some(Token {
                text,
                handle,
                breaks,
            })
        })
        .collect()
}

/// `words` without common words at either end, joined; `None` when
/// nothing is left.
fn trimmed(words: &[&str]) -> Option<String> {
    let start = words.iter().position(|w| !is_common(w))?;
    let end = words.iter().rposition(|w| !is_common(w))?;
    Some(words[start..=end].join(" "))
}

/// Name-like candidates from `entries` (on-screen texts in screen
/// order): see the module docs. Idempotent.
pub fn name_candidates(entries: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for entry in entries {
        let toks = tokens(entry);
        let words: Vec<&str> = toks.iter().flat_map(|t| t.text.split(' ')).collect();
        if words.is_empty() {
            continue;
        }
        if words.len() <= MAX_CANDIDATE_WORDS {
            names.extend(trimmed(&words));
            continue;
        }
        let mut run: Vec<&str> = Vec::new();
        let flush = |run: &mut Vec<&str>, names: &mut Vec<String>| {
            for piece in run.chunks(MAX_CANDIDATE_WORDS) {
                names.extend(trimmed(piece));
            }
            run.clear();
        };
        for t in &toks {
            if t.handle {
                flush(&mut run, &mut names);
                let words: Vec<&str> = t.text.split(' ').collect();
                names.extend(trimmed(&words));
                continue;
            }
            if t.capitalised() {
                run.push(&t.text);
                if t.breaks {
                    flush(&mut run, &mut names);
                }
                continue;
            }
            flush(&mut run, &mut names);
            if t.lower_latin() && t.text.chars().count() >= MIN_PLAIN_CHARS && !is_common(&t.text) {
                plain.push(t.text.clone());
            }
        }
        flush(&mut run, &mut names);
    }
    let mut seen = HashSet::new();
    names
        .into_iter()
        .chain(plain)
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

    #[test]
    fn a_bubble_gives_its_names_not_the_sentence() {
        let got = name_candidates(&v(&[
            "Hey Maren, did Tobiah Quell send the deck to the Lantern Works team yet?",
        ]));
        assert_eq!(got, v(&["Maren", "Tobiah Quell", "Lantern Works", "deck"]));
    }

    #[test]
    fn mentions_lose_the_at_sign() {
        let got = name_candidates(&v(&["thanks for the review @orin_vale, merging now"]));
        assert_eq!(got, v(&["orin vale", "review", "merging"]));
    }

    #[test]
    fn short_labels_are_kept_whole_less_common_words() {
        let got = name_candidates(&v(&[
            "Ysolde Brannick",
            "Reply to Kestrel",
            "Type a message",
            "10:42",
            "Online",
        ]));
        assert_eq!(got, v(&["Ysolde Brannick", "Kestrel"]));
    }

    #[test]
    fn long_capitalised_runs_are_split_into_three_word_pieces() {
        let got = name_candidates(&v(&[
            "minutes from Alder Fenwick Corin Malloy Sab Teague today",
        ]));
        assert_eq!(
            got,
            v(&["Alder Fenwick Corin", "Malloy Sab Teague", "minutes"])
        );
    }

    #[test]
    fn hebrew_keeps_short_entries_and_drops_common_ones() {
        let got = name_candidates(&v(&[
            "נויה טלמור",
            "תודה רבה",
            "בסדר",
            "מישהו יכול להביא את המטען מחר בבוקר?",
        ]));
        assert_eq!(got, v(&["נויה טלמור"]));
    }

    #[test]
    fn names_come_before_plain_words_and_the_list_is_capped() {
        let plain: Vec<String> = (0..60u8)
            .map(|i| format!("zq{}{}", (b'a' + i / 26) as char, (b'a' + i % 26) as char))
            .collect();
        let mut screen = vec![plain.join(" ")];
        screen.push("Pellam Oakes".into());
        let got = name_candidates(&screen);
        assert_eq!(got.len(), MAX_CANDIDATES);
        assert_eq!(got[0], "Pellam Oakes");
    }

    #[test]
    fn dedupe_is_case_insensitive() {
        let got = name_candidates(&v(&["Wren Hollis", "wren hollis", "WREN HOLLIS"]));
        assert_eq!(got, v(&["Wren Hollis"]));
    }

    #[test]
    fn possessives_and_punctuation_are_stripped() {
        let got = name_candidates(&v(&[
            "I think Brisa's (the new lead) is out until Thursday.",
        ]));
        assert_eq!(got, v(&["Brisa", "lead"]));
    }

    #[test]
    fn running_twice_changes_nothing() {
        let screen = v(&[
            "Hey Maren, did Tobiah Quell send the deck to the Lantern Works team yet?",
            "@orin_vale can you check the invoice from Dunmore Supply",
            "נויה טלמור",
            "Ysolde Brannick",
        ]);
        let once = name_candidates(&screen);
        assert_eq!(name_candidates(&once), once);
    }
}
