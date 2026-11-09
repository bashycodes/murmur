//! Whisper silence-hallucination guard.
//!
//! On near-silent input (a breath, a click, the tail of a key press)
//! Whisper tends to emit stock phrases from its subtitle training data:
//! "Thank you.", "you", "Thanks for watching!", subtitle credits. Pasting
//! those is worse than pasting nothing.
//!
//! Two tiers:
//! * **Credits** ("Subtitles by the Amara.org community", …) are never
//!   dictation and are always suppressed.
//! * **Stock phrases** are suppressed only with evidence that there was
//!   little or no speech: the VAD found less than [`MAX_SPEECH_MS`] of
//!   speech, or Whisper's own no-speech probability is at least
//!   [`NO_SPEECH_THOLD`]. A clearly spoken "Thank you." (≥ ~0.4 s of
//!   speech, low no-speech probability) is kept.

/// Speech (VAD frames) below which a stock phrase is a hallucination.
pub const MAX_SPEECH_MS: u64 = 400;
/// Whisper's default `no_speech_thold`.
pub const NO_SPEECH_THOLD: f32 = 0.6;

/// Stock phrases, normalised (lowercase, letters/digits/spaces only).
const STOCK: &[&str] = &[
    "you",
    "thank you",
    "thank you very much",
    "thank you so much",
    "thanks",
    "thanks for watching",
    "thank you for watching",
    "thanks for listening",
    "thank you for listening",
    "please subscribe",
    "like and subscribe",
    "bye",
    "bye bye",
    "goodbye",
    "okay",
    "oh",
    "so",
    "hmm",
    "uh",
    "um",
    "i",
    "the end",
];

/// Credit lines: suppressed whatever the audio looked like.
const CREDITS: &[&str] = &[
    "subtitles by",
    "subtitled by",
    "transcribed by",
    "translated by",
    "captions by",
    "amara org",
    "transcription by",
];

/// Lowercase, keep letters/digits, collapse everything else to one space.
pub fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            out.push(c);
            space = false;
        } else if c != '\'' && c != '\u{2019}' {
            space = true;
        }
    }
    out
}

/// What the ASR saw: speech duration from the VAD (`None` = VAD off) and
/// Whisper's highest segment no-speech probability (`None` = unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Evidence {
    pub speech_ms: Option<u64>,
    pub no_speech_prob: Option<f32>,
}

/// True when `text` should be discarded as a silence hallucination.
pub fn is_hallucination(text: &str, ev: Evidence) -> bool {
    let n = normalise(text);
    if n.is_empty() {
        // Only punctuation ("...", "-") or annotations.
        return !text.trim().is_empty();
    }
    if CREDITS.iter().any(|c| n.contains(c)) {
        return true;
    }
    // "Thank you. Thank you." counts as the phrase repeated.
    let stock = STOCK.iter().any(|p| {
        n == *p || {
            let words: Vec<&str> = n.split(' ').collect();
            let k = p.split(' ').count();
            words.len().is_multiple_of(k)
                && words.len() > k
                && words.chunks(k).all(|c| c.join(" ") == *p)
        }
    });
    if !stock {
        return false;
    }
    let little_speech = ev.speech_ms.is_some_and(|ms| ms < MAX_SPEECH_MS);
    let whisper_says_none = ev.no_speech_prob.is_some_and(|p| p >= NO_SPEECH_THOLD);
    little_speech || whisper_says_none
}

/// Stock phrases whisper appends to a segment that ends in silence.
const TRAILING: &[&str] = &[
    "thank you",
    "thanks",
    "thank you very much",
    "thank you so much",
    "thanks for watching",
    "thank you for watching",
    "thanks for listening",
    "thank you for listening",
];

/// `text` without a trailing stock sentence ("…, and that's it. Thank
/// you."), when there is other text before it; `None` when there is
/// nothing to strip. For pieces of a segmented dictation that are not the
/// last one (D22), where a "Thank you." with a high no-speech probability
/// is whisper filling the pause, not the user.
pub fn strip_trailing_stock(text: &str) -> Option<&str> {
    let t = text.trim_end();
    let body = t.trim_end_matches(['.', '!', '?', ' ']);
    let cut = body.rfind(['.', '!', '?'])?;
    let (head, tail) = (&t[..=cut], &t[cut + 1..]);
    TRAILING
        .contains(&normalise(tail).as_str())
        .then(|| head.trim_end())
        .filter(|h| !normalise(h).is_empty())
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    #[test]
    fn strips_only_a_trailing_stock_sentence() {
        assert_eq!(
            strip_trailing_stock("We shipped it. Thank you."),
            Some("We shipped it.")
        );
        assert_eq!(
            strip_trailing_stock("Is it done? Thanks for watching!"),
            Some("Is it done?")
        );
        assert_eq!(strip_trailing_stock("Thank you."), None);
        assert_eq!(strip_trailing_stock("We said thank you."), None);
        assert_eq!(
            strip_trailing_stock("Done. Thank you all for coming."),
            None
        );
    }
}
