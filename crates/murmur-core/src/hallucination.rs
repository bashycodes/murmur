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
//!   speech, or Whisper says there was none ([`whisper_says_no_speech`]:
//!   no-speech probability above [`NO_SPEECH_THOLD`] and average
//!   log-probability below [`LOGPROB_THOLD`]). A clearly spoken "Thank
//!   you." (≥ ~0.4 s of speech, or decoded with confidence) is kept.

/// Speech (VAD frames) below which a stock phrase is a hallucination.
pub const MAX_SPEECH_MS: u64 = 400;
/// Whisper's default `no_speech_thold` (OpenAI `transcribe.py`
/// `no_speech_threshold`).
pub const NO_SPEECH_THOLD: f32 = 0.6;
/// Whisper's default `logprob_thold` (OpenAI `transcribe.py`
/// `logprob_threshold`).
pub const LOGPROB_THOLD: f32 = -1.0;

/// OpenAI Whisper's no-speech rule (`transcribe.py`, D91): a window is
/// silence only when its no-speech probability is above
/// [`NO_SPEECH_THOLD`] **and** its text's average log-probability is
/// below [`LOGPROB_THOLD`]. A high no-speech probability alone is not
/// enough: a short, clearly decoded word ("you") can read above 0.6 on
/// small multilingual models. Unknown values (`None`) never say silence.
pub fn whisper_says_no_speech(no_speech_prob: Option<f32>, avg_logprob: Option<f32>) -> bool {
    matches!(
        (no_speech_prob, avg_logprob),
        (Some(p), Some(l)) if p > NO_SPEECH_THOLD && l < LOGPROB_THOLD
    )
}

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
/// Whisper's no-speech probability with the average log-probability of
/// the same window (`AsrOutput`; `None` = unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Evidence {
    pub speech_ms: Option<u64>,
    pub no_speech_prob: Option<f32>,
    pub avg_logprob: Option<f32>,
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
    little_speech || whisper_says_no_speech(ev.no_speech_prob, ev.avg_logprob)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_speech_needs_both_signals() {
        // OpenAI transcribe.py: no_speech_prob > 0.6 and avg_logprob < -1.0.
        assert!(whisper_says_no_speech(Some(0.7), Some(-1.3)));
        // Confident text: kept, however high the no-speech probability.
        assert!(!whisper_says_no_speech(Some(0.95), Some(-0.4)));
        // Low no-speech probability: kept, however unsure the text.
        assert!(!whisper_says_no_speech(Some(0.3), Some(-2.0)));
        // Strict bounds.
        assert!(!whisper_says_no_speech(Some(0.6), Some(-1.3)));
        assert!(!whisper_says_no_speech(Some(0.7), Some(-1.0)));
        // Unknown: never silence.
        assert!(!whisper_says_no_speech(Some(0.9), None));
        assert!(!whisper_says_no_speech(None, Some(-2.0)));
        assert!(!whisper_says_no_speech(None, None));
    }
}
