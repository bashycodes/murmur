//! D65: a Hebrew dictation is never committed in English. The on-device
//! cleanup prompt carries a Hebrew few-shot for Hebrew-script dictations,
//! and an answer that left the transcript's script falls back to the rules
//! text (`llm_language_changed`). All text here is made up.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::SessionConfig;
use flow_core::{CommitResult, CommitStatus, FlowCore};

const HE: &str = "תזכיר לי לקנות לחם וחלב בדרך הביתה ואם יש זמן גם לעבור בדואר לאסוף את החבילה";

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

/// An on-device cleaner with a fixed answer (`None` = echo the
/// transcript) that keeps the last prompt it saw.
struct Local(Option<&'static str>, Mutex<Option<CleanPrompt>>);

impl Cleaner for Local {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        *self.1.lock().unwrap() = Some(p.clone());
        Ok(self.0.map_or_else(|| p.transcript.clone(), str::to_string))
    }
}

fn dictate(
    text: &'static str,
    language: &'static str,
    answer: Option<&'static str>,
) -> (CommitResult, CleanPrompt) {
    let local = Arc::new(Local(answer, Mutex::new(None)));
    let core = FlowCore::from_engines(Arc::new(Said(text, language)), Some(local.clone()));
    core.start_session(SessionConfig {
        languages: vec![language.into()],
        ..Default::default()
    })
    .unwrap();
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    core.push_audio(&tone);
    let r = core.commit();
    let p = local.1.lock().unwrap().clone().expect("the model ran");
    (r, p)
}

#[test]
fn a_translated_hebrew_dictation_keeps_the_hebrew_rules_text() {
    let (r, _) = dictate(
        HE,
        "he",
        Some("Remind me to buy bread and milk on the way home, and pick up the parcel."),
    );
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_language_changed"));
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert!(
        r.final_text.starts_with("תזכיר לי לקנות לחם"),
        "{}",
        r.final_text
    );
}

#[test]
fn a_hebrew_answer_is_used_and_the_prompt_has_a_hebrew_turn() {
    let (r, p) = dictate(HE, "he", None);
    assert_eq!(r.status, CommitStatus::Formatted, "{:?}", r.fallback_reason);
    assert!(p.examples.last().unwrap().1.contains("החבילה הגיעה"));
}

#[test]
fn english_dictations_are_unchanged() {
    let t = "so the train was late and I missed the connection I'll be home soon";
    let (r, p) = dictate(
        t,
        "en",
        Some("The train was late and I missed the connection. I'll be home soon."),
    );
    assert_eq!(r.status, CommitStatus::Formatted);
    assert!(p.examples.iter().all(|(u, _)| !u.contains("LANGUAGE:")));
}
