//! Session engine behaviour (§9 v2) against MockAsr — no model needed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest, MockAsr};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{CommitStatus, FlowCore};

const CHUNK: usize = 640; // 40 ms @ 16 kHz, Wispr's chunk size

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn push_all(core: &FlowCore, pcm: &[i16]) {
    for c in pcm.chunks(CHUNK) {
        core.push_audio(c);
    }
}

fn mock_core(text: &str) -> (FlowCore, Arc<MockAsr>) {
    let asr = Arc::new(MockAsr::new(text));
    (FlowCore::from_engines(asr.clone(), None), asr)
}

struct FixedCleaner(&'static str, Duration);
impl Cleaner for FixedCleaner {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        std::thread::sleep(self.1);
        Ok(self.0.into())
    }
}

#[test]
fn commit_returns_raw_transcript_without_llm() {
    let (core, _) = mock_core("hello world");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(1000));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.asr_text, "hello world");
    assert!(r.final_text.contains("hello world"), "{r:?}");
    assert_eq!(r.language.as_deref(), Some("en"));
    assert_eq!(r.timings.audio_ms, 1000);
}

#[test]
fn commit_without_audio_is_empty() {
    let (core, asr) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Empty);
    assert_eq!(asr.calls(), 0);
}

#[test]
fn silent_audio_is_empty() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &vec![0i16; 16000]);
    assert_eq!(core.commit().status, CommitStatus::Empty);
}

#[test]
fn commit_without_session_is_error() {
    let (core, _) = mock_core("hello");
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.is_some());
}

#[test]
fn push_before_start_is_dropped() {
    let (core, _) = mock_core("hello");
    assert!(!core.push_audio(&speech(100)));
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(200));
    assert_eq!(core.commit().timings.audio_ms, 200);
}

#[test]
fn late_context_reaches_asr_prompt_and_postprocess() {
    let (core, asr) = mock_core("hello world");
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::word("Kubernetes")],
        },
        ..Default::default()
    };
    core.start_session(cfg).unwrap();
    push_all(&core, &speech(500));
    core.set_context(FieldContext {
        screen_words: vec!["Alice".into()],
        ..Default::default()
    });
    push_all(&core, &speech(500));
    core.commit();
    let p = asr.last_request().unwrap().initial_prompt;
    assert!(p.contains("Kubernetes") && p.contains("Alice"), "{p}");
}

#[test]
fn forced_language_is_passed_to_asr() {
    let (core, asr) = mock_core("hallo");
    core.start_session(SessionConfig {
        languages: vec!["de".into()],
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(300));
    let r = core.commit();
    assert_eq!(asr.last_request().unwrap().language.as_deref(), Some("de"));
    assert_eq!(r.language.as_deref(), Some("de"));
}

#[test]
fn auto_language_with_multiple_prefs() {
    let (core, asr) = mock_core("x");
    core.start_session(SessionConfig {
        languages: vec!["en".into(), "de".into()],
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(300));
    core.commit();
    assert_eq!(asr.last_request().unwrap().language, None);
}

#[test]
fn llm_cleanup_formats() {
    let asr = Arc::new(MockAsr::new("um so lets ship it"));
    let core = FlowCore::from_engines(
        asr,
        Some(Arc::new(FixedCleaner("So let's ship it.", Duration::ZERO))),
    );
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(500));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert_eq!(r.asr_text, "um so lets ship it");
    assert!(r.final_text.starts_with("So let's ship it"), "{r:?}");
}

#[test]
fn llm_timeout_falls_back_to_raw_transcript() {
    let asr = Arc::new(MockAsr::new("hello there"));
    let core = FlowCore::from_engines(
        asr,
        Some(Arc::new(FixedCleaner(
            "Hello there.",
            Duration::from_millis(500),
        ))),
    );
    core.start_session(SessionConfig {
        llm_timeout_ms: 30,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(500));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_timeout"));
    assert!(r.final_text.contains("hello there"));
}

#[test]
fn raw_mode_skips_llm() {
    let asr = Arc::new(MockAsr::new("hello there"));
    let core = FlowCore::from_engines(asr, Some(Arc::new(FixedCleaner("CHANGED", Duration::ZERO))));
    core.start_session(SessionConfig {
        raw_mode: true,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(500));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert!(!r.final_text.contains("CHANGED"));
}

#[test]
fn cancel_discards_session() {
    let (core, asr) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(500));
    core.cancel();
    assert_eq!(core.commit().status, CommitStatus::Error);
    assert_eq!(asr.calls(), 0);
}

#[test]
fn sessions_are_isolated() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(800));
    core.cancel();
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(120));
    assert_eq!(core.commit().timings.audio_ms, 120);
}

#[test]
fn restart_without_commit_replaces_session() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(800));
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(200));
    assert_eq!(core.commit().timings.audio_ms, 200);
}

#[test]
fn max_samples_truncates() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig {
        max_samples: 16_000,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(3000));
    let r = core.commit();
    assert_eq!(r.timings.audio_ms, 1000);
    assert!(r.truncated);
}

#[test]
fn push_audio_does_not_block_on_slow_asr() {
    let asr = Arc::new(MockAsr::new("partial").with_delay(Duration::from_millis(300)));
    let core = FlowCore::from_engines(asr, None);
    core.start_session(SessionConfig {
        partial_interval_ms: Some(40),
        ..Default::default()
    })
    .unwrap();
    let pcm = speech(4000);
    let t = Instant::now();
    for c in pcm.chunks(CHUNK) {
        core.push_audio(c);
        std::thread::sleep(Duration::from_millis(2));
    }
    let per_push = t.elapsed() / (pcm.len() / CHUNK) as u32;
    assert!(
        per_push < Duration::from_millis(10),
        "push took {per_push:?}"
    );
    let r = core.commit();
    assert_eq!(r.timings.audio_ms, 4000, "{r:?}");
    assert_eq!(r.timings.dropped_samples, 0);
}

#[test]
fn partials_are_produced() {
    let (core, _) = mock_core("live text");
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(core.partial(), None);
    push_all(&core, &speech(1000));
    let deadline = Instant::now() + Duration::from_secs(2);
    while core.partial().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(core.partial().as_deref(), Some("live text"));
    core.commit();
}

struct Broken;
impl AsrEngine for Broken {
    fn transcribe(&self, _p: &[f32], _r: &AsrRequest) -> Result<AsrOutput, AsrError> {
        Err(AsrError::Transcribe("decoder exploded".into()))
    }
}

#[test]
fn asr_failure_is_error_status() {
    let core = FlowCore::from_engines(Arc::new(Broken), None);
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(300));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.unwrap().contains("decoder exploded"));
}

#[test]
fn learn_edit_uses_session_dictionary() {
    let (core, _) = mock_core("x");
    core.start_session(SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::word("Kubernetes")],
        },
        ..Default::default()
    })
    .unwrap();
    let c = core.learn_edit("use cubernetes and whisper", "use Kubernetes and Wispr");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].corrected, "Wispr");
}

#[test]
fn commit_result_serializes_to_spec_json() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(300));
    let j = serde_json::to_value(core.commit()).unwrap();
    assert_eq!(j["status"], "RAW_TRANSCRIPT");
    assert!(j["timings"]["asr_ms"].is_u64());
    assert_eq!(j["asr_text"], "hello");
}
