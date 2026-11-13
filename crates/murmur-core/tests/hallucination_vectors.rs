//! Vector-style table for the Whisper silence-hallucination guard.
//! Each row: transcript, VAD speech ms, Whisper no-speech prob, Whisper
//! average log-probability of the same window, suppressed?

use murmur_core::hallucination::{is_hallucination, Evidence};

const NONE: Option<f32> = None;

/// Transcript, VAD speech ms, no-speech prob, avg log-prob, suppressed?
type Vector = (&'static str, Option<u64>, Option<f32>, Option<f32>, bool);

#[rustfmt::skip]
const VECTORS: &[Vector] = &[
    // Stock phrases on near-silence: suppressed.
    ("Thank you.",                     Some(120), NONE,      NONE,       true),
    (" you",                           Some(60),  NONE,      NONE,       true),
    ("Thanks for watching!",           Some(200), NONE,      NONE,       true),
    ("Thank you. Thank you.",          Some(300), NONE,      NONE,       true),
    ("Bye.",                           Some(0),   NONE,      NONE,       true),
    // Whisper says no speech (OpenAI's rule, D91): no-speech probability
    // above 0.6 and average log-probability below -1.0.
    ("Thank you.",                     None,      Some(0.85), Some(-1.3), true),
    ("Thank you.",                     Some(900), Some(0.7), Some(-1.3), true),
    // A high no-speech probability alone is not enough (D91): a short
    // "you" decoded with confidence, or no log-probability known.
    ("you",                            None,      Some(0.7),  Some(-0.4), false),
    ("Thank you.",                     None,      Some(0.85), NONE,       false),
    // At the thresholds exactly: not silence (both bounds are strict).
    ("Thank you.",                     None,      Some(0.6),  Some(-1.3), false),
    ("Thank you.",                     None,      Some(0.9),  Some(-1.0), false),
    // Clearly spoken stock phrase: kept.
    ("Thank you.",                     Some(650), Some(0.05), NONE,       false),
    ("Thank you.",                     None,      NONE,      NONE,       false),
    ("you",                            Some(500), Some(0.1), NONE,       false),
    // Credits: always suppressed.
    ("Subtitles by the Amara.org community", Some(5000), Some(0.0), NONE,       true),
    ("Transcribed by ESO, translated by —",  None,       NONE,      NONE,       true),
    // Only punctuation: suppressed; empty: not a hallucination (already empty).
    ("...",                            Some(900), NONE,      NONE,       true),
    ("",                               Some(0),   NONE,      NONE,       false),
    // Real dictation that merely contains a stock phrase: kept.
    ("Thank you for the quick reply.", Some(100), Some(0.9), NONE,       false),
    ("You should see this.",           Some(100), NONE,      NONE,       false),
    ("Okay, ship it.",                 Some(150), NONE,      NONE,       false),
    ("thank-you note",                 Some(100), NONE,      NONE,       false),
];

#[test]
fn hallucination_vectors() {
    let mut fails = Vec::new();
    for &(text, speech_ms, no_speech_prob, avg_logprob, want) in VECTORS {
        let got = is_hallucination(
            text,
            Evidence {
                speech_ms,
                no_speech_prob,
                avg_logprob,
            },
        );
        if got != want {
            fails.push(format!(
                "{text:?} {speech_ms:?} {no_speech_prob:?} {avg_logprob:?}: got {got}"
            ));
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}
