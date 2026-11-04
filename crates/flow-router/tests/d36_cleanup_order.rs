//! D36: who cleans a dictation is decided by the cleanup order
//! {local, server, cloud}, independent of the STT route. A real `FlowCore`
//! (MockAsr + fake on-device LLM) is the local route; a fake remote backend
//! is the server (STT sessions and text-only `clean`); a fake OpenRouter
//! cleaner is the cloud.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::asr::{AbortFlag, MockAsr};
use flow_core::cleanup::{CleanError, CleanPrompt, CleanRequest, Cleaner};
use flow_core::config::CleanupLevel;
use flow_core::FlowCore;
use flow_protocol::{Cleaned, HelloOk, FEATURE_CLEAN, FEATURE_SUMMARY};
use flow_router::*;

const KEY: &str = "sk-or-LEAKCHECK-d36-0123456789";
const IN: &str = "the demo is on thursday right";
const LOCAL_OUT: &str = "The demo is on Thursday right?";
const SERVER_OUT: &str = "The demo is on Thursday, right.";
const CLOUD_OUT: &str = "The demo is on Thursday, right?";
/// What the server's own session pipeline returns when it cleaned.
const SESSION_OUT: &str = "server cleaned text";

/// Fake on-device LLM; `fail` makes it error.
struct Local {
    calls: AtomicUsize,
    fail: Mutex<bool>,
}
impl Cleaner for Local {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if *self.fail.lock().unwrap() {
            return Err(CleanError::Backend("llm_crashed".into()));
        }
        Ok(LOCAL_OUT.into())
    }
}

/// Fake OpenRouter cleaner.
struct Cloud {
    calls: AtomicUsize,
    seen: Mutex<Vec<CleanPrompt>>,
}
impl Cleaner for Cloud {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.clean_metered(p, d, &AbortFlag::default()).0
    }
    fn engine(&self) -> &'static str {
        "cloud"
    }
    fn estimate_cost_usd(&self, _p: &CleanPrompt) -> Option<f64> {
        Some(0.001)
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        _d: Instant,
        _c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(p.clone());
        (Ok(CLOUD_OUT.into()), CleanUsage::default())
    }
}

/// How the fake server answers.
#[derive(Clone, Copy, PartialEq)]
enum Answer {
    Text,
    Error(&'static str),
    Disconnected,
}

/// Fake flow-server: records the session configs it got and the text
/// cleanup requests; its session result is "cleaned" unless the client
/// asked it to skip the LLM (or it has no LLM).
struct Server {
    llm: Mutex<bool>,
    /// Advertises D53's `summary` with `clean` (a newer server).
    summary: Mutex<bool>,
    hello_known: Mutex<bool>,
    /// `Some(llm)`: the hello of an older connection, still cached until
    /// the next connection (`warm`, `clean`, a session) renews it.
    old_hello: Mutex<Option<bool>>,
    warms: AtomicUsize,
    commit_fails: Mutex<bool>,
    answer: Mutex<Answer>,
    starts: Mutex<Vec<CleanupLevel>>,
    /// Each session's `llm_skip_clean`.
    skips: Mutex<Vec<bool>>,
    cleans: Mutex<Vec<CleanRequest>>,
}

impl Server {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            llm: Mutex::new(true),
            summary: Mutex::new(true),
            hello_known: Mutex::new(true),
            old_hello: Mutex::new(None),
            warms: AtomicUsize::new(0),
            commit_fails: Mutex::new(false),
            answer: Mutex::new(Answer::Text),
            starts: Mutex::new(Vec::new()),
            skips: Mutex::new(Vec::new()),
            cleans: Mutex::new(Vec::new()),
        })
    }
    fn cleans(&self) -> usize {
        self.cleans.lock().unwrap().len()
    }
    fn levels(&self) -> Vec<CleanupLevel> {
        self.starts.lock().unwrap().clone()
    }
    fn warms(&self) -> usize {
        self.warms.load(Ordering::SeqCst)
    }
    /// A new connection: its hello is the server's current one.
    fn connected(&self) {
        *self.old_hello.lock().unwrap() = None;
    }
}

fn result(status: &str, final_text: &str, llm: bool) -> CommitResult {
    let mut v = serde_json::json!({
        "status": status, "asr_text": IN, "final_text": final_text, "language": "en",
        "timings": {"audio_ms":300,"asr_ms":10,"llm_ms":0,"postprocess_ms":0,"total_ms":10,"dropped_samples":0},
        "press_enter": false, "fallback_reason": null, "truncated": false, "error": null,
    });
    if llm {
        v["llm"] = serde_json::json!({"engine": "local", "model": "server-llm"});
    }
    serde_json::from_value(v).unwrap()
}

impl Backend for Server {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }
    fn ready(&self) -> bool {
        true
    }
    fn start_session(&self, c: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.connected();
        self.starts.lock().unwrap().push(c.cleanup_level);
        self.skips.lock().unwrap().push(c.llm_skip_clean);
        Ok(StartInfo::default())
    }
    fn push_audio(&self, _p: &[i16]) -> bool {
        true
    }
    fn set_context(&self, _c: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, _d: Instant) -> Result<CommitResult, BackendError> {
        if *self.commit_fails.lock().unwrap() {
            return Err(BackendError::new(Failure::Disconnected, "gone"));
        }
        let level = *self.starts.lock().unwrap().last().unwrap();
        Ok(
            if level == CleanupLevel::None || !*self.llm.lock().unwrap() {
                result("RAW_TRANSCRIPT", "The demo is on thursday right.", false)
            } else {
                result("FORMATTED", SESSION_OUT, true)
            },
        )
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        HealthReport::default()
    }
    fn warm(&self) -> Result<(), BackendError> {
        self.warms.fetch_add(1, Ordering::SeqCst);
        self.connected();
        Ok(())
    }
    fn server_hello(&self) -> Option<HelloOk> {
        let old = *self.old_hello.lock().unwrap();
        let llm = old.unwrap_or(*self.llm.lock().unwrap());
        self.hello_known.lock().unwrap().then(|| HelloOk {
            llm,
            // The build with the AI model is the newer one.
            server: if llm {
                "flow-server 0.12.0"
            } else {
                "flow-server 0.11.0"
            }
            .into(),
            features: match (llm, *self.summary.lock().unwrap()) {
                (true, true) => vec![FEATURE_CLEAN.into(), FEATURE_SUMMARY.into()],
                (true, false) => vec![FEATURE_CLEAN.into()],
                _ => Vec::new(),
            },
            ..Default::default()
        })
    }
    fn clean_text(&self, req: &CleanRequest, _d: Instant) -> Result<Cleaned, BackendError> {
        self.connected();
        self.cleans.lock().unwrap().push(req.clone());
        if !*self.llm.lock().unwrap() {
            return Ok(Cleaned {
                error: Some("no_llm".into()),
                ..Default::default()
            });
        }
        // flow-remote's refusal of a meeting task to an older server.
        if !req.task.is_cleanup() && !*self.summary.lock().unwrap() {
            return Ok(Cleaned {
                error: Some("no_summary".into()),
                ..Default::default()
            });
        }
        match *self.answer.lock().unwrap() {
            Answer::Text => Ok(Cleaned {
                text: Some(SERVER_OUT.into()),
                model: Some("server-llm".into()),
                ..Default::default()
            }),
            Answer::Error(e) => Ok(Cleaned {
                error: Some(e.into()),
                ..Default::default()
            }),
            Answer::Disconnected => Err(BackendError::new(Failure::Disconnected, "gone")),
        }
    }
}

fn consented() -> OpenRouterConfig {
    let mut o = OpenRouterConfig::default();
    o.cleanup.consent_host = "openrouter.ai".into();
    o.cleanup.consent_at_ms = 1_792_540_900_000;
    o
}

/// Client STT route, server configured (cleanup only), cloud cleanup
/// allowed and consented; `order` = the cleanup order.
fn cfg(order: &[CleanupRoute]) -> RouterConfig {
    RouterConfig {
        allow_server: false,
        allow_client: true,
        server: ServerConfig {
            host: "100.64.0.10".into(),
            token: "0123456789abcdef".into(),
            ..Default::default()
        },
        allow_cloud_cleanup: true,
        cleanup_order: order.to_vec(),
        cloud: CloudConfig {
            openrouter: consented(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// `cfg` with the server as the STT route (local STT as its fallback).
fn server_stt(order: &[CleanupRoute]) -> RouterConfig {
    let mut c = cfg(order);
    c.allow_server = true;
    c.order = vec![Route::Server, Route::Client];
    c
}

struct Rig {
    router: Router,
    local: Arc<Local>,
    server: Arc<Server>,
    cloud: Arc<Cloud>,
}

impl Rig {
    fn local_calls(&self) -> usize {
        self.local.calls.load(Ordering::SeqCst)
    }
    fn cloud_calls(&self) -> usize {
        self.cloud.calls.load(Ordering::SeqCst)
    }
}

fn rig_with(c: RouterConfig, local_llm: bool) -> Rig {
    rig_heard(c, local_llm, IN)
}

/// `rig_with` whose speech model hears `heard`.
fn rig_heard(c: RouterConfig, local_llm: bool, heard: &str) -> Rig {
    let local = Arc::new(Local {
        calls: AtomicUsize::new(0),
        fail: Mutex::new(false),
    });
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(heard)),
        local_llm.then(|| local.clone() as Arc<dyn Cleaner>),
    );
    let server = Server::new();
    let cloud = Arc::new(Cloud {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let (s2, c2) = (server.clone(), cloud.clone());
    let router = Router::new(
        c,
        Some(Arc::new(core) as Arc<dyn Backend>),
        Some(Box::new(move |_s: &ServerConfig| {
            s2.clone() as Arc<dyn Backend>
        })),
    )
    .with_cleanup_factory(Box::new(move |_o, _k| c2.clone() as Arc<dyn Cleaner>));
    router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    Rig {
        router,
        local,
        server,
        cloud,
    }
}

fn rig(c: RouterConfig) -> Rig {
    rig_with(c, true)
}

/// A fake clock for the "no clean" expiry.
#[derive(Clone)]
struct FakeClock(Arc<Mutex<Instant>>);
impl FakeClock {
    fn advance(&self, s: u64) {
        *self.0.lock().unwrap() += Duration::from_secs(s);
    }
}

/// `rig_with` on a fake clock; no local LLM.
fn clocked(c: RouterConfig) -> (Rig, FakeClock) {
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let k = rig_with(c, false);
    let c2 = clock.clone();
    let router = k.router.with_clock(Arc::new(move || *c2.0.lock().unwrap()));
    (Rig { router, ..k }, clock)
}

/// The status once the background re-probe settled.
fn settled_llm(r: &Router) -> Option<bool> {
    for _ in 0..200 {
        if let Some(v) = r.cleanup_status().server_llm {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn session() -> SessionConfig {
    SessionConfig {
        llm_skip_clean: false,
        dictionary: serde_json::from_value(serde_json::json!([
            {"word": "wispr", "replacement": "Wispr"}
        ]))
        .unwrap(),
        ..Default::default()
    }
}

fn dictate(r: &Router) -> CommitResult {
    r.start_session(session()).unwrap();
    r.set_context(FieldContext {
        before: "Hi team, ".into(),
        ..Default::default()
    });
    let pcm: Vec<i16> = (0..300 * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    assert!(r.push_audio(&pcm));
    r.commit()
}

fn has(r: &CommitResult, s: &str) -> bool {
    r.final_text.to_lowercase().contains(&s.to_lowercase())
}

use CleanupRoute::{Cloud as C, Local as L, Server as S};

// ---- config ----------------------------------------------------------------

#[test]
fn empty_order_means_local_then_server() {
    let k = rig(cfg(&[]));
    assert_eq!(k.router.cleanup_status().order, vec![L, S]);
    let c: RouterConfig = serde_json::from_str(r#"{"cleanup_order":["server","local"]}"#).unwrap();
    assert_eq!(c.cleanup_order, vec![S, L]);
    let mut dup = cfg(&[S, L, S]);
    assert!(dup.validate().is_err());
    dup.cleanup_order = vec![S, L, C];
    assert!(dup.validate().is_ok());
}

#[test]
fn empty_order_keeps_pre_d36_behaviour() {
    // On-device dictation without a local LLM: the server is not asked.
    let k = rig_with(cfg(&[]), false);
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!((k.server.cleans(), r.fallback_reason), (0, None));
    // Server dictation: the server cleans it, even with a local LLM.
    let k = rig(server_stt(&[]));
    let r = dictate(&k.router);
    assert_eq!(r.final_text, SESSION_OUT, "{r:?}");
    assert_eq!(k.server.levels(), vec![session().cleanup_level]);
    assert_eq!((k.local_calls(), k.server.cleans()), (0, 0));
}

#[test]
fn status_reports_server_llm_and_circuit() {
    let k = rig(cfg(&[L, S, C]));
    let st = k.router.cleanup_status();
    assert_eq!(st.server_llm, Some(true));
    assert!(st.server_circuit.is_some_and(|c| !c.open));
    // Not in the order: no server status.
    let k = rig(cfg(&[L, C]));
    let st = k.router.cleanup_status();
    assert_eq!((st.server_llm, st.server_circuit), (None, None));
}

// ---- client STT ------------------------------------------------------------

#[test]
fn local_first_with_a_local_llm_never_asks_the_server() {
    let k = rig(cfg(&[L, S, C]));
    let r = dictate(&k.router);
    assert!(has(&r, LOCAL_OUT), "{r:?}");
    assert_eq!(
        (k.local_calls(), k.server.cleans(), k.cloud_calls()),
        (1, 0, 0)
    );
}

#[test]
fn no_local_llm_falls_to_the_server_with_the_full_request() {
    let k = rig_with(cfg(&[L, S, C]), false);
    let r = dictate(&k.router);
    assert!(has(&r, SERVER_OUT), "{r:?}");
    assert_eq!((k.server.cleans(), k.cloud_calls()), (1, 0));
    let rt = r.route.unwrap();
    assert_eq!(rt.llm_engine.as_deref(), Some("server"));
    let req = k.server.cleans.lock().unwrap()[0].clone();
    // The user's own machine sees what the on-device LLM would.
    assert_eq!(req.ctx.before, "Hi team, ");
    assert_eq!(req.dictionary.entries.len(), 1);
    assert!(req.timeout_ms > 0 && req.timeout_ms <= session().llm_timeout_ms);
    assert!(!req.text.is_empty());
}

#[test]
fn server_first_cleans_before_the_local_llm() {
    let k = rig(cfg(&[S, L, C]));
    let r = dictate(&k.router);
    assert!(has(&r, SERVER_OUT), "{r:?}");
    assert_eq!((k.local_calls(), k.server.cleans()), (0, 1));
}

#[test]
fn local_failure_falls_back_to_the_server() {
    let k = rig(cfg(&[L, S]));
    *k.local.fail.lock().unwrap() = true;
    let r = dictate(&k.router);
    assert!(has(&r, SERVER_OUT), "{r:?}");
    assert_eq!((k.local_calls(), k.server.cleans()), (1, 1));
}

#[test]
fn server_without_llm_is_skipped_without_a_request() {
    let k = rig_with(cfg(&[S, C]), false);
    *k.server.llm.lock().unwrap() = false;
    let r = dictate(&k.router);
    assert!(has(&r, CLOUD_OUT), "{r:?}");
    assert_eq!((k.server.cleans(), k.cloud_calls()), (0, 1));
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
}

#[test]
fn server_refusal_falls_to_the_next_engine() {
    for e in ["no_llm", "busy", "llm_timeout", "llm_error: backend"] {
        let k = rig(cfg(&[S, L]));
        *k.server.answer.lock().unwrap() = Answer::Error(e);
        let r = dictate(&k.router);
        assert!(has(&r, LOCAL_OUT), "{e}: {r:?}");
        assert_eq!((k.server.cleans(), k.local_calls()), (1, 1), "{e}");
    }
}

#[test]
fn unreachable_server_opens_its_circuit() {
    let k = rig(cfg(&[S, L]));
    *k.server.answer.lock().unwrap() = Answer::Disconnected;
    assert!(has(&dictate(&k.router), LOCAL_OUT));
    let c = k.router.cleanup_status().server_circuit.unwrap();
    assert!(c.open, "{c:?}");
    // Open circuit: the next dictation does not wait for the server.
    assert!(has(&dictate(&k.router), LOCAL_OUT));
    assert_eq!((k.server.cleans(), k.local_calls()), (1, 2));
    k.router.reset_circuit();
    *k.server.answer.lock().unwrap() = Answer::Text;
    assert!(has(&dictate(&k.router), SERVER_OUT));
}

#[test]
fn unreachable_server_before_the_cloud_leaves_the_cloud_first() {
    // 2026-11-02 local-cleanup-skipped: phone AI off, order local → server
    // → cloud (the default), server unreachable. Once its circuit is open
    // the cloud is the first engine that can clean, so D30 applies: raw
    // transcript in and no "already clean" skip (which used to stop the
    // whole chain on a tidy-looking dictation).
    // Tidy already (the rules leave the LLM nothing obvious to do).
    let heard = CLOUD_OUT;
    let k = rig_heard(cfg(&[L, S, C]), false, heard);
    *k.server.answer.lock().unwrap() = Answer::Disconnected;
    let go = |k: &Rig, llm_skip_clean: bool| {
        k.router
            .start_session(SessionConfig {
                llm_skip_clean,
                ..session()
            })
            .unwrap();
        let pcm: Vec<i16> = (0..300 * 16)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        assert!(k.router.push_audio(&pcm));
        k.router.commit()
    };
    // D43: the first tidy dictation after the drop (circuit still closed,
    // the server believed reachable) is no longer skipped: the server is
    // tried, fails, and the cloud cleans it.
    let r = go(&k, true);
    assert_ne!(
        r.fallback_reason.as_deref(),
        Some("llm_skipped_clean"),
        "{r:?}"
    );
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert_eq!((k.server.cleans(), k.cloud_calls()), (1, 1), "{r:?}");
    assert!(k.router.cleanup_status().server_circuit.unwrap().open);
    let before = k.cloud_calls();
    let r = go(&k, true);
    assert_ne!(
        r.fallback_reason.as_deref(),
        Some("llm_skipped_clean"),
        "{r:?}"
    );
    assert_eq!(k.cloud_calls(), before + 1, "{r:?}");
    assert_eq!(k.server.cleans(), 1, "open circuit: not asked again");
    assert_eq!(r.cleanup_route.as_deref(), Some("cloud"), "{r:?}");
    let seen = k.cloud.seen.lock().unwrap().last().unwrap().clone();
    assert_eq!(seen.transcript, heard, "raw transcript in");
}

#[test]
fn server_not_in_the_order_is_never_asked() {
    let k = rig_with(cfg(&[L, C]), false);
    let r = dictate(&k.router);
    assert!(has(&r, CLOUD_OUT), "{r:?}");
    assert_eq!(k.server.cleans(), 0);
}

#[test]
fn local_left_out_of_the_order_is_never_used() {
    let k = rig(cfg(&[C]));
    assert!(has(&dictate(&k.router), CLOUD_OUT));
    // Nothing enabled at all: deterministic cleanup only.
    let mut c = cfg(&[C]);
    c.allow_cloud_cleanup = false;
    let k = rig(c);
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.fallback_reason, None, "switched off, not a failure");
    assert_eq!(
        (k.local_calls(), k.cloud_calls(), k.server.cleans()),
        (0, 0, 0)
    );
}

#[test]
fn cloud_first_then_server_on_cloud_refusal() {
    let mut c = cfg(&[C, S, L]);
    // No consent: the gate refuses fast.
    c.cloud.openrouter.cleanup.consent_at_ms = 0;
    let k = rig(c);
    let r = dictate(&k.router);
    assert!(has(&r, SERVER_OUT), "{r:?}");
    assert_eq!(
        (k.cloud_calls(), k.server.cleans(), k.local_calls()),
        (0, 1, 0)
    );
}

#[test]
fn command_mode_never_uses_the_server() {
    let k = rig(cfg(&[S]));
    assert!(k.router.command_cleaner(None).is_none());
}

// ---- server STT ------------------------------------------------------------

#[test]
fn server_first_is_one_pass() {
    let k = rig_with(server_stt(&[S, L, C]), false);
    let r = dictate(&k.router);
    assert_eq!(r.final_text, SESSION_OUT, "{r:?}");
    assert_eq!(k.server.levels(), vec![session().cleanup_level]);
    assert_eq!(k.server.cleans(), 0);
    // D43: the server cleaning its own session never skips tidy text.
    let k2 = rig_with(server_stt(&[S, L, C]), false);
    k2.router
        .start_session(SessionConfig {
            llm_skip_clean: true,
            ..session()
        })
        .unwrap();
    k2.router.cancel();
    assert_eq!(*k2.server.skips.lock().unwrap(), vec![false]);
    let rt = r.route.unwrap();
    assert_eq!(rt.engine, "remote");
    assert_eq!(rt.llm_engine.as_deref(), Some("server"));
}

#[test]
fn server_stt_with_local_first_cleans_on_the_client() {
    let k = rig(server_stt(&[L, S, C]));
    let r = dictate(&k.router);
    assert!(has(&r, LOCAL_OUT), "{r:?}");
    assert_eq!(
        k.server.levels(),
        vec![CleanupLevel::None],
        "server skips its LLM"
    );
    assert_eq!((k.local_calls(), k.server.cleans()), (1, 0));
    let rt = r.route.unwrap();
    assert_eq!(rt.engine, "remote");
    assert_eq!(rt.llm_engine.as_deref(), Some("local"));
}

#[test]
fn server_stt_with_cloud_first_sends_the_cloud_a_narrowed_prompt() {
    let k = rig_with(server_stt(&[C, S]), false);
    let r = dictate(&k.router);
    assert!(has(&r, CLOUD_OUT), "{r:?}");
    assert_eq!(k.server.levels(), vec![CleanupLevel::None]);
    assert_eq!(k.server.cleans(), 0);
    assert_eq!(r.route.unwrap().llm_engine.as_deref(), Some("cloud"));
    // Default privacy: surrounding text never goes to the cloud.
    let seen = k.cloud.seen.lock().unwrap();
    assert!(!seen[0].user.contains("Hi team"), "{:?}", seen[0].user);
}

#[test]
fn server_meant_to_clean_but_without_llm_hands_over_to_the_rest() {
    let k = rig_with(server_stt(&[S, C]), false);
    // hello not seen yet: the server is assumed able, but has no LLM.
    *k.server.hello_known.lock().unwrap() = false;
    *k.server.llm.lock().unwrap() = false;
    let r = dictate(&k.router);
    assert!(has(&r, CLOUD_OUT), "{r:?}");
    assert_eq!(k.server.levels(), vec![session().cleanup_level]);
    assert_eq!(
        (k.server.cleans(), k.cloud_calls()),
        (0, 1),
        "server not asked twice"
    );
}

#[test]
fn level_none_is_never_refinished() {
    let k = rig(server_stt(&[L, S]));
    k.router
        .start_session(SessionConfig {
            cleanup_level: CleanupLevel::None,
            ..session()
        })
        .unwrap();
    assert!(k.router.push_audio(&vec![1000; 4800]));
    let r = k.router.commit();
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(k.local_calls(), 0);
}

#[test]
fn failover_from_the_server_gives_the_local_engine_the_full_config() {
    let k = rig(server_stt(&[L, S]));
    *k.server.commit_fails.lock().unwrap() = true;
    let r = dictate(&k.router);
    assert!(has(&r, LOCAL_OUT), "{r:?}");
    assert_eq!(r.route.as_ref().unwrap().engine, "local");
    assert_eq!(k.local_calls(), 1);
}

#[test]
fn server_known_without_llm_counts_as_no_cleaner() {
    let k = rig_with(cfg(&[L, S]), false);
    *k.server.llm.lock().unwrap() = false;
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.fallback_reason, None);
    assert_eq!(k.server.cleans(), 0);
}

#[test]
fn switched_off_entries_keep_their_place_but_never_clean() {
    let mut c = cfg(&[S, L, C]);
    c.cleanup_off = vec![S];
    let k = rig(c.clone());
    assert_eq!(k.router.cleanup_status().order, vec![L, C]);
    assert!(has(&dictate(&k.router), LOCAL_OUT));
    assert_eq!(k.server.cleans(), 0);
    // All three off: no AI cleanup at all (not the pre-D36 empty order).
    c.cleanup_off = vec![S, L, C];
    let k = rig(server_stt(&[]));
    k.router.update_config(RouterConfig {
        cleanup_order: vec![S, L, C],
        cleanup_off: vec![S, L, C],
        ..k.router.config()
    });
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(k.server.levels(), vec![CleanupLevel::None]);
    assert_eq!(
        (k.local_calls(), k.cloud_calls(), k.server.cleans()),
        (0, 0, 0)
    );
}

// ---- a cached "no clean" expires --------------------------------------------

/// The server had no AI model, then was upgraded (and restarted): the hello
/// cached from the old connection still says "no".
fn upgraded(k: &Rig) {
    *k.server.llm.lock().unwrap() = true;
    *k.server.old_hello.lock().unwrap() = Some(false);
}

#[test]
fn no_clean_is_rechecked_after_the_health_ttl() {
    let (k, clock) = clocked(cfg(&[S, C]));
    *k.server.llm.lock().unwrap() = false;
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    upgraded(&k);
    // Within the TTL (30 s): believed, skipped, no connection.
    clock.advance(10);
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    assert!(has(&dictate(&k.router), CLOUD_OUT));
    assert_eq!((k.server.warms(), k.server.cleans()), (0, 0));
    // Past it: one background re-probe renews the hello.
    clock.advance(21);
    assert_eq!(settled_llm(&k.router), Some(true));
    assert_eq!(k.server.warms(), 1);
    assert!(has(&dictate(&k.router), SERVER_OUT));
}

#[test]
fn a_dictation_after_the_ttl_asks_the_server_again() {
    let (k, clock) = clocked(cfg(&[S, C]));
    *k.server.llm.lock().unwrap() = false;
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    upgraded(&k);
    clock.advance(31);
    assert!(has(&dictate(&k.router), SERVER_OUT));
    assert_eq!(settled_llm(&k.router), Some(true));
}

#[test]
fn still_no_clean_after_the_recheck_waits_another_ttl() {
    let (k, clock) = clocked(cfg(&[S, C]));
    *k.server.llm.lock().unwrap() = false;
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    clock.advance(31);
    assert_eq!(settled_llm(&k.router), Some(false));
    assert_eq!(k.server.warms(), 1);
    clock.advance(20);
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    assert!(has(&dictate(&k.router), CLOUD_OUT));
    assert_eq!((k.server.warms(), k.server.cleans()), (1, 0));
}

#[test]
fn the_health_ttl_setting_is_used() {
    let mut c = cfg(&[S, C]);
    c.server.health_ttl_s = 5;
    let (k, clock) = clocked(c);
    *k.server.llm.lock().unwrap() = false;
    assert_eq!(k.router.cleanup_status().server_llm, Some(false));
    upgraded(&k);
    clock.advance(6);
    assert_eq!(settled_llm(&k.router), Some(true));
}

#[test]
fn a_server_with_clean_is_never_rechecked() {
    let (k, clock) = clocked(cfg(&[S, C]));
    for _ in 0..5 {
        clock.advance(3600);
        assert_eq!(k.router.cleanup_status().server_llm, Some(true));
    }
    assert_eq!(k.server.warms(), 0);
}

// ---- D53: meeting summaries on the cleanup order ---------------------------

const SUMMARY_IN: &str = "Me: the review moves to Monday\nOthers: I will send the agenda";

fn summarise(k: &Rig) -> Result<flow_core::summary::Summary, flow_core::summary::SummaryError> {
    let base = Some(k.local.clone() as Arc<dyn Cleaner>);
    flow_core::summary::summarise(
        k.router.cleanup_cleaner(base),
        SUMMARY_IN,
        &flow_core::summary::Options::default(),
    )
}

#[test]
fn summaries_follow_the_cleanup_order_server_included() {
    let k = rig(cfg(&[S, L, C]));
    let s = summarise(&k).unwrap();
    assert_eq!((s.engine.as_str(), s.text.as_str()), ("server", SERVER_OUT));
    let req = k.server.cleans.lock().unwrap()[0].clone();
    assert_eq!(req.task, flow_core::cleanup::CleanTask::MeetingSummary);
    assert_eq!(req.text, SUMMARY_IN);
    assert_eq!((k.local_calls(), k.cloud_calls()), (0, 0));
    // Local first: the on-device model writes it.
    let k = rig(cfg(&[L, S, C]));
    assert_eq!(summarise(&k).unwrap().engine, "local");
    assert_eq!(k.server.cleans(), 0);
}

#[test]
fn an_older_server_is_skipped_for_summaries() {
    let k = rig(cfg(&[S, C]));
    *k.server.summary.lock().unwrap() = false;
    let s = summarise(&k).unwrap();
    assert_eq!(s.engine, "cloud");
    assert_eq!(k.cloud_calls(), 1);
    // Still a good server for dictations: no circuit opened.
    assert!(!k.router.cleanup_status().server_circuit.unwrap().open);
}

#[test]
fn summaries_keep_the_cloud_consent_gate() {
    let mut c = cfg(&[C]);
    c.cloud.openrouter.cleanup.consent_at_ms = 0;
    let k = rig(c);
    assert!(summarise(&k).is_err());
    assert_eq!(k.cloud_calls(), 0, "nothing sent without consent");
    // Cloud switched off in the order: never asked.
    let mut c = cfg(&[L, C]);
    c.cleanup_off = vec![C];
    let k = rig(c);
    let s = summarise(&k).unwrap();
    assert_eq!((s.engine.as_str(), k.cloud_calls()), ("local", 0));
}

#[test]
fn no_engine_on_means_no_summary() {
    let mut c = cfg(&[L, S, C]);
    c.cleanup_off = vec![L, S, C];
    let k = rig(c);
    assert!(k
        .router
        .cleanup_cleaner(Some(k.local.clone() as Arc<dyn Cleaner>))
        .is_none());
    assert_eq!(
        summarise(&k).unwrap_err(),
        flow_core::summary::SummaryError::NoEngine
    );
}
