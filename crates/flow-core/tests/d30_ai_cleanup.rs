//! D30: AI cleanup is the main path. With a cloud cleaner active the raw
//! transcript goes straight to it (prompt v4c, loosened guard); a timeout,
//! error or reject runs today's pipeline (fallback rules, then the
//! on-device LLM), and without an on-device LLM the fallback rules alone.
//! Without a cloud cleaner nothing changes. Fake cleaners only: no network.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AbortFlag, MockAsr};
use flow_core::cleanup::{
    build_prompt, CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope, RoutedCleaner,
};
use flow_core::config::{AppCategory, CleanupLevel, FieldContext, SessionConfig, Style};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{CommitResult, CommitStatus, FlowCore, Usd};

/// Returns `out` (or `err`) after `delay`; counts calls; records prompts.
struct Fake {
    out: Result<&'static str, CleanError>,
    delay: Duration,
    calls: AtomicUsize,
    engine: &'static str,
    timeout: Option<u64>,
    seen: Mutex<Vec<CleanPrompt>>,
}

impl Fake {
    fn ok(out: &'static str) -> Self {
        Self {
            out: Ok(out),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
            engine: "local",
            timeout: None,
            seen: Mutex::new(Vec::new()),
        }
    }
    fn err(e: CleanError) -> Self {
        Self {
            out: Err(e),
            ..Self::ok("")
        }
    }
    fn cloud(mut self) -> Self {
        self.engine = "cloud";
        self.timeout = Some(2500);
        self
    }
    fn slow(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
    fn timeout(mut self, ms: u64) -> Self {
        self.timeout = Some(ms);
        self
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn prompt(&self, i: usize) -> CleanPrompt {
        self.seen.lock().unwrap()[i].clone()
    }
}

impl Cleaner for Fake {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(p.clone());
        std::thread::sleep(self.delay);
        match &self.out {
            Ok(s) => Ok((*s).into()),
            Err(CleanError::Unavailable(c)) => Err(CleanError::Unavailable(c.clone())),
            Err(e) => Err(CleanError::Backend(e.code().into())),
        }
    }
    fn prompt_scope(&self) -> PromptScope {
        if self.engine == "cloud" {
            PromptScope::MINIMAL
        } else {
            PromptScope::FULL
        }
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        d: Instant,
        c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let r = self.clean_cancellable(p, d, c);
        let cost = (self.engine == "cloud").then_some(Usd(0.0016));
        (
            r,
            CleanUsage {
                engine: Some(self.engine.into()),
                model: Some(format!("{}-model", self.engine)),
                cost_usd: cost,
                ..Default::default()
            },
        )
    }
}

/// A raw transcript with spoken punctuation, a filler and a number word:
/// the fallback rules would change all three.
const RAW: &str = "um so the demo is on thursday at three comma right question mark";
/// What AI cleanup answers for it.
const AI: &str = "The demo is on Thursday at 3, right?";
/// What the on-device LLM answers (for the rules' text).
const LOCAL: &str = "The demo is on Thursday at 3, right?";

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn dictate_cfg(core: &FlowCore, cfg: SessionConfig) -> CommitResult {
    core.start_session(cfg).unwrap();
    core.set_context(FieldContext {
        app_category: AppCategory::Other,
        screen_words: vec!["SCREENSECRET".into()],
        ..Default::default()
    });
    for c in speech(300).chunks(640) {
        core.push_audio(c);
    }
    core.commit()
}

fn dictate(core: &FlowCore) -> CommitResult {
    dictate_cfg(core, SessionConfig::default())
}

fn routed(cloud: &Arc<Fake>, local: Option<&Arc<Fake>>) -> Arc<dyn Cleaner> {
    Arc::new(RoutedCleaner::new(
        cloud.clone(),
        local.map(|l| l.clone() as Arc<dyn Cleaner>),
    ))
}

/// A core with on-device LLM `local` (if any) and the cloud cleaner
/// installed the way the router does (`set_cleaner`).
fn core_with(raw: &str, cloud: &Arc<Fake>, local: Option<&Arc<Fake>>) -> FlowCore {
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(raw)),
        local.map(|l| l.clone() as Arc<dyn Cleaner>),
    );
    core.set_cleaner(Some(routed(cloud, local)));
    core
}

/// Today's pipeline: the same dictation with no cloud cleaner.
fn today(raw: &str, local: Option<Arc<Fake>>) -> CommitResult {
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(raw)),
        local.map(|l| l as Arc<dyn Cleaner>),
    );
    dictate(&core)
}

#[test]
fn cloud_ok_uses_the_ai_answer() {
    let cloud = Arc::new(Fake::ok(AI).cloud());
    let local = Arc::new(Fake::ok(LOCAL));
    let r = dictate(&core_with(RAW, &cloud, Some(&local)));
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert_eq!(r.cleanup_route.as_deref(), Some("cloud"));
    assert_eq!(r.cloud_fallback, None);
    assert_eq!(r.fallback_reason, None);
    assert_eq!(r.final_text.trim_end(), AI);
    assert_eq!((cloud.calls(), local.calls()), (1, 0));
    let llm = r.llm.expect("usage");
    assert_eq!(llm.engine.as_deref(), Some("cloud"));
    assert_eq!(llm.cost_usd, Some(Usd(0.0016)));
}

#[test]
fn cloud_gets_the_raw_transcript_with_prompt_v4c_and_no_rules_pre_pass() {
    let cloud = Arc::new(Fake::ok(AI).cloud());
    let r = dictate(&core_with(RAW, &cloud, None));
    assert_eq!(r.asr_text, RAW);
    let p = cloud.prompt(0);
    // Exactly the transcript: no filler drop, no "comma" -> ",", no digits.
    assert_eq!(p.transcript, RAW);
    assert!(
        p.user.contains(&format!("TRANSCRIPT: \u{ab}{RAW}\u{bb}")),
        "{}",
        p.user
    );
    // Prompt v4c, not v2.
    let v4c = flow_core::cleanup::build_prompt_versioned(
        RAW,
        CleanupLevel::Light,
        Style::Default,
        &FieldContext::default(),
        &Dictionary::default(),
        PromptScope::MINIMAL,
        flow_core::cleanup::PromptVersion::V4c,
    );
    assert_eq!(p.system, v4c.system);
    assert_eq!(p.examples, v4c.examples);
    // The cloud's minimal scope still holds.
    assert!(!p.user.contains("SCREENSECRET"));
}

#[test]
fn ai_cleanup_runs_even_when_the_rules_would_skip_the_llm() {
    // D27's llm_skip_clean is for the on-device LLM; AI cleanup always runs.
    let cloud = Arc::new(Fake::ok("The demo is on Thursday.").cloud());
    let r = dictate(&core_with("The demo is on Thursday.", &cloud, None));
    assert_eq!(cloud.calls(), 1);
    assert_eq!(r.cleanup_route.as_deref(), Some("cloud"));
}

#[test]
fn cloud_timeout_falls_back_to_the_local_pipeline() {
    let cloud = Arc::new(
        Fake::ok(AI)
            .cloud()
            .timeout(200)
            .slow(Duration::from_millis(1500)),
    );
    let local = Arc::new(Fake::ok(LOCAL));
    let t = Instant::now();
    let r = dictate_cfg(
        &core_with(RAW, &cloud, Some(&local)),
        SessionConfig {
            llm_skip_clean: false,
            ..Default::default()
        },
    );
    assert!(
        t.elapsed() < Duration::from_millis(1200),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.cloud_fallback.as_deref(), Some("llm_timeout"));
    assert_eq!(r.cleanup_route.as_deref(), Some("local"), "{r:?}");
    assert_eq!(r.status, CommitStatus::Formatted);
    assert_eq!(r.llm.as_ref().unwrap().engine.as_deref(), Some("local"));
    // The on-device LLM got today's input: the rules' text, prompt v2,
    // full scope.
    let lp = local.prompt(0);
    assert_eq!(lp.transcript, "The demo is on thursday at three, right?");
    assert!(lp.user.contains("SCREENSECRET"));
    assert_eq!(lp.system, build_v2(&lp.transcript).system);
    assert_eq!(cloud.calls(), 1);
}

fn build_v2(t: &str) -> CleanPrompt {
    build_prompt(
        t,
        CleanupLevel::Light,
        Style::Default,
        &FieldContext::default(),
        &Dictionary::default(),
    )
}

#[test]
fn cloud_reject_falls_back_to_the_local_pipeline() {
    // Not backed by the transcript: the loosened guard rejects it.
    let cloud = Arc::new(Fake::ok("Please buy a new car tomorrow.").cloud());
    let local = Arc::new(Fake::ok(LOCAL));
    let r = dictate_cfg(
        &core_with(RAW, &cloud, Some(&local)),
        SessionConfig {
            llm_skip_clean: false,
            ..Default::default()
        },
    );
    assert!(
        matches!(
            r.cloud_fallback.as_deref(),
            Some("llm_unsupported" | "llm_dropped")
        ),
        "{r:?}"
    );
    assert_eq!(r.cleanup_route.as_deref(), Some("local"));
    assert_eq!(r.final_text.trim_end(), LOCAL);
    assert_eq!((cloud.calls(), local.calls()), (1, 1));
}

#[test]
fn cloud_error_or_refusal_falls_back_to_the_local_pipeline() {
    for e in [
        CleanError::Unavailable("cloud_cleanup_not_configured".into()),
        CleanError::Backend("provider_error".into()),
    ] {
        let code = e.code().to_string();
        let cloud = Arc::new(Fake::err(e).cloud());
        let local = Arc::new(Fake::ok(LOCAL));
        let r = dictate_cfg(
            &core_with(RAW, &cloud, Some(&local)),
            SessionConfig {
                llm_skip_clean: false,
                ..Default::default()
            },
        );
        assert_eq!(
            r.cloud_fallback.as_deref(),
            Some(format!("llm_error: {code}").as_str())
        );
        assert_eq!(r.cleanup_route.as_deref(), Some("local"), "{r:?}");
        assert_eq!(local.calls(), 1);
    }
}

#[test]
fn local_pipeline_after_a_cloud_failure_keeps_llm_skip_clean() {
    let cloud = Arc::new(Fake::ok("Something else entirely here.").cloud());
    let local = Arc::new(Fake::ok(LOCAL));
    let r = dictate(&core_with("The demo is on Thursday.", &cloud, Some(&local)));
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_skipped_clean"));
    assert_eq!(r.cleanup_route.as_deref(), Some("rules"));
    assert!(r.cloud_fallback.is_some());
    assert_eq!(local.calls(), 0);
}

#[test]
fn no_local_model_gives_the_fallback_rules() {
    let cloud = Arc::new(Fake::ok("Please buy a new car tomorrow.").cloud());
    let r = dictate(&core_with(RAW, &cloud, None));
    assert_eq!(r.cleanup_route.as_deref(), Some("rules"), "{r:?}");
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert!(r.cloud_fallback.is_some());
    // The cloud's reason is what the debug log shows as `llm`.
    assert_eq!(r.fallback_reason, r.cloud_fallback);
    // Same text as today's pipeline without any LLM.
    assert_eq!(r.final_text, today(RAW, None).final_text);
    // The cloud's cost is still reported.
    assert_eq!(r.llm.unwrap().cost_usd, Some(Usd(0.0016)));
}

#[test]
fn a_bare_cloud_cleaner_falls_back_to_the_rules() {
    // `cleanup_order = [cloud]` / no phone model: the router installs the
    // gate alone.
    let cloud = Arc::new(Fake::ok("").cloud());
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(RAW)), None);
    core.set_cleaner(Some(cloud.clone() as Arc<dyn Cleaner>));
    let r = dictate(&core);
    assert_eq!(r.cloud_fallback.as_deref(), Some("llm_empty"));
    assert_eq!(r.cleanup_route.as_deref(), Some("rules"));
    assert_eq!(r.final_text, today(RAW, None).final_text);
}

#[test]
fn cloud_off_is_todays_pipeline() {
    // Only the on-device LLM: no AI cleanup fields, the LLM sees the
    // rules' text with prompt v2, and the result equals a core where the
    // same cleaner was installed through set_cleaner(None).
    let local = Arc::new(Fake::ok(LOCAL));
    let cfg = SessionConfig {
        llm_skip_clean: false,
        ..Default::default()
    };
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(RAW)),
        Some(local.clone() as Arc<dyn Cleaner>),
    );
    let r = dictate_cfg(&core, cfg.clone());
    assert_eq!(r.cleanup_route, None);
    assert_eq!(r.cloud_fallback, None);
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("cleanup_route") && !json.contains("cloud_fallback"));
    let lp = local.prompt(0);
    assert_eq!(lp.system, build_v2(&lp.transcript).system);
    assert_ne!(lp.transcript, RAW, "the rules ran first");
    // A local-first routed cleaner (cleanup_order [local, cloud] with the
    // phone model loaded) is not AI cleanup either.
    let cloud = Arc::new(Fake::ok(AI).cloud());
    let core2 = FlowCore::from_engines(Arc::new(MockAsr::new(RAW)), None);
    core2.set_cleaner(Some(Arc::new(RoutedCleaner::new(
        local.clone(),
        Some(cloud.clone() as Arc<dyn Cleaner>),
    ))));
    let r2 = dictate_cfg(&core2, cfg);
    assert_eq!(cloud.calls(), 0);
    assert_eq!(r2.final_text, r.final_text);
    assert_eq!(r2.cleanup_route, None);
}

#[test]
fn no_ai_cleanup_for_raw_mode_level_none_terminals_and_snippets() {
    let cloud = Arc::new(Fake::ok(AI).cloud());
    let core = core_with(RAW, &cloud, None);
    for cfg in [
        SessionConfig {
            raw_mode: true,
            ..Default::default()
        },
        SessionConfig {
            cleanup_level: CleanupLevel::None,
            ..Default::default()
        },
    ] {
        let r = dictate_cfg(&core, cfg);
        assert_eq!(r.cleanup_route, None);
    }
    core.start_session(SessionConfig::default()).unwrap();
    core.set_context(FieldContext {
        app_category: AppCategory::Terminal,
        ..Default::default()
    });
    for c in speech(300).chunks(640) {
        core.push_audio(c);
    }
    assert_eq!(core.commit().cleanup_route, None);
    // A whole-utterance snippet trigger is pasted as its expansion.
    let snip = core_with("my address", &cloud, None);
    let r = dictate_cfg(
        &snip,
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::snippet("my address", "1 Main St")],
            },
            ..Default::default()
        },
    );
    assert_eq!(r.cleanup_route, None);
    assert_eq!(cloud.calls(), 0);
}

#[test]
fn ai_lists_get_a_blank_line_after_the_intro() {
    let cloud = Arc::new(
        Fake::ok("For the trip we need:\n<ol><li>Tickets</li><li>A hotel</li><li>A car</li></ol>")
            .cloud(),
    );
    let r = dictate(&core_with(
        "for the trip we need first tickets second a hotel and third a car",
        &cloud,
        None,
    ));
    assert_eq!(r.cleanup_route.as_deref(), Some("cloud"), "{r:?}");
    assert!(
        r.final_text
            .starts_with("For the trip we need:\n\n1. Tickets\n2. A hotel\n3. A car"),
        "{:?}",
        r.final_text
    );
}
