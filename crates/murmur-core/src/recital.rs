//! Whisper reciting its `initial_prompt` (D87).
//!
//! On near-silence, or after the speech ends, Whisper can write out the
//! dictionary and screen words it was primed with instead of what was
//! said. [`check`] compares a decode with the prompt's word list: a result
//! that is almost all list words, and uses most of the list, is a recital
//! (decode again without the prompt); a recital after real speech is cut
//! off. Same thresholds as OpenWhispr's echo filter
//! (`docs/findings/2026-11-07-openwhispr-analysis.md` §B.4).
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
    let list: HashSet<String> = prompt_phrases(prompt)
        .iter()
        .flat_map(|p| p.split_whitespace().map(norm))
        .filter(|w| !w.is_empty())
        .collect();
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
    if cut == 0 || cut == tokens.len() || !recites(&words[cut..]) {
        return Recital::None;
    }
    // Only a list that starts a new sentence or is written as a comma
    // list: "… ask Quorvel, Tamsik and Brennolt" is speech.
    let after_sentence = tokens[cut - 1].ends_with(['.', '!', '?']);
    let comma_list = tokens[cut..tokens.len() - 1]
        .iter()
        .all(|t| t.ends_with(',') || norm(t).is_empty());
    if !(after_sentence || comma_list) {
        return Recital::None;
    }
    let speech = tokens[..cut].join(" ");
    Recital::Tail(speech.trim_end_matches([',', ';', ':']).to_string())
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

    #[test]
    fn a_list_after_real_speech_is_cut_off() {
        assert_eq!(
            check(
                "Let's move the review to Friday. Quorvel, Tamsik, Brennolt Ardy, Vexmoor.",
                PROMPT
            ),
            Recital::Tail("Let's move the review to Friday.".into())
        );
        // A comma list straight after the speech.
        assert_eq!(
            check(
                "see you there, Quorvel, Tamsik, Brennolt, Ardy, Vexmoor, Lintrava",
                PROMPT
            ),
            Recital::Tail("see you there".into())
        );
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
