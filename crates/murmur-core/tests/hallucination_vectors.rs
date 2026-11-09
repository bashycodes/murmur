//! Vector-style table for the Whisper silence-hallucination guard.
//! Each row: transcript, VAD speech ms, Whisper no-speech prob, suppressed?

use murmur_core::hallucination::{is_hallucination, Evidence};

const NONE: Option<f32> = None;

#[rustfmt::skip]
const VECTORS: &[(&str, Option<u64>, Option<f32>, bool)] = &[
    // Stock phrases on near-silence: suppressed.
    ("Thank you.",                     Some(120), NONE,       true),
    (" you",                           Some(60),  NONE,       true),
    ("Thanks for watching!",           Some(200), NONE,       true),
    ("Thank you. Thank you.",          Some(300), NONE,       true),
    ("Bye.",                           Some(0),   NONE,       true),
    // Whisper's own no-speech probability is enough evidence.
    ("Thank you.",                     None,      Some(0.85), true),
    ("Thank you.",                     Some(900), Some(0.7),  true),
    // Clearly spoken stock phrase: kept.
    ("Thank you.",                     Some(650), Some(0.05), false),
    ("Thank you.",                     None,      NONE,       false),
    ("you",                            Some(500), Some(0.1),  false),
    // Credits: always suppressed.
    ("Subtitles by the Amara.org community", Some(5000), Some(0.0), true),
    ("Transcribed by ESO, translated by —",  None,       NONE,      true),
    // Only punctuation: suppressed; empty: not a hallucination (already empty).
    ("...",                            Some(900), NONE,       true),
    ("",                               Some(0),   NONE,       false),
    // Real dictation that merely contains a stock phrase: kept.
    ("Thank you for the quick reply.", Some(100), Some(0.9),  false),
    ("You should see this.",           Some(100), NONE,       false),
    ("Okay, ship it.",                 Some(150), NONE,       false),
    ("thank-you note",                 Some(100), NONE,       false),
];

#[test]
fn hallucination_vectors() {
    let mut fails = Vec::new();
    for &(text, speech_ms, no_speech_prob, want) in VECTORS {
        let got = is_hallucination(
            text,
            Evidence {
                speech_ms,
                no_speech_prob,
            },
        );
        if got != want {
            fails.push(format!(
                "{text:?} {speech_ms:?} {no_speech_prob:?}: got {got}"
            ));
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}
