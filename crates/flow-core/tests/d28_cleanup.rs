//! D28 W0: prompt scope, RoutedCleaner, `FlowCore::set_cleaner`
//! (docs/proposals/openrouter-only-cloud.md §2.5). Fake cleaners only.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AbortFlag, MockAsr};
use flow_core::cleanup::{
    build_prompt, build_prompt_scoped, run_cleanup, CleanError, CleanOutcome, CleanPrompt,
    CleanUsage, Cleaner, FallbackReason, PromptScope, RoutedCleaner,
};
use flow_core::config::{AppCategory, CleanupLevel, FieldContext, SessionConfig, Style};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{CommitStatus, FlowCore, Usd};

const SENTINELS: [&str; 5] = [
    "BEFORESECRET",
    "SELECTEDSECRET",
    "AFTERSECRET",
    "com.secret.appid",
    "SCREENSECRET",
];

fn leaky_ctx() -> FieldContext {
    FieldContext {
        app_id: "com.secret.appid".into(),
        app_category: AppCategory::WorkChat,
        before: "hello BEFORESECRET".into(),
        selected: "SELECTEDSECRET".into(),
        after: "AFTERSECRET bye".into(),
        screen_words: vec!["SCREENSECRET".into()],
        ..Default::default()
    }
}

fn dict() -> Dictionary {
    Dictionary {
        entries: vec![DictEntry::word("Kubernetes")],
    }
}

fn all_text(p: &CleanPrompt) -> String {
    let mut s = format!("{}\n{}\n{}", p.system, p.user, p.transcript);
    for (u, a) in &p.examples {
        s.push_str(u);
        s.push_str(a);
    }
    s
}

#[test]
fn minimal_scope_sends_nothing_beyond_the_dictation() {
    let p = build_prompt_scoped(
        "the dictated words",
        CleanupLevel::Light,
        Style::Casual,
        &leaky_ctx(),
        &dict(),
        PromptScope::default(),
    );
    let all = all_text(&p);
    for s in SENTINELS {
        assert!(!all.contains(s), "{s} leaked into the prompt:\n{all}");
    }
    // What may leave: transcript, category, style, dictionary.
    assert!(p.user.contains("the dictated words"));
    assert!(p.user.contains("CATEGORY: work chat"), "{}", p.user);
    assert!(p.user.contains("STYLE: casual"), "{}", p.user);
    assert!(p.user.contains("DICTIONARY: Kubernetes"), "{}", p.user);
    assert!(p.user.contains("APP: the focused app"), "{}", p.user);
    assert_eq!(PromptScope::default(), PromptScope::MINIMAL);
}

#[test]
fn each_scope_item_admits_only_its_own_context() {
    let ctx = leaky_ctx();
    let with = |scope| {
        all_text(&build_prompt_scoped(
            "x",
            CleanupLevel::Light,
            Style::Default,
            &ctx,
            &dict(),
            scope,
        ))
    };
    let s = with(PromptScope {
        surrounding_text: true,
        ..Default::default()
    });
    assert!(
        s.contains("BEFORESECRET") && s.contains("SELECTEDSECRET") && s.contains("AFTERSECRET")
    );
    assert!(!s.contains("SCREENSECRET") && !s.contains("com.secret.appid"));
    let s = with(PromptScope {
        screen_words: true,
        ..Default::default()
    });
    assert!(s.contains("SCREENSECRET"));
    assert!(!s.contains("BEFORESECRET") && !s.contains("com.secret.appid"));
    let s = with(PromptScope {
        app_id: true,
        ..Default::default()
    });
    assert!(s.contains("com.secret.appid"));
    assert!(!s.contains("BEFORESECRET") && !s.contains("SCREENSECRET"));
}

#[test]
fn full_scope_is_byte_identical_to_build_prompt() {
    for (ctx, level, style) in [
        (leaky_ctx(), CleanupLevel::Light, Style::Default),
        (
            FieldContext::default(),
            CleanupLevel::Medium,
            Style::VeryCasual,
        ),
        (leaky_ctx(), CleanupLevel::High, Style::Excited),
    ] {
        assert_eq!(
            build_prompt("um so yes", level, style, &ctx, &dict()),
            build_prompt_scoped("um so yes", level, style, &ctx, &dict(), PromptScope::FULL)
        );
    }
}

// ---- fake cleaners ------------------------------------------------------

/// Returns `out` (or `err`) after `delay`; counts calls; records prompts.
struct Fake {
    out: Result<&'static str, CleanError>,
    delay: Duration,
    calls: AtomicUsize,
    scope: PromptScope,
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
            scope: PromptScope::FULL,
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
        self.scope = PromptScope::MINIMAL;
        self
    }
    fn slow(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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
        self.scope
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
        let cost = (self.engine == "cloud").then_some(Usd(0.0007));
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

fn prompt(t: &str) -> CleanPrompt {
    build_prompt(
        t,
        CleanupLevel::Light,
        Style::Default,
        &FieldContext::default(),
        &Dictionary::default(),
    )
}

fn routed(p: &Arc<Fake>, f: &Arc<Fake>) -> Arc<RoutedCleaner> {
    Arc::new(RoutedCleaner::new(
        p.clone(),
        Some(f.clone() as Arc<dyn Cleaner>),
    ))
}

const IN: &str = "the demo is on thursday right";
const OUT: &str = "The demo is on Thursday, right?";

#[test]
fn fast_failure_falls_back() {
    let p = Arc::new(Fake::err(CleanError::Unavailable("connect_failed".into())).cloud());
    let f = Arc::new(Fake::ok(OUT));
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(2500));
    assert_eq!(r, CleanOutcome::Formatted(OUT.into()));
    assert_eq!((p.calls(), f.calls()), (1, 1));
}

#[test]
fn backend_error_is_final() {
    let p = Arc::new(Fake::err(CleanError::Backend("provider_error".into())).cloud());
    let f = Arc::new(Fake::ok(OUT));
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(2500));
    assert_eq!(
        r,
        CleanOutcome::Fallback(FallbackReason::Error("provider_error".into()))
    );
    assert_eq!(f.calls(), 0);
}

#[test]
fn an_answer_is_final_and_used_as_given() {
    // D58: no output guard. Even an answer the transcript does not back is
    // used as the primary gave it; no second LLM.
    let p = Arc::new(Fake::ok("Please buy a new car tomorrow.").cloud());
    let f = Arc::new(Fake::ok(OUT));
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(2500));
    assert_eq!(
        r,
        CleanOutcome::Formatted("Please buy a new car tomorrow.".into())
    );
    assert_eq!(f.calls(), 0);
}

#[test]
fn an_empty_answer_falls_back_and_is_final() {
    let p = Arc::new(Fake::ok("  ").cloud());
    let f = Arc::new(Fake::ok(OUT));
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(2500));
    assert_eq!(r, CleanOutcome::Fallback(FallbackReason::Empty));
    assert_eq!(f.calls(), 0);
}

#[test]
fn timeout_gives_deterministic_text_and_no_fallback() {
    let p = Arc::new(Fake::ok(OUT).cloud().slow(Duration::from_millis(400)));
    let f = Arc::new(Fake::ok(OUT));
    let t = Instant::now();
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(100));
    assert_eq!(r, CleanOutcome::Fallback(FallbackReason::Timeout));
    assert!(
        t.elapsed() < Duration::from_millis(300),
        "{:?}",
        t.elapsed()
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(f.calls(), 0);
}

#[test]
fn fast_failure_too_close_to_the_deadline_does_not_fall_back() {
    let p = Arc::new(
        Fake::err(CleanError::Unavailable("connect_failed".into()))
            .cloud()
            .slow(Duration::from_millis(300)),
    );
    let f = Arc::new(Fake::ok(OUT));
    // 1000 ms budget, 300 ms spent: 700 < MIN_FALLBACK_MS left.
    let r = run_cleanup(routed(&p, &f), prompt(IN), IN, Duration::from_millis(1000));
    assert_eq!(
        r,
        CleanOutcome::Fallback(FallbackReason::Error("connect_failed".into()))
    );
    assert_eq!(f.calls(), 0);
}

#[test]
fn routed_scope_is_the_intersection() {
    let p = Arc::new(Fake::ok(OUT).cloud());
    let f = Arc::new(Fake::ok(OUT));
    assert_eq!(routed(&p, &f).prompt_scope(), PromptScope::MINIMAL);
    assert_eq!(routed(&f, &f).prompt_scope(), PromptScope::FULL);
    assert_eq!(routed(&p, &f).engine(), "cloud");
}

// ---- FlowCore::set_cleaner ---------------------------------------------

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn dictate(core: &FlowCore, cfg: SessionConfig, ctx: FieldContext) -> flow_core::CommitResult {
    core.start_session(cfg).unwrap();
    core.set_context(ctx);
    for c in speech(300).chunks(640) {
        core.push_audio(c);
    }
    core.commit()
}

fn cfg() -> SessionConfig {
    SessionConfig {
        llm_skip_clean: false,
        ..Default::default()
    }
}

#[test]
fn set_cleaner_installs_and_none_restores_the_built_in() {
    let local = Arc::new(Fake::ok(OUT));
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(IN)), Some(local.clone()));
    let cloud = Arc::new(Fake::ok(OUT).cloud());
    core.set_cleaner(Some(routed(&cloud, &local)));
    let r = dictate(&core, cfg(), leaky_ctx());
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert_eq!((cloud.calls(), local.calls()), (1, 0));
    let llm = r.llm.expect("llm usage");
    assert_eq!(llm.engine.as_deref(), Some("cloud"));
    assert_eq!(llm.cost_usd, Some(Usd(0.0007)));
    // The cloud saw only the dictation.
    let seen = all_text(&cloud.seen.lock().unwrap()[0]);
    for s in SENTINELS {
        assert!(!seen.contains(s), "{s} reached the cloud cleaner");
    }
    // Built-in accessor unchanged; active one is the routed cleaner.
    assert!(Arc::ptr_eq(
        &core.cleaner().unwrap(),
        &(local.clone() as Arc<dyn Cleaner>)
    ));
    assert_eq!(core.active_cleaner().unwrap().engine(), "cloud");

    core.set_cleaner(None);
    let r = dictate(&core, cfg(), leaky_ctx());
    assert_eq!((cloud.calls(), local.calls()), (1, 1));
    assert_eq!(r.llm.unwrap().engine.as_deref(), Some("local"));
    // The local LLM gets the full context.
    assert!(all_text(&local.seen.lock().unwrap()[0]).contains("BEFORESECRET"));
}

#[test]
fn llm_skip_clean_still_skips_the_local_llm_after_ai_cleanup() {
    // D30: AI cleanup (cloud) always runs on the raw transcript; when it
    // falls back (here: an empty answer), the fallback pipeline still
    // skips the on-device LLM on a clean dictation.
    let local = Arc::new(Fake::ok(OUT));
    let cloud = Arc::new(Fake::ok(" ").cloud());
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("The demo is on Thursday.")), None);
    core.set_cleaner(Some(routed(&cloud, &local)));
    let r = dictate(&core, SessionConfig::default(), FieldContext::default());
    assert_eq!(
        r.fallback_reason.as_deref(),
        Some("llm_skipped_clean"),
        "{r:?}"
    );
    assert_eq!((cloud.calls(), local.calls()), (1, 0));
    assert_eq!(r.cleanup_route.as_deref(), Some("rules"));
    assert_eq!(r.llm.unwrap().engine.as_deref(), Some("cloud"));
}

#[test]
fn cleaner_timeout_overrides_the_session_llm_timeout() {
    let mut cloud = Fake::ok(OUT).cloud().slow(Duration::from_millis(250));
    cloud.timeout = Some(1000);
    let cloud = Arc::new(cloud);
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(IN)), None);
    core.set_cleaner(Some(cloud.clone()));
    let r = dictate(
        &core,
        SessionConfig {
            llm_timeout_ms: 100,
            ..cfg()
        },
        FieldContext::default(),
    );
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
}

#[test]
fn cloud_down_gives_deterministic_text_within_the_timeout() {
    // Cloud hangs, no local LLM: the commit returns the deterministic
    // text at the cleaner's timeout (+ slack), never blocking dictation.
    let mut cloud = Fake::ok(OUT).cloud().slow(Duration::from_secs(5));
    cloud.timeout = Some(300);
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(IN)), None);
    core.set_cleaner(Some(Arc::new(cloud)));
    let t = Instant::now();
    let r = dictate(&core, cfg(), FieldContext::default());
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_timeout"));
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
}
