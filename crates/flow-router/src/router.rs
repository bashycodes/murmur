//! The router: routing policy (§1.3), PCM buffer + replay on fallback,
//! circuit breaker, route bookkeeping.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use flow_core::dictionary::Dictionary;
use flow_core::learn::Candidate;
use flow_core::{CommitResult, CommitStatus, RouteInfo};

use crate::{
    now_unix_ms, Backend, BackendError, Capabilities, CircuitStatus, EngineKind, Failure,
    HealthReport, Prefer, RemoteFactory, RouteStatus, RouterConfig, RouterError, ServerConfig,
};

const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Headroom added to the local ring when a fallback replays the buffer, so
/// the replay plus live audio never overflows it.
const REPLAY_HEADROOM: usize = SAMPLE_RATE * 30;

struct Circuit {
    open_until: Option<Instant>,
    backoff: Duration,
    reason: Option<Failure>,
    /// Auth/proto failures: open until the server config changes.
    sticky: bool,
}

impl Default for Circuit {
    fn default() -> Self {
        Self {
            open_until: None,
            backoff: BACKOFF_MIN,
            reason: None,
            sticky: false,
        }
    }
}

impl Circuit {
    fn is_open(&self, now: Instant) -> bool {
        self.sticky || self.open_until.is_some_and(|t| t > now)
    }

    fn record_failure(&mut self, kind: Failure, now: Instant) {
        match kind {
            Failure::AuthFailed | Failure::ProtoMismatch => {
                self.sticky = true;
                self.reason = Some(kind);
            }
            Failure::ConnectFailed | Failure::Timeout | Failure::Disconnected => {
                self.open_until = Some(now + self.backoff);
                self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                self.reason = Some(kind);
            }
            // The server is up (busy) or there is nothing to retry.
            _ => {}
        }
    }

    fn status(&self, now: Instant) -> CircuitStatus {
        let open = self.is_open(now);
        CircuitStatus {
            open,
            reason: open
                .then(|| self.reason.map(|r| r.as_str().to_string()))
                .flatten(),
            retry_in_ms: match (open, self.sticky, self.open_until) {
                (true, false, Some(t)) => Some(t.saturating_duration_since(now).as_millis() as u64),
                _ => None,
            },
            until_config_change: open && self.sticky,
        }
    }
}

struct State {
    cfg: RouterConfig,
    local: Option<Arc<dyn Backend>>,
    local_capable: bool,
    asr_engines: Vec<String>,
    llm: bool,
    /// Remote backend built for this server config (keeps its warm WS).
    remote: Option<(ServerConfig, Arc<dyn Backend>)>,
    circuit: Circuit,
    last_route: Option<RouteInfo>,
    health: Option<HealthReport>,
    /// Dictionary of the last session (`learn_edit`).
    last_dictionary: Dictionary,
}

struct Active {
    cfg: SessionConfig,
    /// Backend currently fed with audio; `None` = the session is dead.
    engine: Option<Arc<dyn Backend>>,
    kind: EngineKind,
    /// Local backend to switch to if the remote primary fails.
    fallback: Option<Arc<dyn Backend>>,
    /// Session PCM (only while a fallback exists), capped at `max_samples`.
    buffer: Option<Vec<i16>>,
    samples: u64,
    last_ctx: Option<FieldContext>,
    route: RouteInfo,
    /// Error the commit reports for a dead session.
    dead: Option<String>,
}

impl Active {
    fn remote_primary(&self) -> bool {
        self.kind == EngineKind::Remote && self.engine.is_some()
    }
}

/// The §9 session API over local and remote backends. Thread-safe; one
/// active session at a time (like `FlowCore`).
pub struct Router {
    state: Mutex<State>,
    session: Mutex<Option<Active>>,
    remote_factory: Option<RemoteFactory>,
}

fn error_result(msg: impl Into<String>, route: Option<RouteInfo>) -> CommitResult {
    let mut r: CommitResult = serde_json::from_str(
        r#"{"status":"ERROR","asr_text":"","final_text":"","language":null,"timings":{"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},"press_enter":false,"fallback_reason":null,"truncated":false,"error":null}"#,
    )
    .expect("static CommitResult JSON");
    r.status = CommitStatus::Error;
    r.error = Some(msg.into());
    r.route = route;
    r
}

impl Router {
    pub fn new(
        cfg: RouterConfig,
        local: Option<Arc<dyn Backend>>,
        remote: Option<RemoteFactory>,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                cfg,
                local_capable: local.is_some(),
                local,
                asr_engines: Vec::new(),
                llm: false,
                remote: None,
                circuit: Circuit::default(),
                last_route: None,
                health: None,
                last_dictionary: Dictionary::default(),
            }),
            session: Mutex::new(None),
            remote_factory: remote,
        }
    }

    /// Declare that this build has a local engine even before its models are
    /// loaded (`capabilities().local`, the `allow_client` gate). Default:
    /// whether a local backend was given to `new` / `set_local`. Additive.
    pub fn with_local_capability(self, yes: bool) -> Self {
        self.state.lock().unwrap().local_capable = yes;
        self
    }

    /// Local ASR engines / LLM for `capabilities()`. Additive.
    pub fn with_local_info(self, asr_engines: Vec<String>, llm: bool) -> Self {
        {
            let mut st = self.state.lock().unwrap();
            st.asr_engines = asr_engines;
            st.llm = llm;
        }
        self
    }

    /// Takes effect at the next `start_session`. A changed server config
    /// drops the cached remote backend and resets the circuit.
    pub fn update_config(&self, cfg: RouterConfig) {
        let mut st = self.state.lock().unwrap();
        if st.cfg.server != cfg.server {
            st.remote = None;
            st.circuit = Circuit::default();
            st.health = None;
        }
        st.cfg = cfg;
    }

    /// The shell (re)loaded (or unloaded) its local models.
    pub fn set_local(&self, local: Option<Arc<dyn Backend>>) {
        let mut st = self.state.lock().unwrap();
        if local.is_some() {
            st.local_capable = true;
        }
        st.local = local;
    }

    pub fn config(&self) -> RouterConfig {
        self.state.lock().unwrap().cfg.clone()
    }

    pub fn capabilities(&self) -> Capabilities {
        let st = self.state.lock().unwrap();
        Capabilities {
            local: st.local_capable,
            remote: self.remote_factory.is_some(),
            asr_engines: st.asr_engines.clone(),
            llm: st.llm,
            proto: flow_protocol::PROTO_MAX,
            core_version: flow_core::VERSION.into(),
        }
    }

    // ---- §9 mirror -------------------------------------------------------

    pub fn start_session(&self, cfg: SessionConfig) -> Result<(), RouterError> {
        self.cancel();
        let mut st = self.state.lock().unwrap();
        st.last_dictionary = cfg.dictionary.clone();
        let server_eff = self.remote_factory.is_some() && st.cfg.allow_server;
        let client_eff = st.local_capable && st.cfg.allow_client;
        let local_ready = st.local.clone().filter(|l| l.ready());
        let (active, res) = match (server_eff, client_eff) {
            (false, false) => (
                Active::dead(cfg, RouterError::NoEngineEnabled.to_string(), None),
                Err(RouterError::NoEngineEnabled),
            ),
            (false, true) => match local_ready {
                Some(l) => Self::start_local(cfg, l, None),
                None => (
                    Active::dead(cfg, RouterError::LocalNotReady.to_string(), None),
                    Err(RouterError::LocalNotReady),
                ),
            },
            (true, false) => self.start_remote(&mut st, cfg, None, None),
            (true, true) => match (st.cfg.prefer, local_ready) {
                (Prefer::Client, Some(l)) => Self::start_local(cfg, l, None),
                (Prefer::Client, None) => {
                    self.start_remote(&mut st, cfg, None, Some(Failure::LocalNotReady))
                }
                (Prefer::Server, l) => {
                    if let Some(l) = l.filter(|_| st.circuit.is_open(Instant::now())) {
                        let mut route = local_route(Some(Failure::CircuitOpen));
                        route.server = Some(st.cfg.server.addr());
                        Self::start_local(cfg, l, Some(route))
                    } else {
                        // Remote first; a fallback is wanted even if local is
                        // not ready yet (it may be by the time it is needed).
                        let fb = st.local.clone();
                        self.start_remote(&mut st, cfg, fb, None)
                    }
                }
            },
        };
        drop(st);
        *self.session.lock().unwrap() = Some(active);
        res
    }

    pub fn push_audio(&self, pcm: &[i16]) -> bool {
        let mut g = self.session.lock().unwrap();
        let Some(a) = g.as_mut() else { return false };
        if a.engine.is_none() {
            return true;
        }
        a.samples += pcm.len() as u64;
        if let Some(buf) = a.buffer.as_mut() {
            let room = a.cfg.max_samples.saturating_sub(buf.len());
            buf.extend_from_slice(&pcm[..pcm.len().min(room)]);
        }
        let engine = a.engine.clone().unwrap();
        if !a.remote_primary() {
            engine.push_audio(pcm);
            return true;
        }
        let ok = engine.push_audio(pcm);
        let failure = a.relevant_failure(engine.session_failure()).or_else(|| {
            (!ok).then(|| BackendError::new(Failure::Disconnected, "audio not accepted"))
        });
        if let Some(e) = failure {
            let e = normalize_mid_session(e);
            self.note_remote_failure(e.kind);
            a.switch_to_local(e);
        }
        true
    }

    pub fn set_context(&self, ctx: FieldContext) {
        let mut g = self.session.lock().unwrap();
        let Some(a) = g.as_mut() else { return };
        if let Some(e) = &a.engine {
            e.set_context(&ctx);
        }
        a.last_ctx = Some(ctx);
    }

    pub fn partial(&self) -> Option<String> {
        let g = self.session.lock().unwrap();
        g.as_ref()?.engine.as_ref()?.partial()
    }

    /// Never fails: errors come back as an `ERROR` result. `route` is set
    /// whenever a session was started.
    pub fn commit(&self) -> CommitResult {
        let Some(mut a) = self.session.lock().unwrap().take() else {
            return error_result("no active session", None);
        };
        let r = self.commit_active(&mut a);
        self.state.lock().unwrap().last_route = r.route.clone();
        r
    }

    pub fn cancel(&self) {
        let a = self.session.lock().unwrap().take();
        if let Some(e) = a.and_then(|a| a.engine) {
            e.cancel();
        }
    }

    /// Always local and model-free: same answer in every build.
    pub fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<Candidate> {
        let st = self.state.lock().unwrap();
        flow_core::learn::learn_edit(pasted, edited, &st.last_dictionary)
    }

    /// Blocking health probe of the configured server (Developer "Test
    /// connection"). Never panics; failures come back in the report.
    pub fn probe_server(&self) -> HealthReport {
        let (server, backend) = {
            let mut st = self.state.lock().unwrap();
            let server = st.cfg.server.clone();
            if self.remote_factory.is_none() {
                return HealthReport::failed(
                    EngineKind::Remote,
                    &BackendError::new(
                        Failure::ServerNotConfigured,
                        "this build has no server support",
                    ),
                );
            }
            if !server.is_configured() {
                return HealthReport::failed(
                    EngineKind::Remote,
                    &BackendError::new(Failure::ServerNotConfigured, "no server host set"),
                );
            }
            (server, self.remote_for(&mut st))
        };
        let mut h = backend.probe();
        h.server.get_or_insert_with(|| server.addr());
        let mut st = self.state.lock().unwrap();
        if st.cfg.server == server {
            if h.ok {
                st.circuit = Circuit::default();
            } else if let Some(k) = h.error.as_deref().and_then(failure_from_str) {
                st.circuit.record_failure(k, Instant::now());
            }
            st.health = Some(h.clone());
        }
        h
    }

    /// Blocking probe of an arbitrary (unsaved) server config, e.g. the
    /// Developer form before Save. Does not touch router state. Additive.
    pub fn probe_config(&self, server: &ServerConfig) -> HealthReport {
        match &self.remote_factory {
            None => HealthReport::failed(
                EngineKind::Remote,
                &BackendError::new(
                    Failure::ServerNotConfigured,
                    "this build has no server support",
                ),
            ),
            Some(_) if !server.is_configured() => HealthReport::failed(
                EngineKind::Remote,
                &BackendError::new(Failure::ServerNotConfigured, "no server host set"),
            ),
            Some(f) => {
                let mut h = f(server).probe();
                h.server.get_or_insert_with(|| server.addr());
                h
            }
        }
    }

    pub fn route_status(&self) -> RouteStatus {
        let st = self.state.lock().unwrap();
        RouteStatus {
            last: st.last_route.clone(),
            circuit: st.circuit.status(Instant::now()),
            health: st.health.clone(),
        }
    }

    /// Close the circuit breaker (Developer "Reset circuit"). Additive.
    pub fn reset_circuit(&self) {
        self.state.lock().unwrap().circuit = Circuit::default();
    }

    // ---- internals --------------------------------------------------------

    fn remote_for(&self, st: &mut State) -> Arc<dyn Backend> {
        if let Some((c, b)) = &st.remote {
            if *c == st.cfg.server {
                return b.clone();
            }
        }
        let f = self.remote_factory.as_ref().expect("remote factory");
        let b = f(&st.cfg.server);
        st.remote = Some((st.cfg.server.clone(), b.clone()));
        b
    }

    fn note_remote_failure(&self, kind: Failure) {
        self.state
            .lock()
            .unwrap()
            .circuit
            .record_failure(kind, Instant::now());
    }

    fn start_local(
        cfg: SessionConfig,
        local: Arc<dyn Backend>,
        route: Option<RouteInfo>,
    ) -> (Active, Result<(), RouterError>) {
        let route = route.unwrap_or_else(|| local_route(None));
        match local.start_session(&cfg) {
            Ok(_) => (
                Active::live(cfg, local, EngineKind::Local, None, route),
                Ok(()),
            ),
            Err(e) => {
                let msg = format!("local engine: {}", e.message);
                (
                    Active::dead(cfg, msg.clone(), Some(route)),
                    Err(RouterError::ServerUnavailable(msg)),
                )
            }
        }
    }

    /// Remote primary. `fallback` = local backend for buffer replay;
    /// `why` = why remote is used although local was preferred.
    fn start_remote(
        &self,
        st: &mut State,
        cfg: SessionConfig,
        fallback: Option<Arc<dyn Backend>>,
        why: Option<Failure>,
    ) -> (Active, Result<(), RouterError>) {
        let addr = st.cfg.server.addr();
        let mut route = RouteInfo {
            engine: EngineKind::Remote.as_str().into(),
            fallback: why.map(|f| f.as_str().into()),
            server: Some(addr),
            ..Default::default()
        };
        let no_fallback_err = |e: &BackendError| match e.kind {
            Failure::ServerNotConfigured => RouterError::ServerNotConfigured,
            _ => RouterError::ServerUnavailable(e.to_string()),
        };
        let fail = |cfg: SessionConfig, e: BackendError, route: RouteInfo| {
            // Start failed: local from scratch if allowed, else a dead session.
            match fallback.clone() {
                Some(l) if l.ready() => {
                    let mut r = local_route(Some(e.kind));
                    r.server = route.server.clone();
                    Self::start_local(cfg, l, Some(r))
                }
                Some(_) => {
                    let msg = format!("{e}; local model not ready");
                    let mut r = route;
                    r.fallback = Some(e.kind.as_str().into());
                    (
                        Active::dead(cfg, format!("server_unavailable: {msg}"), Some(r)),
                        Err(RouterError::ServerUnavailable(msg)),
                    )
                }
                None => {
                    let mut r = route;
                    r.fallback = Some(e.kind.as_str().into());
                    let err = no_fallback_err(&e);
                    (Active::dead(cfg, err.to_string(), Some(r)), Err(err))
                }
            }
        };
        if !st.cfg.server.is_configured() {
            let e = BackendError::new(Failure::ServerNotConfigured, "no server host set");
            return fail(cfg, e, route);
        }
        // Auth/proto failures are not retried until the config changes,
        // even without a fallback.
        if st.circuit.sticky {
            let kind = st.circuit.reason.unwrap_or(Failure::AuthFailed);
            let e = BackendError::new(kind, "not retried until the server settings change");
            return fail(cfg, e, route);
        }
        let remote = self.remote_for(st);
        match remote.start_session(&cfg) {
            Ok(info) if info.queued && fallback.as_ref().is_some_and(|l| l.ready()) => {
                remote.cancel();
                let e = BackendError::new(Failure::Busy, "server queued the session");
                fail(cfg, e, route)
            }
            Ok(_) => {
                let buffer = fallback.is_some().then(Vec::new);
                let mut a = Active::live(cfg, remote, EngineKind::Remote, fallback, route);
                a.buffer = buffer;
                (a, Ok(()))
            }
            Err(e) => {
                st.circuit.record_failure(e.kind, Instant::now());
                route.fallback = None;
                fail(cfg, e, route)
            }
        }
    }

    fn commit_active(&self, a: &mut Active) -> CommitResult {
        if let Some(msg) = a.dead.clone() {
            return error_result(msg, Some(a.route.clone()));
        }
        let engine = a.engine.clone().expect("live session has an engine");
        if !a.remote_primary() {
            return finish(engine.commit(far_deadline()), &a.route);
        }
        if let Some(e) = a.relevant_failure(engine.session_failure()) {
            let e = normalize_mid_session(e);
            self.note_remote_failure(e.kind);
            return a.fallback_commit(e);
        }
        let audio_ms = a.samples * 1000 / SAMPLE_RATE as u64;
        let server = self.state.lock().unwrap().cfg.server.clone();
        let budget = server.commit_timeout_base_ms + audio_ms * 3 / 10 + a.cfg.llm_timeout_ms;
        let deadline = Instant::now() + Duration::from_millis(budget);
        match engine.commit(deadline) {
            Ok(mut r) => {
                // Success closes a half-open circuit.
                let mut st = self.state.lock().unwrap();
                if !st.circuit.sticky {
                    st.circuit = Circuit::default();
                }
                drop(st);
                let mut route = a.route.clone();
                if let Some(rr) = r.route.take() {
                    route.rtt_ms = rr.rtt_ms.or(route.rtt_ms);
                    route.queue_ms = rr.queue_ms.or(route.queue_ms);
                    route.server_core = rr.server_core.or(route.server_core);
                    route.server = rr.server.or(route.server);
                }
                r.route = Some(route);
                r
            }
            Err(e) => {
                // `switch_to_local` cancels the remote session.
                self.note_remote_failure(e.kind);
                a.fallback_commit(e)
            }
        }
    }
}

impl Active {
    /// A queued (busy) server is only a failure when there is a ready local
    /// engine to go to; otherwise the session waits for a server worker.
    fn relevant_failure(&self, e: Option<BackendError>) -> Option<BackendError> {
        e.filter(|e| e.kind != Failure::Busy || self.fallback.as_ref().is_some_and(|l| l.ready()))
    }

    fn live(
        cfg: SessionConfig,
        engine: Arc<dyn Backend>,
        kind: EngineKind,
        fallback: Option<Arc<dyn Backend>>,
        route: RouteInfo,
    ) -> Self {
        Self {
            cfg,
            engine: Some(engine),
            kind,
            fallback,
            buffer: None,
            samples: 0,
            last_ctx: None,
            route,
            dead: None,
        }
    }

    fn dead(cfg: SessionConfig, msg: String, route: Option<RouteInfo>) -> Self {
        Self {
            cfg,
            engine: None,
            kind: EngineKind::Local,
            fallback: None,
            buffer: None,
            samples: 0,
            last_ctx: None,
            route: route.unwrap_or_default(),
            dead: Some(msg),
        }
    }

    /// The remote session failed: continue on local with the buffered
    /// audio, or die (no fallback / local not ready).
    fn switch_to_local(&mut self, e: BackendError) {
        if let Some(r) = self.engine.take() {
            r.cancel();
        }
        self.route.fallback = Some(e.kind.as_str().into());
        let local = match self.fallback.take() {
            Some(l) if l.ready() => l,
            Some(_) => {
                self.dead = Some(format!("server_unavailable: {e}; local model not ready"));
                return;
            }
            None => {
                self.dead = Some(format!("server_unavailable: {e}"));
                return;
            }
        };
        let buf = self.buffer.take().unwrap_or_default();
        let mut cfg = self.cfg.clone();
        cfg.ring_capacity_samples = cfg.ring_capacity_samples.max(buf.len() + REPLAY_HEADROOM);
        if let Err(le) = local.start_session(&cfg) {
            self.dead = Some(format!(
                "server_unavailable: {e}; local engine: {}",
                le.message
            ));
            return;
        }
        for chunk in buf.chunks(flow_protocol::RECOMMENDED_FRAME_SAMPLES * 10) {
            local.push_audio(chunk);
        }
        if let Some(ctx) = &self.last_ctx {
            local.set_context(ctx);
        }
        self.engine = Some(local);
        self.kind = EngineKind::Local;
        self.route.engine = EngineKind::Local.as_str().into();
    }

    fn fallback_commit(&mut self, e: BackendError) -> CommitResult {
        self.switch_to_local(e);
        if let Some(msg) = self.dead.clone() {
            return error_result(msg, Some(self.route.clone()));
        }
        let local = self.engine.clone().expect("switched to local");
        finish(local.commit(far_deadline()), &self.route)
    }
}

fn finish(r: Result<CommitResult, BackendError>, route: &RouteInfo) -> CommitResult {
    match r {
        Ok(mut r) => {
            r.route = Some(route.clone());
            r
        }
        Err(e) => error_result(e.to_string(), Some(route.clone())),
    }
}

fn local_route(fallback: Option<Failure>) -> RouteInfo {
    RouteInfo {
        engine: EngineKind::Local.as_str().into(),
        fallback: fallback.map(|f| f.as_str().into()),
        ..Default::default()
    }
}

/// A connect-time failure seen mid-session is a lost connection.
fn normalize_mid_session(e: BackendError) -> BackendError {
    match e.kind {
        Failure::ConnectFailed => BackendError::new(Failure::Disconnected, e.message),
        _ => e,
    }
}

fn far_deadline() -> Instant {
    Instant::now() + Duration::from_secs(24 * 3600)
}

fn failure_from_str(s: &str) -> Option<Failure> {
    serde_json::from_value(serde_json::Value::String(s.into())).ok()
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn f<T: Send + Sync>() {}
    f::<Router>();
    let _ = now_unix_ms;
}
