//! Routing policy (§1.3) with fake backends: no network, no models.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::dictionary::{DictEntry, Dictionary};
use flow_router::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum StartMode {
    Ok,
    Queued,
    Fail(Failure),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum CommitMode {
    Ok,
    OkError,
    Fail(Failure),
}

struct Fake {
    kind: EngineKind,
    ready: AtomicBool,
    start: Mutex<StartMode>,
    commit: Mutex<CommitMode>,
    /// The session dies after this many samples (remote mid-session failure).
    die_after: Mutex<Option<u64>>,
    received: AtomicU64,
    starts: AtomicUsize,
    cancels: AtomicUsize,
    commits: AtomicUsize,
    contexts: Mutex<Vec<FieldContext>>,
    last_cfg: Mutex<Option<SessionConfig>>,
    active: AtomicBool,
    /// start_session takes this long (a slow connect).
    start_delay: Mutex<std::time::Duration>,
    /// commit blocks until cancel() (or 10 s).
    block_commit: AtomicBool,
}

impl Fake {
    fn new(kind: EngineKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            ready: AtomicBool::new(true),
            start: Mutex::new(StartMode::Ok),
            commit: Mutex::new(CommitMode::Ok),
            die_after: Mutex::new(None),
            received: AtomicU64::new(0),
            starts: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            contexts: Mutex::new(Vec::new()),
            last_cfg: Mutex::new(None),
            active: AtomicBool::new(false),
            start_delay: Mutex::new(std::time::Duration::ZERO),
            block_commit: AtomicBool::new(false),
        })
    }
    fn slow_start(self: Arc<Self>, ms: u64) -> Arc<Self> {
        *self.start_delay.lock().unwrap() = std::time::Duration::from_millis(ms);
        self
    }
    fn blocking_commit(self: Arc<Self>) -> Arc<Self> {
        self.block_commit.store(true, Ordering::SeqCst);
        self
    }
    fn with_start(self: Arc<Self>, m: StartMode) -> Arc<Self> {
        *self.start.lock().unwrap() = m;
        self
    }
    fn with_commit(self: Arc<Self>, m: CommitMode) -> Arc<Self> {
        *self.commit.lock().unwrap() = m;
        self
    }
    fn dying_after(self: Arc<Self>, n: u64) -> Arc<Self> {
        *self.die_after.lock().unwrap() = Some(n);
        self
    }
    fn not_ready(self: Arc<Self>) -> Arc<Self> {
        self.ready.store(false, Ordering::SeqCst);
        self
    }
    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
    fn received(&self) -> u64 {
        self.received.load(Ordering::SeqCst)
    }
    fn dead(&self) -> bool {
        self.die_after
            .lock()
            .unwrap()
            .is_some_and(|n| self.received() >= n)
    }
}

fn result(text: &str, status: &str) -> CommitResult {
    serde_json::from_value(serde_json::json!({
        "status": status, "asr_text": text, "final_text": text, "language": "en",
        "timings": {"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},
        "press_enter": false, "fallback_reason": null, "truncated": false, "error": null
    }))
    .unwrap()
}

impl Backend for Fake {
    fn kind(&self) -> EngineKind {
        self.kind
    }
    fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(*self.start_delay.lock().unwrap());
        *self.last_cfg.lock().unwrap() = Some(cfg.clone());
        match *self.start.lock().unwrap() {
            StartMode::Fail(f) => Err(BackendError::new(f, "fake start failure")),
            m => {
                self.received.store(0, Ordering::SeqCst);
                self.contexts.lock().unwrap().clear();
                self.active.store(true, Ordering::SeqCst);
                Ok(StartInfo {
                    queued: m == StartMode::Queued,
                    ..Default::default()
                })
            }
        }
    }
    fn push_audio(&self, pcm: &[i16]) -> bool {
        if !self.active.load(Ordering::SeqCst) || self.dead() {
            return false;
        }
        self.received.fetch_add(pcm.len() as u64, Ordering::SeqCst);
        !self.dead()
    }
    fn set_context(&self, ctx: &FieldContext) {
        self.contexts.lock().unwrap().push(ctx.clone());
    }
    fn partial(&self) -> Option<String> {
        Some(format!("{} partial", self.kind.as_str()))
    }
    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        if self.block_commit.load(Ordering::SeqCst) {
            let t0 = Instant::now();
            let c0 = self.cancels.load(Ordering::SeqCst);
            while self.cancels.load(Ordering::SeqCst) == c0
                && t0.elapsed() < std::time::Duration::from_secs(10)
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            return Err(BackendError::new(Failure::Error, "cancelled"));
        }
        self.active.store(false, Ordering::SeqCst);
        assert!(deadline > Instant::now());
        if self.dead() {
            return Err(BackendError::new(Failure::Disconnected, "fake died"));
        }
        let app = self
            .contexts
            .lock()
            .unwrap()
            .last()
            .map(|c| c.app_id.clone())
            .unwrap_or_default();
        let text = format!("{} {} {}", self.kind.as_str(), self.received(), app);
        match *self.commit.lock().unwrap() {
            CommitMode::Ok => {
                let mut r = result(&text, "RAW_TRANSCRIPT");
                if self.kind == EngineKind::Remote {
                    r.route = Some(RouteInfo {
                        engine: "remote".into(),
                        rtt_ms: Some(4),
                        queue_ms: Some(0),
                        server_core: Some("0.1.0".into()),
                        ..Default::default()
                    });
                }
                Ok(r)
            }
            CommitMode::OkError => Ok(result("", "ERROR")),
            CommitMode::Fail(f) => Err(BackendError::new(f, "fake commit failure")),
        }
    }
    fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        self.active.store(false, Ordering::SeqCst);
    }
    fn probe(&self) -> HealthReport {
        HealthReport {
            ok: true,
            engine: self.kind,
            ..Default::default()
        }
    }
    fn session_failure(&self) -> Option<BackendError> {
        self.dead()
            .then(|| BackendError::new(Failure::Disconnected, "fake died"))
    }
}

struct Rig {
    router: Router,
    local: Arc<Fake>,
    remote: Arc<Fake>,
    factory_calls: Arc<AtomicUsize>,
}

fn cfg(server: bool, client: bool) -> RouterConfig {
    RouterConfig {
        allow_server: server,
        allow_client: client,
        server: ServerConfig {
            host: "100.64.0.10".into(),
            token: "0123456789abcdef".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn rig_with(c: RouterConfig, local: Arc<Fake>, remote: Arc<Fake>) -> Rig {
    let calls = Arc::new(AtomicUsize::new(0));
    let (r2, c2) = (remote.clone(), calls.clone());
    let factory: RemoteFactory = Box::new(move |_s: &ServerConfig| {
        c2.fetch_add(1, Ordering::SeqCst);
        r2.clone() as Arc<dyn Backend>
    });
    Rig {
        router: Router::new(c, Some(local.clone() as Arc<dyn Backend>), Some(factory)),
        local,
        remote,
        factory_calls: calls,
    }
}

fn rig(server: bool, client: bool) -> Rig {
    rig_with(
        cfg(server, client),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote),
    )
}

const N: usize = 16_000;

/// Push 1 s of audio in 100 ms frames + a context; commit.
fn dictate(r: &Router) -> CommitResult {
    for _ in 0..10 {
        assert!(r.push_audio(&[100i16; N / 10]));
    }
    r.set_context(FieldContext {
        app_id: "com.slack".into(),
        ..Default::default()
    });
    r.commit()
}

fn route(r: &CommitResult) -> RouteInfo {
    r.route.clone().expect("route set")
}

#[test]
fn on_on_up_uses_remote() {
    let k = rig(true, true);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "remote 16000 com.slack");
    let rt = route(&r);
    assert_eq!(rt.engine, "remote");
    assert_eq!(rt.fallback, None);
    assert_eq!(rt.server.as_deref(), Some("100.64.0.10:7313"));
    assert_eq!(rt.rtt_ms, Some(4), "backend route info is merged");
    assert_eq!(k.local.starts(), 0);
}

#[test]
fn on_on_connect_failure_falls_back_to_local_from_scratch() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "local 16000 com.slack");
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback.as_deref(), Some("connect_failed"));
}

#[test]
fn connect_failure_opens_the_circuit_for_the_backoff() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).fallback.as_deref(), Some("circuit_open"));
    assert_eq!(
        k.remote.starts(),
        1,
        "second session must not try the server"
    );
    let st = k.router.route_status();
    assert!(st.circuit.open);
    assert_eq!(st.circuit.reason.as_deref(), Some("connect_failed"));
    assert!(st.circuit.retry_in_ms.unwrap() <= 5000);
    assert!(!st.circuit.until_config_change);
    assert_eq!(st.last.unwrap().fallback.as_deref(), Some("circuit_open"));
    k.router.reset_circuit();
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.remote.starts(), 2);
}

#[test]
fn auth_failure_is_not_retried_until_config_changes() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::AuthFailed)),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("auth_failed")
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("circuit_open")
    );
    assert_eq!(k.remote.starts(), 1);
    assert!(k.router.route_status().circuit.until_config_change);
    // Same config again: still open.
    k.router.update_config(cfg(true, true));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.remote.starts(), 1);
    // New token: retried with a freshly built remote backend.
    let mut c = cfg(true, true);
    c.server.token = "another-token-0123456".into();
    k.router.update_config(c);
    *k.remote.start.lock().unwrap() = StartMode::Ok;
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.remote.starts(), 2);
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 2);
    assert_eq!(route(&dictate(&k.router)).engine, "remote");
}

#[test]
fn queued_start_cancels_remote_and_goes_local_busy() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Queued),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback.as_deref(), Some("busy"));
    assert_eq!(k.remote.cancels.load(Ordering::SeqCst), 1);
    // Busy does not open the circuit: the server is up.
    assert!(!k.router.route_status().circuit.open);
}

#[test]
fn queued_start_without_fallback_waits_for_the_server() {
    let k = rig_with(
        cfg(true, false),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Queued),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "remote");
}

#[test]
fn mid_session_death_replays_the_whole_buffer_into_local() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).dying_after(8000),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    k.router.set_context(FieldContext {
        app_id: "early".into(),
        ..Default::default()
    });
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "local 16000 com.slack", "no sample lost");
    assert_eq!(route(&r).fallback.as_deref(), Some("disconnected"));
    assert!(k.remote.cancels.load(Ordering::SeqCst) >= 1);
    let ctxs = k.local.contexts.lock().unwrap();
    assert_eq!(ctxs[0].app_id, "early", "last context replayed at switch");
    // The local ring was enlarged for the replay.
    let lc = k.local.last_cfg.lock().unwrap().clone().unwrap();
    assert!(lc.ring_capacity_samples >= SessionConfig::default().ring_capacity_samples);
}

#[test]
fn commit_timeout_replays_into_local() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_commit(CommitMode::Fail(Failure::Timeout)),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "local 16000 com.slack");
    assert_eq!(route(&r).fallback.as_deref(), Some("timeout"));
    assert_eq!(k.remote.cancels.load(Ordering::SeqCst), 1);
    assert!(k.router.route_status().circuit.open);
}

#[test]
fn a_remote_error_result_is_final() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_commit(CommitMode::OkError),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(route(&r).engine, "remote");
    assert_eq!(k.local.starts(), 0, "never two answers for one utterance");
}

#[test]
fn on_off_server_down_is_server_unavailable() {
    let k = rig_with(
        cfg(true, false),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e.kind(), "server_unavailable");
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.clone().unwrap().starts_with("server_unavailable"));
    assert_eq!(route(&r).fallback.as_deref(), Some("connect_failed"));
    assert_eq!(k.local.starts(), 0);
    // Without a fallback a transient failure is retried at once.
    let _ = k.router.start_session(SessionConfig::default());
    assert_eq!(k.remote.starts(), 2);
}

#[test]
fn on_off_mid_session_death_is_error() {
    let k = rig_with(
        cfg(true, false),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).dying_after(8000),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(route(&r).fallback.as_deref(), Some("disconnected"));
    assert_eq!(k.local.starts(), 0);
}

#[test]
fn off_on_never_builds_a_remote_backend() {
    let k = rig(false, true);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback, None);
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 0);
    assert_eq!(k.local.received(), 16000);
}

#[test]
fn local_only_build_without_factory() {
    let local = Fake::new(EngineKind::Local);
    let r = Router::new(
        cfg(true, true),
        Some(local.clone() as Arc<dyn Backend>),
        None,
    );
    assert!(!r.capabilities().remote);
    assert!(r.capabilities().local);
    r.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&r)).engine, "local");
    let h = r.probe_server();
    assert!(!h.ok);
}

#[test]
fn off_off_is_no_engine_enabled() {
    let k = rig(false, false);
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e, RouterError::NoEngineEnabled);
    assert_eq!(e.kind(), "no_engine_enabled");
    let r = k.router.commit();
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.clone().unwrap().starts_with("no_engine_enabled"));
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 0);
    assert_eq!(k.local.starts(), 0);
}

#[test]
fn prefer_client_uses_local_and_remote_only_when_local_not_ready() {
    let mut c = cfg(true, true);
    c.prefer = Prefer::Client;
    let k = rig_with(
        c.clone(),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 0);

    let k = rig_with(
        c,
        Fake::new(EngineKind::Local).not_ready(),
        Fake::new(EngineKind::Remote),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "remote");
    assert_eq!(route(&r).fallback.as_deref(), Some("local_not_ready"));
}

#[test]
fn empty_host_is_server_not_configured() {
    let mut c = cfg(true, true);
    c.server.host = " ".into();
    let k = rig_with(
        c.clone(),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback.as_deref(), Some("server_not_configured"));
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 0);

    c.allow_client = false;
    k.router.update_config(c);
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e.kind(), "server_not_configured");
}

#[test]
fn local_not_ready_at_fallback_time_is_an_error() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local).not_ready(),
        Fake::new(EngineKind::Remote).dying_after(8000),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.clone().unwrap().contains("local model not ready"));

    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local).not_ready(),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e.kind(), "server_unavailable");
    assert!(e.to_string().contains("local model not ready"));
}

#[test]
fn update_config_applies_from_the_next_session() {
    let k = rig(true, true);
    k.router.start_session(SessionConfig::default()).unwrap();
    k.router.push_audio(&[1; 1600]);
    k.router.update_config(cfg(false, true));
    let r = dictate(&k.router);
    assert_eq!(
        route(&r).engine,
        "remote",
        "current session keeps its route"
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
}

#[test]
fn set_local_replaces_the_fallback_engine() {
    let k = rig(false, true);
    let other = Fake::new(EngineKind::Local);
    k.router.set_local(Some(other.clone() as Arc<dyn Backend>));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert_eq!(other.starts(), 1);
    assert_eq!(k.local.starts(), 0);
    k.router.set_local(None);
    assert_eq!(
        k.router
            .start_session(SessionConfig::default())
            .unwrap_err(),
        RouterError::LocalNotReady
    );
}

#[test]
fn session_api_edges() {
    let k = rig(true, true);
    assert!(!k.router.push_audio(&[1; 10]), "no session");
    assert_eq!(k.router.commit().status, CommitStatus::Error);
    assert!(k.router.commit().route.is_none());
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.router.partial().as_deref(), Some("remote partial"));
    k.router.cancel();
    assert_eq!(k.remote.cancels.load(Ordering::SeqCst), 1);
    assert_eq!(
        k.router.commit().status,
        CommitStatus::Error,
        "cancel → commit = ERROR"
    );
    // A new start replaces (cancels) an open session.
    k.router.start_session(SessionConfig::default()).unwrap();
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.remote.cancels.load(Ordering::SeqCst), 2);
}

#[test]
fn buffer_is_capped_at_max_samples() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).dying_after(20_000),
    );
    k.router
        .start_session(SessionConfig {
            max_samples: 12_000,
            ..Default::default()
        })
        .unwrap();
    for _ in 0..30 {
        k.router.push_audio(&[1; 1000]);
    }
    k.router.commit();
    // 12 000 replayed + the 10 000 live samples after the switch at 20 000.
    assert_eq!(k.local.received(), 12_000 + 10_000);
}

#[test]
fn learn_edit_uses_the_last_session_dictionary() {
    let k = rig(false, true);
    let dict = Dictionary {
        entries: vec![DictEntry::word("Wispr")],
    };
    k.router
        .start_session(SessionConfig {
            dictionary: dict.clone(),
            ..Default::default()
        })
        .unwrap();
    dictate(&k.router);
    let pasted = "I use whisper flow and kubernetes daily";
    let edited = "I use Wispr Flow and Kubernetes daily";
    assert_eq!(
        k.router.learn_edit(pasted, edited),
        flow_core::learn::learn_edit(pasted, edited, &dict)
    );
}

#[test]
fn capabilities_reflect_build_and_hints() {
    let k = rig(true, true);
    let c = k.router.capabilities();
    assert!(c.local && c.remote);
    assert_eq!(c.proto, 1);
    assert_eq!(c.core_version, flow_core::VERSION);
    let r = Router::new(RouterConfig::default(), None, None)
        .with_local_capability(true)
        .with_local_info(vec!["whisper".into(), "parakeet".into()], true);
    let c = r.capabilities();
    assert!(c.local && !c.remote && c.llm);
    assert_eq!(c.asr_engines, ["whisper", "parakeet"]);
    // Capable but nothing loaded yet: local_not_ready, not no_engine_enabled.
    let cfg = RouterConfig {
        allow_server: false,
        ..Default::default()
    };
    r.update_config(cfg);
    assert_eq!(
        r.start_session(SessionConfig::default()).unwrap_err(),
        RouterError::LocalNotReady
    );
}

#[test]
fn probe_server_caches_health_and_closes_the_circuit() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert!(k.router.route_status().circuit.open);
    let h = k.router.probe_server();
    assert!(h.ok);
    assert_eq!(h.server.as_deref(), Some("100.64.0.10:7313"));
    let st = k.router.route_status();
    assert!(!st.circuit.open);
    assert!(st.health.unwrap().ok);
}

#[test]
fn config_validation() {
    let mut c = cfg(true, true);
    assert!(c.validate().is_ok());
    c.server.host = "8.8.8.8".into();
    assert_eq!(c.validate().unwrap_err().kind(), "bad_config");
    c.server.tls = true;
    assert!(c.validate().is_ok());
    c.server.token = "short".into();
    assert!(c.validate().is_err());
    c.server.token = String::new();
    c.server.tls_pin_sha256 = Some("ab:".repeat(32));
    assert!(c.validate().is_ok());
    c.server.tls_pin_sha256 = Some("xyz".into());
    assert!(c.validate().is_err());
    c.server.tls_pin_sha256 = None;
    c.server.connect_timeout_ms = 50;
    assert!(c.validate().is_err());
    assert!(
        RouterConfig::default().validate().is_ok(),
        "empty host is valid"
    );
    let s = ServerConfig {
        host: "[fd7a:115c:a1e0::1]".into(),
        ..Default::default()
    };
    assert_eq!(s.addr(), "[fd7a:115c:a1e0::1]:7313");
}

#[test]
fn router_is_send_and_sync() {
    fn f<T: Send + Sync>() {}
    f::<Router>();
}

#[test]
fn route_order_follows_prefer_and_toggles() {
    use EngineKind::*;
    let mut c = cfg(true, true);
    assert_eq!(c.route_order(), [Remote, Local]);
    c.prefer = Prefer::Client;
    assert_eq!(c.route_order(), [Local, Remote]);
    assert_eq!(cfg(true, false).route_order(), [Remote]);
    assert_eq!(cfg(false, true).route_order(), [Local]);
    assert!(cfg(false, false).route_order().is_empty());
}

// ---- live session route (start info + poll) ---------------------------------

fn live(r: &Router) -> SessionRoute {
    r.poll().route.expect("live session reports its route")
}

/// The route at start equals the committed route when no fallback happened.
fn assert_start_matches_commit(start: &SessionRoute, r: &CommitResult) {
    assert_eq!(*start, SessionRoute::from_route_info(&route(r)), "{r:?}");
}

#[test]
fn start_info_and_poll_report_the_server_route() {
    let k = rig(true, true);
    let info = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap();
    let start = info.route.clone().expect("start route");
    assert_eq!(start.engine, EngineKind::Remote);
    assert_eq!(start.server.as_deref(), Some("100.64.0.10:7313"));
    assert_eq!(start.fallback, None);
    assert_eq!(start.provider, None);
    assert!(!info.queued);
    assert_eq!(live(&k.router), start);
    assert_eq!(k.router.session_route(), Some(start.clone()));
    let p = k.router.poll();
    assert_eq!(
        p.partial.as_deref(),
        Some("remote partial"),
        "SessionPoll fields kept"
    );
    let r = dictate(&k.router);
    assert_start_matches_commit(&start, &r);
    assert_eq!(k.router.poll().route, None, "no session after commit");
    assert_eq!(k.router.session_route(), None);
}

#[test]
fn start_info_and_poll_report_the_local_route() {
    let k = rig(false, true);
    let start = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap()
        .route
        .unwrap();
    assert_eq!(start.engine, EngineKind::Local);
    assert_eq!((start.server.clone(), start.fallback.clone()), (None, None));
    assert_eq!(live(&k.router), start);
    assert_start_matches_commit(&start, &dictate(&k.router));
}

#[test]
fn start_route_names_the_fallback_when_the_server_is_down_at_start() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    let start = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap()
        .route
        .unwrap();
    assert_eq!(start.engine, EngineKind::Local);
    assert_eq!(start.fallback.as_deref(), Some("connect_failed"));
    assert_eq!(start.server, None, "server only named while it serves");
    // A start-time fallback is already in the start route: still equal.
    assert_start_matches_commit(&start, &dictate(&k.router));
}

#[test]
fn poll_switches_to_local_after_a_mid_session_server_death() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).dying_after(8000),
    );
    let start = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap()
        .route
        .unwrap();
    assert_eq!(start.engine, EngineKind::Remote);
    assert!(k.router.push_audio(&[100i16; 4000]));
    assert_eq!(live(&k.router).engine, EngineKind::Remote);
    assert!(k.router.push_audio(&[100i16; 4000]));
    let now = live(&k.router);
    assert_eq!(now.engine, EngineKind::Local, "pill must change live");
    assert_eq!(now.fallback.as_deref(), Some("disconnected"));
    assert_eq!(now.server, None);
    // The failover connects off the push thread: partials resume once the
    // local engine is installed.
    let t0 = Instant::now();
    while k.router.poll().partial.is_none() && t0.elapsed().as_secs() < 2 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(k.router.poll().partial.as_deref(), Some("local partial"));
    assert_eq!(live(&k.router), now, "installed route = announced route");
    let r = k.router.commit();
    assert_eq!(route(&r).engine, "local");
    assert_ne!(
        start,
        SessionRoute::from_route_info(&route(&r)),
        "fallback happened"
    );
    assert_eq!(
        now,
        SessionRoute::from_route_info(&route(&r)),
        "last poll = commit"
    );
}

#[test]
fn failed_start_reports_no_live_route() {
    let k = rig_with(
        cfg(true, false),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Fail(Failure::ConnectFailed)),
    );
    assert!(k
        .router
        .start_session_info(SessionConfig::default())
        .is_err());
    assert_eq!(k.router.poll(), RouterPoll::default());
    assert_eq!(k.router.session_route(), None);
}

#[test]
fn queued_start_without_fallback_reports_queued() {
    let k = rig_with(
        cfg(true, false),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_start(StartMode::Queued),
    );
    let info = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap();
    assert!(info.queued);
    assert_eq!(info.route.unwrap().engine, EngineKind::Remote);
}

#[test]
fn router_poll_json_is_session_poll_plus_optional_route() {
    // No session: byte-identical to the core SessionPoll JSON.
    let idle = serde_json::to_string(&RouterPoll::default()).unwrap();
    assert_eq!(
        idle,
        serde_json::to_string(&flow_core::SessionPoll::default()).unwrap()
    );
    let p: RouterPoll = serde_json::from_str(
        r#"{"partial":"hi","endpoint_reached":true,"speech_ms":5,"trailing_silence_ms":7,
            "route":{"engine":"remote","server":"h:1"}}"#,
    )
    .unwrap();
    assert!(p.endpoint_reached);
    assert_eq!(p.route.as_ref().unwrap().engine, EngineKind::Remote);
    let core: flow_core::SessionPoll = p.clone().into();
    assert_eq!(core.speech_ms, 5);
    // StartInfo: `route` omitted while None.
    assert_eq!(
        serde_json::to_string(&StartInfo::default()).unwrap(),
        r#"{"queued":false}"#
    );
}

// ---- C3: a mid-session failover never blocks the audio thread ---------------

#[test]
fn failover_connect_runs_off_the_push_thread() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local).slow_start(400),
        Fake::new(EngineKind::Remote).dying_after(8000),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let mut worst = std::time::Duration::ZERO;
    for _ in 0..10 {
        let t = Instant::now();
        assert!(k.router.push_audio(&[100i16; N / 10]));
        worst = worst.max(t.elapsed());
    }
    assert!(
        worst < std::time::Duration::from_millis(100),
        "push blocked {worst:?} on the failover connect"
    );
    // Pill already names where the session is going.
    assert_eq!(k.router.poll().route.unwrap().engine, EngineKind::Local);
    let r = k.router.commit();
    assert_eq!(
        r.final_text, "local 16000 ",
        "all audio replayed + live tail"
    );
    assert_eq!(route(&r).fallback.as_deref(), Some("disconnected"));
}

#[test]
fn audio_pushed_while_switching_reaches_the_new_engine() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local).slow_start(150),
        Fake::new(EngineKind::Remote).dying_after(4000),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    for _ in 0..10 {
        assert!(k.router.push_audio(&[100i16; N / 10]));
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    let r = k.router.commit();
    assert_eq!(r.final_text, "local 16000 ");
    assert_eq!(k.local.starts(), 1);
}

// ---- C4: cancel during commit aborts it ----------------------------------------

fn cancel_while_committing(k: &Rig) -> (CommitResult, std::time::Duration) {
    for _ in 0..10 {
        k.router.push_audio(&[100i16; N / 10]);
    }
    std::thread::scope(|sc| {
        let h = sc.spawn(|| {
            let t = Instant::now();
            (k.router.commit(), t.elapsed())
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        k.router.cancel();
        h.join().unwrap()
    })
}

#[test]
fn cancel_during_a_remote_commit_aborts_without_fallback() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).blocking_commit(),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let (r, took) = cancel_while_committing(&k);
    assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r.error.as_deref().unwrap().contains("cancelled"), "{r:?}");
    assert!(k.remote.cancels.load(Ordering::SeqCst) >= 1, "server told");
    assert_eq!(
        k.local.starts(),
        0,
        "a cancelled dictation never falls back"
    );
}

#[test]
fn cancel_during_a_local_commit_aborts() {
    let k = rig_with(
        cfg(false, true),
        Fake::new(EngineKind::Local).blocking_commit(),
        Fake::new(EngineKind::Remote),
    );
    k.router.start_session(SessionConfig::default()).unwrap();
    let (r, took) = cancel_while_committing(&k);
    assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    assert_eq!(r.status, CommitStatus::Error);
    assert!(k.local.cancels.load(Ordering::SeqCst) >= 1);
}

#[test]
fn replay_past_max_samples_reports_truncated() {
    let k = rig_with(
        cfg(true, true),
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote).with_commit(CommitMode::Fail(Failure::Timeout)),
    );
    k.router
        .start_session(SessionConfig {
            max_samples: 12_000,
            ..Default::default()
        })
        .unwrap();
    for _ in 0..20 {
        k.router.push_audio(&[1; 1000]);
    }
    let r = k.router.commit();
    assert_eq!(route(&r).engine, "local");
    assert_eq!(k.local.received(), 12_000, "replay is capped");
    assert!(r.truncated, "audio past max_samples was lost: {r:?}");
}

#[test]
fn concurrent_starts_cancel_the_session_they_replace() {
    // C7: the second start must not orphan the first session.
    let k = rig_with(
        cfg(false, true),
        Fake::new(EngineKind::Local).slow_start(300),
        Fake::new(EngineKind::Remote),
    );
    std::thread::scope(|sc| {
        let a = sc.spawn(|| k.router.start_session(SessionConfig::default()));
        // B gets past its own "cancel the open session" step while A is
        // still starting, then waits for A.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let b = sc.spawn(|| k.router.start_session(SessionConfig::default()));
        a.join().unwrap().unwrap();
        b.join().unwrap().unwrap();
    });
    assert_eq!(k.local.starts(), 2);
    assert_eq!(
        k.local.cancels.load(Ordering::SeqCst),
        1,
        "the replaced session is cancelled"
    );
    k.router.cancel();
}
