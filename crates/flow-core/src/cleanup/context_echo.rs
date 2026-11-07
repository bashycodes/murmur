//! D71: a cleaned dictation never repeats the field's own text.
//!
//! The cleanup prompt shows the model the text around the cursor (`TEXT
//! BEFORE CURSOR`, `TEXT AFTER CURSOR`) so it can fit the new words in.
//! A small model sometimes copies it into its answer: with "Gate code: "
//! already in the field and only "four seven one nine" dictated, the 3B
//! answered "Gate code: 4719", and the field then read "Gate code: Gate
//! code: 4719" (`docs/findings/2026-11-07-context-echo.md`). The prompt
//! now says the context is already typed; this check is the backstop.
//!
//! [`strip`] removes words at the start of the answer that repeat the end
//! of the text before the cursor, and words at the end that repeat the
//! start of the text after it, when the transcript itself never said them.
//! Words compare case-insensitively without their edge punctuation. A run
//! must be at least [`MIN_WORDS`] words, or the whole context when that is
//! shorter ("Password: "). A dictation that says the words itself ("gate
//! code is 4719" after "Gate code: ") keeps them.

/// The shortest echo removed, unless the context is that short as a whole.
pub const MIN_WORDS: usize = 2;

/// A whitespace-separated word of `s`: its byte range and its compare key.
struct Word {
    start: usize,
    end: usize,
    key: String,
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
        match (c.is_whitespace(), start) {
            (true, Some(b)) => {
                let k = key(&s[b..i]);
                if !k.is_empty() {
                    out.push(Word {
                        start: b,
                        end: i,
                        key: k,
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

fn keys(w: &[Word]) -> Vec<&str> {
    w.iter().map(|w| w.key.as_str()).collect()
}

/// Whether `needle` occurs as a run of consecutive words in `hay`.
fn contains_run(hay: &[&str], needle: &[&str]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn long_enough(n: usize, context_words: usize) -> bool {
    n >= MIN_WORDS || n == context_words
}

/// Separators left between a removed echo and the dictated words.
const SEPARATORS: &[char] = &[':', ';', ',', '-', '\u{2013}', '\u{2014}'];

/// `answer` without the context it repeats (see the module docs), or
/// `None` when it repeats none. Never returns an empty answer: an answer
/// that is nothing but context is left to the caller as is.
pub fn strip(answer: &str, transcript: &str, before: &str, after: &str) -> Option<String> {
    let said = words(transcript);
    let said = keys(&said);
    let mut out = answer;
    let mut changed = false;

    let bw = words(before);
    let ow = words(out);
    let max = bw.len().min(ow.len().saturating_sub(1));
    if let Some(n) = (1..=max).rev().find(|&n| {
        let tail = keys(&bw[bw.len() - n..]);
        long_enough(n, bw.len()) && keys(&ow[..n]) == tail && !contains_run(&said, &tail)
    }) {
        out = out[ow[n - 1].end..]
            .trim_start_matches(|c: char| c.is_whitespace() || SEPARATORS.contains(&c));
        changed = true;
    }

    let aw = words(after);
    let ow = words(out);
    let max = aw.len().min(ow.len().saturating_sub(1));
    if let Some(n) = (1..=max).rev().find(|&n| {
        let head = keys(&aw[..n]);
        long_enough(n, aw.len()) && keys(&ow[ow.len() - n..]) == head && !contains_run(&said, &head)
    }) {
        out = out[..ow[ow.len() - n].start]
            .trim_end_matches(|c: char| c.is_whitespace() || SEPARATORS.contains(&c));
        changed = true;
    }

    (changed && !out.is_empty()).then(|| out.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_before_the_cursor_is_removed_from_the_answer() {
        // The shape of the 2026-11-07 report, with made-up text.
        let before = "This is my tablet's port: ";
        let t = "100.64.12.9 colon five five five five one";
        let a = "This is my tablet's port: 100.64.12.9:55551";
        assert_eq!(
            strip(a, t, before, "").as_deref(),
            Some("100.64.12.9:55551")
        );
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
    fn an_answer_that_is_only_context_is_left_alone() {
        assert_eq!(strip("Gate code:", "um", "Gate code: ", ""), None);
        assert_eq!(strip("Fine, thanks.", "fine thanks", "", ""), None);
    }
}
