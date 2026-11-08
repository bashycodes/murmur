//! D71: a cleaned dictation never repeats the field's own text.
//!
//! The cleanup prompt shows the model the text around the cursor (`TEXT
//! BEFORE CURSOR`, `TEXT AFTER CURSOR`) so it can fit the new words in.
//! A small model sometimes copies it into its answer: with "Gate code: "
//! already in the field and only "four seven one nine" dictated, the 3B
//! answered "Gate code: 4719", and the field then read "Gate code: Gate
//! code: 4719" (`docs/findings/2026-11-07-context-echo.md`). Prompt framing
//! was measured and dropped (it made the model lose dictated words), so this
//! check after the model is the fix.
//!
//! [`strip`] removes words at the start of the answer that repeat the end of
//! the text before the cursor, and words at the end that repeat the start of
//! the text after it: the same window of the field the prompt showed. Words
//! compare case-insensitively without their edge punctuation. A run must be
//! at least [`MIN_WORDS`] words, or the whole window when that is shorter
//! ("Password: ").
//!
//! The check is conservative: a wrong strip loses dictated words, a missed
//! one only leaves the echo. So a run is removed only when fewer than half
//! of its words could come from the text the model was given (the
//! transcript, or the rules' text on the on-device path). A word could come
//! from it when that text has the word, a near spelling of it (the model
//! fixing "acne" to "Acme"), consecutive words that join into it ("p m" ->
//! "pm"), a longer word it abbreviates ("Dr." for "doctor"), or when it holds
//! a digit (the model writes spoken numbers as digits). Fillers between the
//! dictated words do not matter: words are matched one by one, not as a run.

/// The shortest echo removed, unless the context window is that short.
pub const MIN_WORDS: usize = 2;

/// A word of `s` (split at whitespace and zero-width spaces): its byte range,
/// its compare key, and whether it is written with a final dot ("Dr.").
struct Word {
    start: usize,
    end: usize,
    key: String,
    dotted: bool,
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{200B}' | '\u{FEFF}')
}

fn key(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// The words of `s` that have a letter or digit (a lone "—" or ":" is
/// not a word).
fn words(s: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices().chain(std::iter::once((s.len(), ' '))) {
        match (is_space(c), start) {
            (true, Some(b)) => {
                let raw = &s[b..i];
                let k = key(raw);
                if !k.is_empty() {
                    let dotted = raw
                        .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '.')
                        .ends_with('.');
                    out.push(Word {
                        start: b,
                        end: i,
                        key: k,
                        dotted,
                    });
                }
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    out
}

/// Edit similarity of two words, 1.0 = equal.
fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    1.0 - row[b.len()] as f32 / a.len().max(b.len()).max(1) as f32
}

/// `short` keeps `long`'s first letter and the rest of its letters in
/// order, and is at least two letters shorter ("dr" for "doctor", but not
/// "for" for "four").
fn abbreviates(short: &str, long: &str) -> bool {
    let mut l = long.chars();
    short.chars().count() + 2 <= long.chars().count()
        && short.chars().next() == long.chars().next()
        && short.chars().all(|c| l.any(|d| d == c))
}

/// Whether answer word `w` could come from the words the model was given.
/// Only a word written with a final dot counts as an abbreviation, so "the"
/// is not taken for "three".
fn could_be_said(word: &Word, said: &[&str]) -> bool {
    let w = word.key.as_str();
    if w.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let near = |s: &&str| {
        *s == w
            || (w.chars().count() >= 4 && similarity(w, s) >= 0.7)
            || (word.dotted && w.chars().count() <= 4 && abbreviates(w, s))
    };
    let joined = |n: usize| said.windows(n).any(|ws| ws.concat() == w);
    said.iter().any(near) || joined(2) || joined(3)
}

/// The run is an echo: long enough, and mostly words the model was not given.
fn is_echo(run: &[Word], window_words: usize, said: &[&str]) -> bool {
    let n = run.len();
    let could = run.iter().filter(|w| could_be_said(w, said)).count();
    (n >= MIN_WORDS || n == window_words) && could * 2 < n
}

fn same(a: &[Word], b: &[Word]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.key == y.key)
}

/// Separators left between a removed echo and the dictated words: ASCII and
/// full-width colons, semicolons, commas, dashes, the Hebrew maqaf, quotes.
const SEPARATORS: &[char] = &[
    ':', ';', ',', '-', '\u{2013}', '\u{2014}', '\u{05BE}', '\u{FF1A}', '\u{FF1B}', '\u{FF0C}',
    '"', '\'', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{00AB}', '\u{00BB}',
];

fn is_cut(c: char) -> bool {
    is_space(c) || SEPARATORS.contains(&c)
}

/// Which end of the answer an echo sits at.
#[derive(Clone, Copy)]
enum Side {
    /// The answer starts with the end of the text before the cursor.
    Before,
    /// The answer ends with the start of the text after the cursor.
    After,
}

/// The answer without the echo of `window` at `side`, if there is one.
fn strip_side<'a>(answer: &'a str, window: &str, said: &[&str], side: Side) -> Option<&'a str> {
    let cw = words(window);
    let aw = words(answer);
    // Keep at least one answer word: an answer that is only context stays.
    let max = cw.len().min(aw.len().saturating_sub(1));
    let n = (1..=max).rev().find(|&n| {
        let (ctx, ans) = match side {
            Side::Before => (&cw[cw.len() - n..], &aw[..n]),
            Side::After => (&cw[..n], &aw[aw.len() - n..]),
        };
        same(ctx, ans) && is_echo(ans, cw.len(), said)
    })?;
    let out = match side {
        Side::Before => answer[aw[n - 1].end..].trim_start_matches(is_cut),
        Side::After => answer[..aw[aw.len() - n].start].trim_end_matches(is_cut),
    };
    (!out.is_empty()).then_some(out)
}

/// `answer` without the context it repeats (see the module docs), or `None`
/// when it repeats none. `given` is the text the model was given; `before` /
/// `after` are the windows of the field the prompt showed. Never returns an
/// empty answer.
pub fn strip(answer: &str, given: &str, before: &str, after: &str) -> Option<String> {
    let said = words(given);
    let said: Vec<&str> = said.iter().map(|w| w.key.as_str()).collect();
    let lead = strip_side(answer, before, &said, Side::Before);
    let rest = lead.unwrap_or(answer);
    let tail = strip_side(rest, after, &said, Side::After);
    match (lead, tail) {
        (None, None) => None,
        _ => Some(tail.unwrap_or(rest).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_before_the_cursor_is_removed_from_the_answer() {
        // The shape of the 2026-11-07 report, with made-up text.
        let before = "This is my tablet's port: ";
        let t = "192.0.2.9 colon five five five one";
        let a = "This is my tablet's port: 192.0.2.9:5551";
        assert_eq!(strip(a, t, before, "").as_deref(), Some("192.0.2.9:5551"));
    }

    #[test]
    fn only_the_end_of_the_context_counts_and_case_is_ignored() {
        let before = "Notes from the call.\nNext step: ";
        assert_eq!(
            strip(
                "next step: Send the draft by Friday.",
                "send the draft by friday",
                before,
                ""
            )
            .as_deref(),
            Some("Send the draft by Friday.")
        );
        assert_eq!(
            strip(
                "Notes from the call. Next step: send the draft.",
                "send the draft",
                before,
                ""
            )
            .as_deref(),
            Some("send the draft.")
        );
    }

    #[test]
    fn words_the_speaker_said_stay() {
        let before = "Gate code: ";
        assert_eq!(
            strip(
                "Gate code: 4719",
                "gate code colon four seven one nine",
                before,
                ""
            ),
            None
        );
        assert_eq!(
            strip(
                "The gate code is 4719.",
                "the gate code is 4719",
                before,
                ""
            ),
            None
        );
    }

    #[test]
    fn spoken_numbers_and_abbreviations_the_model_formatted_stay() {
        // Review case 1: "3 pm" is not in the transcript as written.
        assert_eq!(
            strip("3 PM works.", "three p m works", "Meeting at 3 pm. ", ""),
            None
        );
        assert_eq!(
            strip("3 pm works.", "three p m works", "Meeting at 3 pm ", ""),
            None
        );
        assert_eq!(
            strip("Dr. Lane is in.", "doctor lane is in", "Ask Dr. Lane ", ""),
            None
        );
    }

    #[test]
    fn a_short_word_is_an_abbreviation_only_when_written_with_a_dot() {
        // "the" is not "three" in the spoken port.
        let given = "demo dot example dot org port four four three";
        assert_eq!(
            strip(
                "Server for the demo: demo.example.org port 443.",
                given,
                "Server for the demo: ",
                ""
            )
            .as_deref(),
            Some("demo.example.org port 443.")
        );
    }

    #[test]
    fn fillers_between_the_dictated_words_do_not_make_an_echo() {
        // Review case 2: the raw transcript has "um" inside the phrase.
        assert_eq!(
            strip(
                "Next step: send it.",
                "next, um, step: send it",
                "Plan. Next step: ",
                ""
            ),
            None
        );
    }

    #[test]
    fn a_name_the_model_corrected_stays() {
        // Review case 3: speech-to-text heard "acne", the field says Acme.
        assert_eq!(
            strip(
                "Acme Corp: invoice sent.",
                "acne corp invoice sent",
                "Acme Corp: ",
                ""
            ),
            None
        );
    }

    #[test]
    fn one_word_is_an_echo_only_when_it_is_the_whole_context() {
        assert_eq!(
            strip(
                "Password: blue heron 42",
                "blue heron forty two",
                "Password: ",
                ""
            )
            .as_deref(),
            Some("blue heron 42")
        );
        // "the" ends a longer context: a one-word match is not enough.
        assert_eq!(
            strip(
                "The station around 6.",
                "the station around six",
                "See you at the ",
                ""
            ),
            None
        );
    }

    #[test]
    fn text_after_the_cursor_is_removed_from_the_end() {
        let after = " is the new door code.";
        assert_eq!(
            strip("5521 is the new door code.", "five five two one", "", after).as_deref(),
            Some("5521")
        );
    }

    #[test]
    fn a_quote_at_the_cut_goes_with_the_echo() {
        let after = "\" was all she wrote.";
        assert_eq!(
            strip("on my way\" was all she wrote.", "on my way", "", after).as_deref(),
            Some("on my way")
        );
        let before = "Tag:\u{200B}\u{201C}";
        assert_eq!(
            strip("Tag: \u{201C}blue sky", "blue sky", before, "").as_deref(),
            Some("blue sky")
        );
    }

    #[test]
    fn hebrew_and_full_width_separators_are_cut() {
        assert_eq!(
            strip("קוד שער: \u{05BE}4719", "ארבע שבע אחת תשע", "קוד שער: ", "").as_deref(),
            Some("4719")
        );
        assert_eq!(
            strip(
                "Gate code\u{FF1A} 4719",
                "four seven one nine",
                "Gate code\u{FF1A} ",
                ""
            )
            .as_deref(),
            Some("4719")
        );
    }

    #[test]
    fn an_answer_that_is_only_context_is_left_alone() {
        assert_eq!(strip("Gate code:", "um", "Gate code: ", ""), None);
        assert_eq!(strip("Fine, thanks.", "fine thanks", "", ""), None);
    }
}
