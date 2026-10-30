//! D36: the cleanup chain (`ChainCleaner`), the prompt source a server
//! cleaner rebuilds from (`CleanRequest`), the server's `clean_text` and
//! the client-side `refinish` of a server transcript.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AbortFlag, MockAsr};
use flow_core::cleanup::{
    build_prompt_scoped, clean_text, ChainCleaner, CleanError, CleanPrompt, CleanRequest, Cleaner,
    PromptScope, PromptVersion,
};
use flow_core::config::{AppCategory, CleanupLevel, FieldContext, SessionConfig, Style};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{refinish, CommitStatus, FlowCore};

/// A scripted hop: answer, delay, scope, engine; records what it saw.
struct Hop {
    name: &'static str,
    out: Result<&'static str, CleanError>,
    delay: Duration,
    scope: PromptScope,
    engine: &'static str,
    timeout: Option<u64>,
    calls: AtomicUsize,
    seen: Mutex<Vec<(CleanPrompt, Instant)>>,
}

impl Hop {
    fn new(name: &'static str, out: Result<&'static str, CleanError>) -> Self {
        Self {
            name,
            out,
            delay: Duration::ZERO,
            scope: PromptScope::FULL,
            engine: "local",
            timeout: None,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Cleaner for Hop {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((p.clone(), d));
        let end = Instant::now() + self.delay;
        while Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        match &self.out {
            Ok(s) => Ok(s.to_string()),
            Err(CleanError::Unavailable(c)) => Err(CleanError::Unavailable(c.clone())),
            Err(CleanError::Backend(c)) => Err(CleanError::Backend(c.clone())),
        }
    }
    fn prompt_scope(&self) -> PromptScope {
        self.scope
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
    fn model(&self) -> Option<String> {
        Some(self.name.into())
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout
    }
}

fn ctx() -> FieldContext {
    FieldContext {
        app_id: "com.slack".into(),
        app_category: AppCategory::WorkChat,
        before: "Hi team, ".into(),
        screen_words: vec!["Kubernetes".into()],
        ..Default::default()
    }
}

fn dict() -> Dictionary {
    Dictionary {
        entries: vec![DictEntry::replacement("wispr", "Wispr")],
    }
}

fn prompt(scope: PromptScope) -> CleanPrompt {
    build_prompt_scoped(
        "the demo is on thursday right",
        CleanupLevel::Medium,
        Style::Casual,
        &ctx(),
        &dict(),
        scope,
    )
    .with_language(Some("he"))
    .with_timeout(1000)
}

fn chain(hops: &[&Arc<Hop>]) -> ChainCleaner {
    ChainCleaner::new(
        hops.iter()
            .map(|h| (*h).clone() as Arc<dyn Cleaner>)
            .collect(),
    )
}

#[test]
fn prompt_source_carries_the_request_narrowed_to_the_scope() {
    let p = prompt(PromptScope::FULL);
    let src = p.source.clone().unwrap();
    assert_eq!(src.text, "the demo is on thursday right");
    assert_eq!(src.level, CleanupLevel::Medium);
    assert_eq!(src.style, Style::Casual);
    assert_eq!(src.language.as_deref(), Some("he"));
    assert_eq!(src.timeout_ms, 1000);
    assert_eq!(src.ctx.before, "Hi team, ");
    assert_eq!(src.dictionary, dict());
    // The request rebuilds the same prompt.
    assert_eq!(src.prompt_for(PromptScope::FULL), p);
    // A minimal prompt's source holds no surrounding text, screen words or app.
    let m = prompt(PromptScope::MINIMAL).source.unwrap();
    assert!(m.ctx.before.is_empty() && m.ctx.screen_words.is_empty() && m.ctx.app_id.is_empty());
    assert_eq!(m.ctx.app_category, AppCategory::WorkChat);
    // Never wider than the request's own scope.
    let wide = m.prompt_for(PromptScope::FULL);
    assert!(!wide.user.contains("Hi team"));
}

#[test]
fn clean_request_wire_shape() {
    let req = prompt(PromptScope::FULL).source.unwrap();
    let v = serde_json::to_value(&*req).unwrap();
    assert_eq!(v["text"], "the demo is on thursday right");
    assert_eq!(v["level"], "medium");
    assert_eq!(v["style"], "casual");
    assert_eq!(v["prompt"], "v2");
    assert_eq!(v["language"], "he");
    assert!(v.get("scope").is_none(), "scope is never sent");
    let back: CleanRequest = serde_json::from_value(v).unwrap();
    assert_eq!(back.text, req.text);
    assert_eq!(back.prompt, PromptVersion::V2);
    // Partial requests get defaults.
    let r: CleanRequest = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
    assert_eq!((r.level, r.timeout_ms), (CleanupLevel::Light, 0));
}

#[test]
fn first_answer_wins_and_later_hops_are_not_called() {
    let a = Arc::new(Hop::new("a", Ok("A")));
    let b = Arc::new(Hop::new("b", Ok("B")));
    let c = chain(&[&a, &b]);
    let (r, u) = c.clean_metered(&prompt(PromptScope::FULL), far(), &AbortFlag::default());
    assert_eq!(r.unwrap(), "A");
    assert_eq!(u.model.as_deref(), Some("a"));
    assert_eq!((a.calls(), b.calls()), (1, 0));
}

fn far() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

#[test]
fn refusal_and_error_fall_down_the_list() {
    let a = Arc::new(Hop::new("a", Err(CleanError::Unavailable("no".into()))));
    let b = Arc::new(Hop::new("b", Err(CleanError::Backend("llm_error".into()))));
    let c3 = Arc::new(Hop::new("c", Ok("C")));
    let c = chain(&[&a, &b, &c3]);
    let (r, u) = c.clean_metered(&prompt(PromptScope::FULL), far(), &AbortFlag::default());
    assert_eq!(r.unwrap(), "C");
    assert_eq!(u.model.as_deref(), Some("c"));
    assert_eq!((a.calls(), b.calls(), c3.calls()), (1, 1, 1));
}

#[test]
fn all_failing_reports_the_last_hop_that_ran() {
    let a = Arc::new(Hop::new("a", Err(CleanError::Backend("boom".into()))));
    let b = Arc::new(Hop::new("b", Err(CleanError::Unavailable("off".into()))));
    let (r, u) =
        chain(&[&a, &b]).clean_metered(&prompt(PromptScope::FULL), far(), &AbortFlag::default());
    assert!(matches!(r, Err(CleanError::Unavailable(_))));
    assert!(u.engine.is_some());
}

#[test]
fn a_slow_hop_times_out_within_its_share_and_the_next_answers() {
    let mut slow = Hop::new("slow", Ok("SLOW"));
    slow.delay = Duration::from_millis(2000);
    let slow = Arc::new(slow);
    let fast = Arc::new(Hop::new("fast", Ok("FAST")));
    let c = chain(&[&slow, &fast]);
    // Session timeout 1000 ms in the prompt: the chain's budget is 2000.
    assert_eq!(c.budget_ms(1000), 2000);
    let t = Instant::now();
    let (r, _) = c.clean_metered(
        &prompt(PromptScope::FULL),
        t + Duration::from_millis(2000),
        &AbortFlag::default(),
    );
    assert_eq!(r.unwrap(), "FAST");
    let el = t.elapsed();
    assert!(
        el >= Duration::from_millis(950) && el < Duration::from_millis(1800),
        "{el:?}"
    );
    // The slow hop was given its own deadline.
    let (_, d) = slow.seen.lock().unwrap()[0].clone();
    assert!(d <= t + Duration::from_millis(1100));
}

#[test]
fn no_fallback_without_enough_time_left() {
    let a = Arc::new(Hop::new("a", Err(CleanError::Unavailable("x".into()))));
    let b = Arc::new(Hop::new("b", Ok("B")));
    let c = chain(&[&a, &b]);
    let (r, _) = c.clean_metered(
        &prompt(PromptScope::FULL),
        Instant::now() + Duration::from_millis(100),
        &AbortFlag::default(),
    );
    assert!(r.is_err());
    assert_eq!(b.calls(), 0);
}

#[test]
fn a_cancelled_chain_stops() {
    let a = Arc::new(Hop::new("a", Err(CleanError::Unavailable("x".into()))));
    let b = Arc::new(Hop::new("b", Ok("B")));
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (r, _) =
        chain(&[&a, &b]).clean_metered(&prompt(PromptScope::FULL), far(), &AbortFlag(Some(flag)));
    assert!(r.is_err());
    assert_eq!((a.calls(), b.calls()), (0, 0));
}

#[test]
fn a_narrower_hop_gets_a_rebuilt_prompt_without_the_context() {
    let a = Arc::new(Hop::new("local", Err(CleanError::Unavailable("x".into()))));
    let mut cloud = Hop::new("cloud", Ok("C"));
    cloud.scope = PromptScope::MINIMAL;
    cloud.engine = "cloud";
    let cloud = Arc::new(cloud);
    let c = chain(&[&a, &cloud]);
    assert_eq!(c.prompt_scope(), PromptScope::FULL);
    let p = prompt(c.prompt_scope());
    assert!(p.user.contains("Hi team"));
    c.clean_metered(&p, far(), &AbortFlag::default()).0.unwrap();
    let (local_saw, _) = a.seen.lock().unwrap()[0].clone();
    assert_eq!(local_saw, p);
    let (cloud_saw, _) = cloud.seen.lock().unwrap()[0].clone();
    assert!(!cloud_saw.user.contains("Hi team"), "{}", cloud_saw.user);
    assert!(!cloud_saw.user.contains("Kubernetes"));
    assert!(!cloud_saw.user.contains("com.slack"));
    let src = cloud_saw.source.unwrap();
    assert!(src.ctx.before.is_empty() && src.ctx.app_id.is_empty());
    assert_eq!(src.language.as_deref(), Some("he"));
    assert_eq!(src.timeout_ms, 1000);
}

#[test]
fn a_prompt_without_source_never_reaches_a_narrower_hop() {
    let mut cloud = Hop::new("cloud", Ok("C"));
    cloud.scope = PromptScope::MINIMAL;
    let cloud = Arc::new(cloud);
    let local = Arc::new(Hop::new("local", Err(CleanError::Unavailable("x".into()))));
    let p = CleanPrompt {
        system: String::new(),
        examples: vec![],
        user: "secret selection".into(),
        transcript: String::new(),
        source: None,
    };
    let (r, _) = chain(&[&local, &cloud]).clean_metered(&p, far(), &AbortFlag::default());
    assert!(r.is_err());
    assert_eq!(cloud.calls(), 0);
}

#[test]
fn chain_build_and_ai_parts() {
    let local: Arc<dyn Cleaner> = Arc::new(Hop::new("l", Ok("L")));
    let mut c = Hop::new("c", Ok("C"));
    c.engine = "cloud";
    let cloud: Arc<dyn Cleaner> = Arc::new(c);
    assert!(ChainCleaner::build(vec![]).is_none());
    let one = ChainCleaner::build(vec![local.clone()]).unwrap();
    assert!(Arc::ptr_eq(&one, &local));
    // Cloud first: D30 AI cleanup on the raw transcript, the rest after.
    let cf = ChainCleaner::build(vec![cloud.clone(), local.clone()]).unwrap();
    let (a, rest) = cf.ai_parts().unwrap();
    assert!(Arc::ptr_eq(&a, &cloud) && Arc::ptr_eq(&rest.unwrap(), &local));
    // Local first: no AI split.
    let lf = ChainCleaner::build(vec![local, cloud]).unwrap();
    assert!(lf.ai_parts().is_none());
}

#[test]
fn hop_budgets_sum_and_own_timeouts_count() {
    let mut cloud = Hop::new("c", Ok("C"));
    cloud.timeout = Some(2500);
    let c = chain(&[&Arc::new(Hop::new("l", Ok("L"))), &Arc::new(cloud)]);
    assert_eq!(c.budget_ms(1500), 4000);
}

#[test]
fn server_clean_text_returns_the_answer_or_why_not() {
    let req = prompt(PromptScope::FULL).source.unwrap();
    let ok = Arc::new(Hop::new("srv", Ok("The demo is on Thursday, right?")));
    let (r, u, _) = clean_text(ok.clone(), &req, 120_000);
    assert_eq!(r.unwrap(), "The demo is on Thursday, right?");
    assert_eq!(u.model.as_deref(), Some("srv"));
    // The server builds the full prompt itself, language line included.
    let (saw, _) = ok.seen.lock().unwrap()[0].clone();
    assert!(saw.user.contains("Hi team") && saw.user.contains("LANGUAGE: Hebrew"));
    let mut slow = Hop::new("srv", Ok("late"));
    slow.delay = Duration::from_millis(500);
    let mut req2 = (*req).clone();
    req2.timeout_ms = 100;
    let (r, _, el) = clean_text(Arc::new(slow), &req2, 120_000);
    assert_eq!(r.unwrap_err(), "llm_timeout");
    assert!(el < Duration::from_millis(400));
    let bad = Arc::new(Hop::new("srv", Err(CleanError::Backend("oom".into()))));
    assert_eq!(
        clean_text(bad, &req, 120_000).0.unwrap_err(),
        "llm_error: oom"
    );
    // The server caps the time.
    let mut slow = Hop::new("srv", Ok("late"));
    slow.delay = Duration::from_millis(500);
    let mut req3 = (*req).clone();
    req3.timeout_ms = 60_000;
    let (r, _, _) = clean_text(Arc::new(slow), &req3, 50);
    assert_eq!(r.unwrap_err(), "llm_timeout");
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn session() -> SessionConfig {
    SessionConfig {
        languages: vec!["en".into()],
        dictionary: dict(),
        // Always ask the LLM (no "already clean" skip).
        llm_skip_clean: false,
        ..Default::default()
    }
}

/// What a server without its LLM returns (`cleanup_level: none`).
fn server_result(text: &str) -> flow_core::CommitResult {
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(text)), None);
    let mut cfg = session();
    cfg.cleanup_level = CleanupLevel::None;
    core.start_session(cfg).unwrap();
    core.set_context(ctx());
    core.push_audio(&speech(1000));
    core.commit()
}

#[test]
fn refinish_cleans_a_server_transcript_on_the_client() {
    let r = server_result("um the demo with wispr is on thursday right");
    assert_eq!(r.status, CommitStatus::RawTranscript);
    let cl = Arc::new(Hop::new(
        "local",
        Ok("The demo with wispr is on Thursday, right?"),
    ));
    let out = refinish(&session(), &ctx(), Some(cl.clone()), r.clone());
    assert_eq!(out.status, CommitStatus::Formatted, "{out:?}");
    // Post-processing ran on the cleaned text (the dictionary).
    // (lower-cased after "Hi team, " by the cursor-context rule)
    assert_eq!(
        out.final_text.trim(),
        "the demo with Wispr is on Thursday, right?"
    );
    assert_eq!(out.asr_text, r.asr_text);
    assert_eq!(out.llm.unwrap().model.as_deref(), Some("local"));
    // The cleaner saw the rules' output with the full context.
    let (p, _) = cl.seen.lock().unwrap()[0].clone();
    assert!(p.user.contains("Hi team"));
    assert!(!p.transcript.starts_with("um"));
    // No cleaner: the rules alone.
    let plain = refinish(&session(), &ctx(), None, r.clone());
    assert_eq!(plain.status, CommitStatus::RawTranscript);
    assert!(plain.final_text.contains("Wispr"));
    // Cleanup off in the user's session: no LLM call at all.
    let mut off = session();
    off.cleanup_level = CleanupLevel::None;
    let cl2 = Arc::new(Hop::new("local", Ok("X")));
    let o = refinish(&off, &ctx(), Some(cl2.clone()), r);
    assert_eq!((o.status, cl2.calls()), (CommitStatus::RawTranscript, 0));
}

#[test]
fn refinish_leaves_empty_and_error_results_alone() {
    let mut r = server_result("hello there");
    r.status = CommitStatus::Empty;
    let cl = Arc::new(Hop::new("local", Ok("X")));
    let out = refinish(&session(), &ctx(), Some(cl.clone()), r.clone());
    assert_eq!(out, r);
    r.status = CommitStatus::Error;
    assert_eq!(refinish(&session(), &ctx(), Some(cl.clone()), r.clone()), r);
    assert_eq!(cl.calls(), 0);
}

#[test]
fn a_failed_local_hop_falls_back_inside_a_real_commit() {
    // FlowCore with a chain cleaner: local refuses, "server" answers.
    let local = Arc::new(Hop::new("local", Err(CleanError::Unavailable("x".into()))));
    let server = Arc::new(Hop::new("server", Ok("The demo is on Thursday, right?")));
    let chain: Arc<dyn Cleaner> = Arc::new(chain(&[&local, &server]));
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new("the demo is on thursday right")),
        Some(chain),
    );
    core.start_session(session()).unwrap();
    core.set_context(ctx());
    core.push_audio(&speech(1000));
    let r = core.commit();
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert!(
        r.final_text.contains("demo is on Thursday, right?"),
        "{r:?}"
    );
    assert_eq!(r.llm.unwrap().model.as_deref(), Some("server"));
    // The server hop got the session's timeout to size its request.
    let (p, _) = server.seen.lock().unwrap()[0].clone();
    assert_eq!(p.source.unwrap().timeout_ms, session().llm_timeout_ms);
}
