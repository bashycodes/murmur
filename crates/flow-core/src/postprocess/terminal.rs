//! Closing punctuation for a dictation the ASR left open.
//!
//! Not part of the client post-processing, which never adds
//! punctuation. Parakeet-unified often
//! ends a clip without a period, even on whole clips ("…any other school"),
//! and whisper sometimes ends on a comma. The engine runs this on the
//! dictation before the LLM and the client post-processing, so the cursor-context step still drops the period
//! where Wispr would (messaging apps, a continuation after the cursor).

/// Dictations shorter than this stay as they are: names, search terms and
/// form values ("John Smith", "pizza near me").
pub const MIN_WORDS: usize = 4;

/// `text` with a closing period: appended when it ends in a letter or a
/// digit, or replacing a trailing `,` / `;` (whisper's "Thank you, sir,"),
/// when it has at least [`MIN_WORDS`] words. Multi-line text (lists,
/// email layout) and text with paste markers is left alone.
pub fn close_sentence(text: &str) -> String {
    let t = text.trim_end();
    if t.contains('\n') || t.contains("{{") || t.split_whitespace().count() < MIN_WORDS {
        return text.to_string();
    }
    let Some(last) = t.chars().next_back() else {
        return text.to_string();
    };
    if last.is_alphanumeric() {
        format!("{t}.")
    } else if last == ',' || last == ';' {
        format!("{}.", &t[..t.len() - 1])
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_an_open_dictation() {
        // cliptest 2026-10-23, Parakeet-unified whole clips.
        assert_eq!(
            close_sentence("it is not characteristic of Gothic or any other school"),
            "it is not characteristic of Gothic or any other school."
        );
        assert_eq!(
            close_sentence("I'm going to see mister Marshall. Thank you, sir,"),
            "I'm going to see mister Marshall. Thank you, sir."
        );
        assert_eq!(
            close_sentence("we sold 12 units in 2025"),
            "we sold 12 units in 2025."
        );
        assert_eq!(close_sentence("one two three four "), "one two three four.");
    }

    #[test]
    fn leaves_closed_short_or_structured_text() {
        for t in [
            "Is that right?",
            "Stop it!",
            "Already done.",
            "and then...",
            "he said \"hello there friend\"",
            "see (the notes below)",
            "pizza near me",
            "John Smith",
            "",
            "1. Milk\n2. Eggs and bread",
            "Thanks for the update{{BREAK}}Best regards",
            "send it to the team:",
        ] {
            assert_eq!(close_sentence(t), t, "{t:?}");
        }
    }
}
