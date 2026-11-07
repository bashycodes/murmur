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
    assert_eq!(r.fallback_reason, None, "skipped, not failed");
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

fn run_text(asr_text: &str, cfg: SessionConfig, ctx: FieldContext) -> flow_core::CommitResult {
    let (core, _) = mock_core(asr_text);
    core.start_session(cfg).unwrap();
    core.set_context(ctx);
    push_all(&core, &speech(300));
    core.commit()
}

#[test]
fn postprocess_dictionary_and_cursor_context_applied() {
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::replacement("btw", "by the way")],
        },
        ..Default::default()
    };
    let ctx = FieldContext {
        before: "I said that".into(),
        app_id: "notepad".into(),
        ..Default::default()
    };
    let r = run_text("Btw we left.", cfg, ctx);
    assert_eq!(r.final_text, " by the way we left. ");
    assert_eq!(r.asr_text, "Btw we left.");
}

#[test]
fn pronoun_i_keeps_capital_mid_sentence() {
    // Intentional divergence from Wispr vector ctx_pronoun_I_quirk.
    let ctx = FieldContext {
        before: "and then".into(),
        ..Default::default()
    };
    assert_eq!(
        run_text("I left early.", SessionConfig::default(), ctx).final_text,
        " I left early. "
    );
}

#[test]
fn screen_word_proper_noun_keeps_capital() {
    let ctx = FieldContext {
        before: "I met".into(),
        screen_words: vec!["John".into()],
        ..Default::default()
    };
    assert_eq!(
        run_text("John Smith came.", SessionConfig::default(), ctx).final_text,
        " John Smith came. "
    );
}

#[test]
fn press_enter_is_reported_on_windows_only() {
    let r = run_text(
        "Send it press enter",
        SessionConfig::default(),
        FieldContext::default(),
    );
    assert!(r.press_enter);
    // Wispr trims before the phrase; the closing period (D25) moves onto
    // the text, as with server-punctuated "Send it press enter."
    assert_eq!(r.final_text, "Send it.");
    let android = SessionConfig {
        platform: flow_core::config::Platform::Android,
        ..Default::default()
    };
    let r = run_text("Send it press enter", android, FieldContext::default());
    assert!(!r.press_enter);
}

#[test]
fn chat_app_drops_trailing_period_by_classification() {
    let ctx = FieldContext {
        app_id: "slack".into(),
        ..Default::default()
    };
    assert_eq!(
        run_text("Sounds good.", SessionConfig::default(), ctx).final_text,
        "Sounds good "
    );
    let android = SessionConfig {
        platform: flow_core::config::Platform::Android,
        ..Default::default()
    };
    let ctx = FieldContext {
        app_id: "com.whatsapp".into(),
        ..Default::default()
    };
    assert_eq!(
        run_text("Sounds good.", android, ctx).final_text,
        "Sounds good "
    );
}

#[test]
fn android_unknown_context_gets_no_trailing_space() {
    let android = SessionConfig {
        platform: flow_core::config::Platform::Android,
        ..Default::default()
    };
    let ctx = FieldContext {
        context_known: false,
        ..Default::default()
    };
    assert_eq!(
        run_text("Hello world.", android, ctx).final_text,
        "Hello world."
    );
}

#[test]
fn terminal_skips_llm() {
    let asr = Arc::new(MockAsr::new("ls dash la"));
    let core = FlowCore::from_engines(asr, Some(Arc::new(FixedCleaner("CHANGED", Duration::ZERO))));
    core.start_session(SessionConfig::default()).unwrap();
    core.set_context(FieldContext {
        app_id: "windowsterminal".into(),
        ..Default::default()
    });
    push_all(&core, &speech(300));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(r.fallback_reason, None, "skipped, not failed");
    assert!(!r.final_text.contains("CHANGED"));
}

#[test]
fn commit_preempts_in_flight_partial() {
    // A partial decode that is running when the user releases must not add
    // its full duration to release->text latency.
    let asr = Arc::new(MockAsr::new("x").with_delay(Duration::from_millis(800)));
    let core = FlowCore::from_engines(asr, None);
    core.start_session(SessionConfig {
        partial_interval_ms: Some(40),
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(500));
    std::thread::sleep(Duration::from_millis(100)); // partial decode now in flight
    let r = core.commit();
    // Final decode itself costs 800 ms; an un-aborted partial would add up to 800 more.
    assert!(r.timings.total_ms < 1300, "{:?}", r.timings);
    assert_eq!(r.status, CommitStatus::RawTranscript);
}

#[test]
fn whole_utterance_snippet_skips_llm() {
    let asr = Arc::new(MockAsr::new("My email."));
    let core = FlowCore::from_engines(
        asr,
        Some(Arc::new(FixedCleaner(
            "Totally different words here",
            Duration::ZERO,
        ))),
    );
    core.start_session(SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::snippet("my email", "me@example.com")],
        },
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(300));
    let r = core.commit();
    assert_eq!(r.final_text, "me@example.com");
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(r.fallback_reason, None, "LLM must not even run: {r:?}");
}

#[test]
fn dict_matcher_is_built_once_per_dictionary() {
    let (core, _) = mock_core("btw hi");
    let dict = |r: &str| SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::replacement("btw", r)],
        },
        ..Default::default()
    };
    for _ in 0..3 {
        core.start_session(dict("by the way")).unwrap();
        push_all(&core, &speech(300));
        assert!(core.commit().final_text.contains("by the way"));
    }
    assert_eq!(core.dict_matcher_builds(), 1);
    core.start_session(dict("BTW")).unwrap();
    push_all(&core, &speech(300));
    assert!(core.commit().final_text.contains("BTW"));
    assert_eq!(core.dict_matcher_builds(), 2);
}

fn noise(ms: usize, amp: i16, seed: u32) -> Vec<i16> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..ms * 16)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ((x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp as f32) as i16
        })
        .collect()
}

#[test]
fn vad_skips_asr_on_room_noise() {
    let (core, asr) = mock_core("Thank you.");
    core.start_session(SessionConfig::default()).unwrap();
    // RMS ≈ 0.0035: passes the old whole-buffer RMS gate, has no speech.
    push_all(&core, &noise(2000, 200, 7));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Empty, "{r:?}");
    assert_eq!(r.empty_reason.as_deref(), Some("no_speech"));
    assert_eq!(asr.calls(), 0);
}

#[test]
fn vad_trims_leading_and_trailing_silence_before_asr() {
    let (core, asr) = mock_core("hello");
    core.start_session(SessionConfig::default()).unwrap();
    let pcm = [vec![0i16; 32_000], speech(1000), vec![0i16; 32_000]].concat();
    push_all(&core, &pcm);
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.timings.audio_ms, 5000);
    // 1 s of speech + 300 ms head + 400 ms tail padding (± one frame).
    let got = asr.last_len();
    assert!((1680 * 16..=1720 * 16).contains(&got), "{got}");
    assert!((1680..=1720).contains(&r.timings.asr_audio_ms), "{r:?}");
}

#[test]
fn vad_off_passes_the_whole_buffer() {
    let (core, asr) = mock_core("hello");
    core.start_session(SessionConfig {
        vad: false,
        ..Default::default()
    })
    .unwrap();
    let pcm = [vec![0i16; 32_000], speech(1000)].concat();
    push_all(&core, &pcm);
    core.commit();
    assert_eq!(asr.last_len(), pcm.len());
}

#[test]
fn stock_phrase_on_a_breath_is_suppressed() {
    let (core, _) = mock_core("Thank you.");
    core.start_session(SessionConfig::default()).unwrap();
    // 100 ms burst in a second of silence: VAD sees speech, too little.
    push_all(
        &core,
        &[vec![0i16; 8000], speech(100), vec![0i16; 8000]].concat(),
    );
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Empty, "{r:?}");
    assert_eq!(r.empty_reason.as_deref(), Some("hallucination"));
    assert_eq!(r.asr_text, "Thank you.");
    assert!(r.final_text.is_empty() && r.segments.is_empty());
}

#[test]
fn spoken_stock_phrase_is_kept() {
    let (core, _) = mock_core("Thank you.");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(800));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert!(r.final_text.contains("Thank you."));
}

#[test]
fn hallucination_guard_can_be_disabled() {
    let (core, _) = mock_core("Thank you.");
    core.start_session(SessionConfig {
        hallucination_guard: false,
        ..Default::default()
    })
    .unwrap();
    push_all(
        &core,
        &[vec![0i16; 8000], speech(100), vec![0i16; 8000]].concat(),
    );
    assert_eq!(core.commit().status, CommitStatus::RawTranscript);
}

/// Records prefill hints; cleans to a fixed text.
#[derive(Default)]
struct PrefillSpy {
    prefills: std::sync::Mutex<Vec<CleanPrompt>>,
    cancels: std::sync::atomic::AtomicUsize,
}
impl Cleaner for PrefillSpy {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        Ok(p.transcript.clone())
    }
    fn prefill(&self, p: &CleanPrompt) {
        self.prefills.lock().unwrap().push(p.clone());
    }
    fn cancel_prefill(&self) {
        self.cancels
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn llm_prefill_follows_context_and_partials_while_speaking() {
    let spy = Arc::new(PrefillSpy::default());
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("hello world")), Some(spy.clone()));
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..Default::default()
    })
    .unwrap();
    // Session start warms the session's level/style with an empty transcript.
    assert!(wait_until(|| !spy.prefills.lock().unwrap().is_empty()));
    assert!(spy.prefills.lock().unwrap()[0].transcript.is_empty());
    core.set_context(FieldContext {
        app_id: "slack.exe".into(),
        ..Default::default()
    });
    assert!(wait_until(|| spy
        .prefills
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.user.contains("slack.exe"))));
    push_all(&core, &speech(500));
    assert!(wait_until(|| spy.prefills.lock().unwrap().iter().any(
        |p| p.transcript.eq_ignore_ascii_case("hello world") && p.user.contains("slack.exe")
    )));
    let n = spy.prefills.lock().unwrap().len();
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert!(spy.cancels.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    // Unchanged context + partial: no duplicate hints.
    assert!(n <= 4, "{n} prefills");
}

/// The prefill hint carries the text prompt v2 gets at commit (the spoken
/// rules' output), so a KV-caching cleaner reuses the transcript's tokens
/// too, not only the user message's header.
#[test]
fn llm_prefill_hint_is_the_rules_text_the_commit_prompt_starts_with() {
    #[derive(Default)]
    struct Spy {
        prefills: std::sync::Mutex<Vec<String>>,
        cleaned: std::sync::Mutex<Vec<String>>,
    }
    impl Cleaner for Spy {
        fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
            self.cleaned.lock().unwrap().push(p.transcript.clone());
            Ok(p.transcript.clone())
        }
        fn prefill(&self, p: &CleanPrompt) {
            self.prefills.lock().unwrap().push(p.transcript.clone());
        }
    }
    let spy = Arc::new(Spy::default());
    let asr = "um so the meeting moved to tuesday comma right after lunch";
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(asr)), Some(spy.clone()));
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        llm_skip_clean: false,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(500));
    assert!(wait_until(|| spy
        .prefills
        .lock()
        .unwrap()
        .iter()
        .any(|t| !t.is_empty())));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    let hint = spy
        .prefills
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|t| !t.is_empty())
        .cloned()
        .unwrap();
    let cleaned = spy.cleaned.lock().unwrap()[0].clone();
    assert_ne!(
        hint, asr,
        "the hint is the rules' text, not the raw partial"
    );
    assert!(
        cleaned.starts_with(hint.trim_end()),
        "hint {hint:?} is not where the commit prompt {cleaned:?} starts"
    );
}

#[test]
fn llm_prefill_is_skipped_when_llm_would_not_run() {
    for cfg in [
        SessionConfig {
            raw_mode: true,
            ..Default::default()
        },
        SessionConfig {
            llm_prefill: false,
            ..Default::default()
        },
    ] {
        let spy = Arc::new(PrefillSpy::default());
        let core = FlowCore::from_engines(Arc::new(MockAsr::new("hi")), Some(spy.clone()));
        core.start_session(cfg).unwrap();
        push_all(&core, &speech(300));
        std::thread::sleep(Duration::from_millis(100));
        core.commit();
        assert!(spy.prefills.lock().unwrap().is_empty());
    }
}

#[test]
fn spoken_cleanup_runs_by_default_and_can_be_disabled() {
    let r = run_text(
        "um so send it to Sarah no wait to Tom period",
        SessionConfig::default(),
        FieldContext::default(),
    );
    assert_eq!(r.final_text, "Send it to Tom. ", "{r:?}");
    assert_eq!(r.asr_text, "um so send it to Sarah no wait to Tom period");
    let off = SessionConfig {
        spoken_cleanup: false,
        ..Default::default()
    };
    let r = run_text("um send it period", off, FieldContext::default());
    assert!(r.final_text.contains("period"), "{r:?}");
}

#[test]
fn spoken_cleanup_runs_with_the_llm_switched_off() {
    // D27: Android with local AI off sends level none + raw_mode; it must
    // still get the deterministic layer (it used to get none of it).
    use flow_core::config::{CleanupLevel, Platform};
    for (level, raw) in [
        (CleanupLevel::None, true),
        (CleanupLevel::None, false),
        (CleanupLevel::Light, true),
    ] {
        let cfg = SessionConfig {
            cleanup_level: level,
            raw_mode: raw,
            platform: Platform::Android,
            ..Default::default()
        };
        let r = run_text(
            "um so send it to Sarah no wait to Tom period",
            cfg,
            FieldContext::default(),
        );
        assert_eq!(
            r.final_text, "Send it to Tom. ",
            "{level:?} raw={raw} {r:?}"
        );
        // The closing period (D25) is part of the deterministic layer.
        let cfg = SessionConfig {
            cleanup_level: level,
            raw_mode: raw,
            ..Default::default()
        };
        let r = run_text("send the deck to Tom", cfg, FieldContext::default());
        assert_eq!(
            r.final_text, "send the deck to Tom. ",
            "{level:?} raw={raw} {r:?}"
        );
    }
}

#[test]
fn llm_never_runs_at_level_none_or_raw_mode() {
    struct Shout;
    impl Cleaner for Shout {
        fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
            Ok("LLM WAS HERE".into())
        }
    }
    use flow_core::config::CleanupLevel;
    for (level, raw) in [
        (CleanupLevel::None, true),
        (CleanupLevel::None, false),
        (CleanupLevel::Light, true),
    ] {
        let core = FlowCore::from_engines(
            Arc::new(MockAsr::new("send it period")),
            Some(Arc::new(Shout)),
        );
        let cfg = SessionConfig {
            cleanup_level: level,
            raw_mode: raw,
            ..Default::default()
        };
        core.start_session(cfg).unwrap();
        push_all(&core, &speech(300));
        let r = core.commit();
        assert_eq!(r.final_text, "Send it. ", "{level:?} raw={raw} {r:?}");
        assert_eq!(r.status, CommitStatus::RawTranscript);
    }
}

#[test]
fn llm_is_skipped_when_it_cannot_help() {
    struct Shout;
    impl Cleaner for Shout {
        fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
            Ok("Can you send me the slides?".into())
        }
    }
    let run = |skip: bool| {
        let core = FlowCore::from_engines(
            Arc::new(MockAsr::new("Can you send me the slides?")),
            Some(Arc::new(Shout)),
        );
        core.start_session(SessionConfig {
            llm_skip_clean: skip,
            ..Default::default()
        })
        .unwrap();
        push_all(&core, &speech(300));
        core.commit()
    };
    let r = run(true);
    assert_eq!(r.final_text, "Can you send me the slides? ", "{r:?}");
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_skipped_clean"));
    assert_eq!(r.timings.llm_ms, 0);
    let r = run(false);
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert!(r.fallback_reason.is_none());
}

#[test]
fn verbatim_is_spoken_cleanup_off_at_level_none() {
    let cfg = SessionConfig {
        cleanup_level: flow_core::config::CleanupLevel::None,
        spoken_cleanup: false,
        ..Default::default()
    };
    let r = run_text("um send it period", cfg, FieldContext::default());
    assert!(r.final_text.contains("um send it period"), "{r:?}");
    assert!(
        !r.final_text.contains("period."),
        "no closing period: {r:?}"
    );
}

#[test]
fn only_filler_is_empty() {
    let r = run_text(
        "Um, uh, hmm.",
        SessionConfig::default(),
        FieldContext::default(),
    );
    assert_eq!(r.status, CommitStatus::Empty, "{r:?}");
    assert_eq!(r.empty_reason.as_deref(), Some("only_filler"));
}

#[test]
fn llm_sees_the_spoken_cleanup_output() {
    struct Echo(std::sync::Mutex<String>);
    impl Cleaner for Echo {
        fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
            *self.0.lock().unwrap() = p.transcript.clone();
            Ok(p.transcript.clone())
        }
    }
    let echo = Arc::new(Echo(Default::default()));
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new("um I I think it works period")),
        Some(echo.clone()),
    );
    // The cleaned text is short and clean: without this the LLM is skipped.
    core.start_session(SessionConfig {
        llm_skip_clean: false,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(500));
    let r = core.commit();
    assert_eq!(*echo.0.lock().unwrap(), "I think it works.");
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
}

// ---- S11: a previous session's partial never leaks into the next ------------

/// Partial decodes block on a gate the first time (an in-flight decode that
/// ignores the abort flag) and say which call they were.
struct GatedPartialAsr {
    calls: std::sync::atomic::AtomicUsize,
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl AsrEngine for GatedPartialAsr {
    fn transcribe(&self, _pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        if req.purpose != flow_core::asr::AsrPurpose::Partial {
            return Ok(AsrOutput {
                text: "final".into(),
                ..Default::default()
            });
        }
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            if let Some(tx) = self.entered.lock().unwrap().take() {
                let _ = tx.send(());
            }
            if let Some(rx) = self.release.lock().unwrap().take() {
                let _ = rx.recv();
            }
            return Ok(AsrOutput {
                text: "old secret".into(),
                ..Default::default()
            });
        }
        Ok(AsrOutput {
            text: format!("new {n}"),
            ..Default::default()
        })
    }
}

fn partial_cfg() -> SessionConfig {
    SessionConfig {
        partial_interval_ms: Some(100),
        vad: false,
        ..Default::default()
    }
}

#[test]
fn stale_partial_from_a_cancelled_session_is_dropped() {
    let (etx, erx) = std::sync::mpsc::channel();
    let (rtx, rrx) = std::sync::mpsc::channel();
    let asr = Arc::new(GatedPartialAsr {
        calls: Default::default(),
        entered: std::sync::Mutex::new(Some(etx)),
        release: std::sync::Mutex::new(Some(rrx)),
    });
    let core = FlowCore::from_engines(asr, None);
    core.start_session(partial_cfg()).unwrap();
    push_all(&core, &speech(500));
    erx.recv_timeout(Duration::from_secs(5))
        .expect("partial decode started");
    core.cancel();
    core.start_session(partial_cfg()).unwrap();
    rtx.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_ne!(
        core.partial().as_deref(),
        Some("old secret"),
        "the cancelled session's preview leaked into the new one"
    );
    core.cancel();
}

#[test]
fn cancel_and_commit_clear_the_partial() {
    for commit in [false, true] {
        let (core, _) = mock_core("hello there");
        core.start_session(partial_cfg()).unwrap();
        push_all(&core, &speech(500));
        let t0 = Instant::now();
        while core.partial().is_none() && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(core.partial().is_some(), "partial produced");
        if commit {
            core.commit();
        } else {
            core.cancel();
        }
        assert_eq!(core.partial(), None, "commit={commit}");
    }
}

// ---- C4: cancel() during commit aborts the running pipeline -----------------

/// Final decodes run until aborted (or 5 s).
struct AbortableAsr;
impl AsrEngine for AbortableAsr {
    fn transcribe(&self, _pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let t0 = Instant::now();
        while !req.abort.is_set() && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        if req.abort.is_set() {
            return Err(AsrError::Transcribe("aborted".into()));
        }
        Ok(AsrOutput {
            text: "too late".into(),
            ..Default::default()
        })
    }
}

#[test]
fn cancel_during_commit_aborts_it_and_frees_the_worker() {
    let core = FlowCore::from_engines(Arc::new(AbortableAsr), None);
    let cfg = SessionConfig {
        vad: false,
        ..Default::default()
    };
    core.start_session(cfg.clone()).unwrap();
    push_all(&core, &speech(500));
    let (r, took) = std::thread::scope(|sc| {
        let h = sc.spawn(|| {
            let t = Instant::now();
            (core.commit(), t.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        core.cancel();
        h.join().unwrap()
    });
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.as_deref().unwrap().contains("cancelled"), "{r:?}");
    // The worker is free: the next session commits normally (and its own
    // commit is not affected by the earlier cancel).
    core.start_session(cfg).unwrap();
    push_all(&core, &speech(200));
    core.cancel();
    assert_eq!(core.commit().status, CommitStatus::Error, "no session");
}

// ---- C6: a start that overlaps a commit keeps the committing session's state

/// Final decodes signal `entered`, then wait for `release`.
struct GatedFinalAsr {
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: std::sync::Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}
impl AsrEngine for GatedFinalAsr {
    fn transcribe(&self, _pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        if let Some(tx) = self.entered.lock().unwrap().take() {
            let _ = tx.send(());
        }
        if let Some(rx) = self.release.lock().unwrap().take() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
        Ok(AsrOutput {
            text: "Sounds good.".into(),
            ..Default::default()
        })
    }
}

#[test]
fn start_during_commit_keeps_the_committing_sessions_context_and_drops() {
    let (etx, erx) = std::sync::mpsc::channel();
    let (rtx, rrx) = std::sync::mpsc::channel();
    let core = FlowCore::from_engines(
        Arc::new(GatedFinalAsr {
            entered: std::sync::Mutex::new(Some(etx)),
            release: std::sync::Mutex::new(Some(rrx)),
        }),
        None,
    );
    let cfg = SessionConfig {
        vad: false,
        ring_capacity_samples: 16_000,
        ..Default::default()
    };
    core.start_session(cfg.clone()).unwrap();
    core.set_context(FieldContext {
        app_id: "slack".into(),
        ..Default::default()
    });
    // One 3 s push into a 1 s ring: 2 s are dropped, deterministically.
    assert!(core.push_audio(&speech(3000)));
    let r = std::thread::scope(|sc| {
        let h = sc.spawn(|| core.commit());
        erx.recv_timeout(Duration::from_secs(5))
            .expect("final decode started");
        // Hands-free back-to-back: the next dictation starts in another app.
        core.start_session(cfg.clone()).unwrap();
        core.set_context(FieldContext {
            app_id: "notepad".into(),
            ..Default::default()
        });
        rtx.send(()).unwrap();
        h.join().unwrap()
    });
    assert_eq!(
        r.final_text, "Sounds good ",
        "cleaned with slack's context: {r:?}"
    );
    assert_eq!(r.timings.dropped_samples, 32_000, "{r:?}");
    core.cancel();
}

// ---- C18: audio past max_samples is truncated, never counted as dropped ----

#[test]
fn audio_past_max_samples_is_not_counted_as_dropped() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig {
        max_samples: 8_000,
        ring_capacity_samples: 16_000,
        vad: false,
        ..Default::default()
    })
    .unwrap();
    // Three 1 s pushes into a 1 s ring, with time for the worker to drain
    // between them: nothing overflows unless samples past the cap stay in
    // the ring.
    for _ in 0..3 {
        assert!(core.push_audio(&speech(1000)));
        std::thread::sleep(Duration::from_millis(500));
    }
    let r = core.commit();
    assert!(r.truncated, "{r:?}");
    assert_eq!(r.timings.audio_ms, 500);
    assert_eq!(r.timings.dropped_samples, 0, "{r:?}");
}

// ---- Phase 0 of docs/proposals/streaming-asr-latency.md ----------------

#[test]
fn partial_window_is_capped_at_12_s_with_an_ellipsis() {
    let (core, asr) = mock_core("tail words");
    core.start_session(SessionConfig {
        partial_interval_ms: Some(500),
        vad: false,
        ring_capacity_samples: 16_000 * 40,
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(20_000));
    // The worker may preview a shorter buffer while the push is running;
    // wait for the one over the capped window.
    assert!(
        wait_until(|| core.partial().as_deref() == Some("… tail words")),
        "{:?}",
        core.partial()
    );
    assert_eq!(asr.last_len(), 12 * 16_000);
    // The final decode still sees everything.
    let r = core.commit();
    assert_eq!(asr.last_len(), 20 * 16_000);
    assert_eq!(r.asr_text, "tail words");
}

#[test]
fn short_dictation_preview_has_no_ellipsis() {
    let (core, _) = mock_core("hello");
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(2000));
    assert!(wait_until(|| core.partial().is_some()));
    assert_eq!(core.partial().as_deref(), Some("hello"));
}

#[test]
fn no_preview_redecode_in_trailing_silence() {
    let (core, asr) = mock_core("hello");
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(1500));
    push_all(&core, &noise(200, 30, 1));
    assert!(wait_until(|| core.partial().is_some()));
    // Let the worker drain and preview what was pushed so far.
    std::thread::sleep(Duration::from_millis(300));
    let before = asr.calls();
    // 2 s of silence in 100 ms steps: every step is due for a preview, but
    // the last one already covered all the speech.
    for i in 0..20 {
        push_all(&core, &noise(100, 30, 10 + i));
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    // At most the previews due in the first 400 ms of silence.
    assert!(asr.calls() <= before + 4, "{} -> {}", before, asr.calls());
    // New speech previews again.
    let mid = asr.calls();
    push_all(&core, &speech(500));
    assert!(wait_until(|| asr.calls() > mid));
    assert_eq!(core.commit().status, CommitStatus::RawTranscript);
}

#[test]
fn wait_ms_reports_a_commit_queued_behind_the_worker() {
    // Partials are aborted at commit, so an uninterruptible engine is what
    // makes the commit wait: model one that ignores the abort flag.
    struct Stubborn(std::sync::atomic::AtomicUsize);
    impl AsrEngine for Stubborn {
        fn transcribe(&self, _pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
            if req.purpose == flow_core::asr::AsrPurpose::Partial {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(400));
            }
            Ok(AsrOutput {
                text: "ok".into(),
                ..Default::default()
            })
        }
    }
    let asr = Arc::new(Stubborn(Default::default()));
    let core = FlowCore::from_engines(asr.clone(), None);
    core.start_session(SessionConfig {
        partial_interval_ms: Some(40),
        ..Default::default()
    })
    .unwrap();
    push_all(&core, &speech(1000));
    assert!(wait_until(|| asr
        .0
        .load(std::sync::atomic::Ordering::SeqCst)
        > 0));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert!(r.timings.wait_ms > 0, "{:?}", r.timings);
    assert!(r.timings.wait_ms <= r.timings.total_ms, "{:?}", r.timings);
    // Off the JSON when 0, present otherwise.
    let j = serde_json::to_value(&r).unwrap();
    assert!(j["timings"]["wait_ms"].as_u64().unwrap() > 0);
    let (core, _) = mock_core("x");
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, &speech(500));
    let r = core.commit();
    let j = serde_json::to_value(&r).unwrap();
    for k in ["segments", "pre_asr_ms"] {
        assert!(j["timings"].get(k).is_none(), "{k}: {j}");
    }
}

/// Ticket 6aded21e: four dictation shapes the fallback rules mangled (a quote, money and a height), with AI cleanup off as Android sends it.
#[test]
fn fallback_rules_fix_the_four_ab_shapes_without_ai() {
    use flow_core::config::{CleanupLevel, Platform};
    let run = |s: &str| {
        let cfg = SessionConfig {
            cleanup_level: CleanupLevel::None,
            raw_mode: true,
            platform: Platform::Android,
            ..Default::default()
        };
        let ctx = FieldContext {
            app_id: "com.google.android.keep".into(),
            ..Default::default()
        };
        run_text(s, cfg, ctx).final_text.trim_end().to_string()
    };
    assert_eq!(
        run("okay first thing send the invoice second thing call the landlord and third thing please remind me about the dentist"),
        "Okay:\n\n1. Send the invoice\n2. Call the landlord\n3. Please remind me about the dentist"
    );
    assert_eq!(
        run("he told me and I quote I'm never doing that again unquote and then he did it the next day"),
        "He told me, \"I'm never doing that again,\" and then he did it the next day."
    );
    for s in [
        "we spent about two hundred and fifty dollars on the trip in parentheses not counting gas which was another sixty dollars",
        // As Parakeet wrote it on the phone: amounts glued to the word.
        "We spent about$250 on the trip, in parentheses, not counting gas, which was another$60.",
    ] {
        let money = run(s);
        assert!(money.contains("about $250 on the trip"), "{money}");
        assert!(money.contains("another $60"), "{money}");
    }
    assert_eq!(
        run("he is about a hundred and fifty eight cm tall"),
        "He is about 158 cm tall."
    );
}

/// Like the sherpa and whisper engines: "" below a whole-clip RMS of 0.003.
struct GatedAsr(std::sync::Mutex<Option<f32>>);
impl AsrEngine for GatedAsr {
    fn transcribe(&self, pcm: &[f32], _req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let rms = (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
        *self.0.lock().unwrap() = Some(rms);
        Ok(AsrOutput {
            text: if rms < 0.003 {
                String::new()
            } else {
                "shalom".into()
            },
            language: Some("he".into()),
            no_speech_prob: None,
        })
    }
}

/// Commit `pcm` with VAD on; also the RMS the ASR got (None: not called).
fn gated_commit(pcm: &[i16]) -> (flow_core::CommitResult, Option<f32>) {
    let asr = Arc::new(GatedAsr(Default::default()));
    let core = FlowCore::from_engines(asr.clone(), None);
    core.start_session(SessionConfig::default()).unwrap();
    push_all(&core, pcm);
    let r = core.commit();
    let rms = *asr.0.lock().unwrap();
    (r, rms)
}

/// Quiet speech-like audio: 150 Hz voiced bursts at ±`amp` (PCM16),
/// 400 ms on / 200 ms off after 500 ms of lead-in, over hiss at ±`floor`.
fn quiet_words(words: usize, amp: f32, floor: i16) -> Vec<i16> {
    let hiss = noise(600 * words + 1000, floor, 11);
    hiss.iter()
        .enumerate()
        .map(|(i, &h)| {
            let t = i as f32 / 16000.0;
            let in_word = i >= 8000 && i < 8000 + words * 9600 && (i - 8000) % 9600 < 6400;
            let v = if in_word {
                amp * (0.7 * (std::f32::consts::TAU * 150.0 * t).sin()
                    + 0.3 * (std::f32::consts::TAU * 450.0 * t).sin())
            } else {
                0.0
            };
            h.saturating_add(v as i16)
        })
        .collect()
}

#[test]
fn quiet_speech_reaches_the_asr_at_a_usable_level() {
    // FLEURS he (2026-10-31 phone bench): speech at ≈ -60 dBFS was dropped
    // as `no_speech`, and at ≈ -50 dBFS blanked by the engine's RMS gate.
    for amp in [45.0, 150.0] {
        let (r, rms) = gated_commit(&quiet_words(6, amp, 2));
        assert_eq!(r.status, CommitStatus::RawTranscript, "amp {amp}: {r:?}");
        assert_eq!(r.asr_text, "shalom");
        assert!(rms.unwrap() >= 0.003, "amp {amp}: {rms:?}");
    }
}

#[test]
fn silence_and_quiet_noise_are_still_no_speech() {
    for pcm in [
        vec![0i16; 32000],
        noise(2000, 2, 12),
        noise(2000, 30, 13),
        noise(2000, 150, 14),
    ] {
        let (r, rms) = gated_commit(&pcm);
        assert_eq!(r.status, CommitStatus::Empty, "{r:?}");
        assert_eq!(r.empty_reason.as_deref(), Some("no_speech"));
        assert_eq!(rms, None, "the ASR was not called");
    }
}
