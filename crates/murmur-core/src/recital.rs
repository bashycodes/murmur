//! Whisper reciting its `initial_prompt` (D87).
//!
//! On near-silence, or after the speech ends, Whisper can write out the
//! dictionary and screen words it was primed with instead of what was
//! said. [`check`] compares a decode with the prompt's word list: a result
//! that is almost all list words, and uses most of the list, is a recital
//! (decode again without the prompt). Same thresholds as OpenWhispr's echo
//! filter (`docs/findings/2026-11-07-openwhispr-analysis.md` §B.4). A
//! recital after real speech is cut off only under a much stricter rule
//! (the prompt's order, nearly all of the list; [`TAIL_LIST_SHARE`]).
//!
//! Words are split on whitespace, so scripts written without spaces
//! (Chinese, Japanese, Thai) are not checked: a clause is one "word" and
//! stays under [`MIN_WORDS`].

use std::collections::HashSet;

use crate::asr::prompt_phrases;

/// Share of the output's words that are list words for a recital.
pub const OUTPUT_SHARE: f32 = 0.9;
/// Share of the list's distinct words the output must use.
pub const LIST_SHARE: f32 = 0.7;
/// Fewer list words, or a shorter recital, is never treated as one: with
/// one or two dictionary words, saying them is ordinary speech.
pub const MIN_WORDS: usize = 3;
/// A recited tail after speech must use this share of the list ...
pub const TAIL_LIST_SHARE: f32 = 0.9;
/// ... and be at least this long, so a short dictionary read out in full
/// is never cut.
pub const TAIL_MIN_WORDS: usize = 6;

/// What [`check`] found in a decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recital {
    /// Not a recital.
    None,
    /// The whole output is the recited list: decode again without the
    /// prompt.
    Whole,
    /// Real speech followed by a recited list: this text is the speech.
    Tail(String),
}

fn norm(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Whether `text` recites the word list of `prompt` (a
/// [`crate::asr::build_initial_prompt`] string).
pub fn check(text: &str, prompt: &str) -> Recital {
    let sequence: Vec<String> = prompt_phrases(prompt)
        .iter()
        .flat_map(|p| p.split_whitespace().map(norm))
        .filter(|w| !w.is_empty())
        .collect();
    let list: HashSet<String> = sequence.iter().cloned().collect();
    if list.len() < MIN_WORDS {
        return Recital::None;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| norm(t)).collect();
    let recites = |words: &[String]| {
        let n = words.iter().filter(|w| !w.is_empty()).count();
        let listed: Vec<&str> = words
            .iter()
            .filter(|w| list.contains(*w))
            .map(String::as_str)
            .collect();
        let used: HashSet<&str> = listed.iter().copied().collect();
        n >= MIN_WORDS
            && listed.len() as f32 >= OUTPUT_SHARE * n as f32
            && used.len() as f32 >= LIST_SHARE * list.len() as f32
    };
    if recites(&words) {
        return Recital::Whole;
    }
    // The longest run of list words (and bare punctuation) at the end.
    let cut = words
        .iter()
        .rposition(|w| !w.is_empty() && !list.contains(w))
        .map_or(0, |i| i + 1);
    if cut == 0 || cut == tokens.len() || !recites_in_order(&words[cut..], &sequence, &list) {
        return Recital::None;
    }
    let speech = tokens[..cut].join(" ");
    Recital::Tail(speech.trim_end_matches([',', ';', ':']).to_string())
}

/// Whether a tail of list words is the prompt read back: at least
/// [`TAIL_MIN_WORDS`] words, in the prompt's own order, using at least
/// [`TAIL_LIST_SHARE`] of the list. Much stricter than [`check`]'s
/// whole-output rule, because a cut deletes text with no second decode to
/// fall back on: a user dictating some of their own names ("Please cc
/// Quorvel, Tamsik, Brennolt.") must keep them, and a missed recital costs
/// far less than lost speech.
fn recites_in_order(tail: &[String], sequence: &[String], list: &HashSet<String>) -> bool {
    let tail: Vec<&String> = tail.iter().filter(|w| !w.is_empty()).collect();
    let used: HashSet<&String> = tail.iter().copied().collect();
    if tail.len() < TAIL_MIN_WORDS || (used.len() as f32) < TAIL_LIST_SHARE * list.len() as f32 {
        return false;
    }
    // A subsequence of the prompt's words.
    let mut at = 0;
    tail.iter()
        .all(|w| match sequence[at..].iter().position(|s| s == *w) {
            Some(i) => {
                at += i + 1;
                true
            }
            None => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic words only.
    const PROMPT: &str = "Quorvel, Tamsik, Brennolt Ardy, Vexmoor, Lintrava.";

    #[test]
    fn a_decode_of_only_list_words_is_a_recital() {
        assert_eq!(
            check("Quorvel, Tamsik, Brennolt Ardy, Vexmoor.", PROMPT),
            Recital::Whole
        );
        // Whisper's own punctuation and case do not matter.
        assert_eq!(
            check("quorvel tamsik brennolt ardy vexmoor lintrava", PROMPT),
            Recital::Whole
        );
    }

    #[test]
    fn speech_that_uses_some_list_words_is_not() {
        assert_eq!(
            check("Send the Quorvel draft to Tamsik today.", PROMPT),
            Recital::None
        );
        // Mostly list words but only 2 of 6 of the list.
        assert_eq!(check("Quorvel Tamsik Quorvel", PROMPT), Recital::None);
        // 4 of 6 list words, 1 other word in 5: 80 % < 90 %.
        assert_eq!(
            check("Quorvel Tamsik Brennolt and Ardy", PROMPT),
            Recital::None
        );
        assert_eq!(check("", PROMPT), Recital::None);
    }

    #[test]
    fn a_short_list_is_never_recited() {
        assert_eq!(check("Quorvel, Tamsik.", "Quorvel, Tamsik."), Recital::None);
        assert_eq!(check("Quorvel, Tamsik.", ""), Recital::None);
    }

    /// Screen words, then the dictionary, as `build_initial_prompt` writes.
    const LONG_PROMPT: &str = "Pellish, Orrin Vasko, Quorvel, Tamsik, Brennolt Ardy, Vexmoor.";

    #[test]
    fn the_whole_prompt_read_back_after_speech_is_cut_off() {
        assert_eq!(
            check(
                "Let's move the review to Friday. Pellish, Orrin Vasko, Quorvel, \
                 Tamsik, Brennolt Ardy, Vexmoor.",
                LONG_PROMPT
            ),
            Recital::Tail("Let's move the review to Friday.".into())
        );
        assert_eq!(
            check(
                "see you there, pellish orrin vasko quorvel tamsik brennolt ardy vexmoor",
                LONG_PROMPT
            ),
            Recital::Tail("see you there".into())
        );
    }

    #[test]
    fn a_tail_short_of_the_full_prompt_in_order_is_kept() {
        // 7 of 8 list words.
        let said = "We met. Pellish, Orrin Vasko, Quorvel, Tamsik, Brennolt Ardy.";
        assert_eq!(check(said, LONG_PROMPT), Recital::None);
        // All 8, out of the prompt's order.
        let said = "We met. Vexmoor, Pellish, Orrin Vasko, Quorvel, Tamsik, Brennolt Ardy.";
        assert_eq!(check(said, LONG_PROMPT), Recital::None);
    }

    #[test]
    fn dictating_your_own_names_is_never_cut() {
        // Regression (review of 97726f9): both were cut to the first words.
        let dict = "Quorvel, Tamsik, Brennolt, Vexmoor, Lintrava.";
        let said = "Please cc Quorvel, Tamsik, Brennolt, Vexmoor.";
        assert_eq!(check(said, dict), Recital::None);
        let said = "Thanks. Quorvel, Tamsik, Brennolt.";
        assert_eq!(check(said, dict), Recital::None);
        // Even the whole short dictionary in order: under TAIL_MIN_WORDS.
        let said = "Please cc Quorvel, Tamsik, Brennolt, Vexmoor, Lintrava.";
        assert_eq!(check(said, dict), Recital::None);
    }

    #[test]
    fn list_words_ending_a_sentence_are_kept() {
        assert_eq!(
            check("Please ask Quorvel Tamsik Brennolt and Vexmoor", PROMPT),
            Recital::None
        );
        assert_eq!(
            check("Please ask Quorvel Tamsik Brennolt Ardy Vexmoor", PROMPT),
            Recital::None
        );
        // Too few list words in the tail.
        assert_eq!(check("We met. Quorvel, Tamsik.", PROMPT), Recital::None);
    }
}
