//! D63: a tidy English dictation that still holds a number spoken as words
//! is cleaned by the on-device model instead of skipped as "already clean";
//! number idioms and non-English dictations keep the D27 skip.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use murmur_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use murmur_core::config::SessionConfig;
use murmur_core::MurmurCore;

/// Reports a fixed transcript in a fixed language.
struct Said(&'static str, &'static str);

impl AsrEngine for Said {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Ok(AsrOutput {
            text: self.0.into(),
            language: Some(self.1.into()),
            ..Default::default()
        })
    }
}

/// An on-device cleaner that echoes its transcript and counts calls.
#[derive(Default)]
struct Local {
    calls: AtomicUsize,
}

impl Cleaner for Local {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(p.transcript.clone())
    }
}

/// Dictate `text` in `language` with the default skip on; how often did
/// the on-device model run, and with what fallback reason?
fn dictate(text: &'static str, language: &'static str) -> (usize, Option<String>) {
    let local = Arc::new(Local::default());
    let core = MurmurCore::from_engines(Arc::new(Said(text, language)), Some(local.clone()));
    let cfg = SessionConfig {
        languages: vec![language.into()],
        ..Default::default()
    };
    assert!(cfg.llm_skip_clean, "the default skip is on");
    core.start_session(cfg).unwrap();
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    core.push_audio(&tone);
    let r = core.commit();
    (local.calls.load(Ordering::SeqCst), r.fallback_reason)
}

#[test]
fn tidy_english_with_spoken_numbers_is_cleaned() {
    for t in [
        "The tram leaves at nine forty from the square.",
        "The library opens again on the fourteenth.",
        "My locker number is six two eight one.",
    ] {
        let (calls, why) = dictate(t, "en");
        assert_eq!(calls, 1, "{t:?}: {why:?}");
        assert_ne!(why.as_deref(), Some("llm_skipped_clean"), "{t:?}");
    }
}

#[test]
fn number_idioms_and_digits_keep_the_skip() {
    for t in [
        "I'll take one of those green mugs.",
        "She waved at me a second ago.",
        "The tram leaves at 9:40 on the 14th.",
    ] {
        assert_eq!(
            dictate(t, "en"),
            (0, Some("llm_skipped_clean".into())),
            "{t:?}"
        );
    }
}

#[test]
fn a_non_english_dictation_keeps_the_skip() {
    // English number words inside another language's dictation do not
    // wake the model: the detector is English-only.
    let t = "ניפגש בתחנה ב nine forty.";
    assert_eq!(dictate(t, "he"), (0, Some("llm_skipped_clean".into())));
}
