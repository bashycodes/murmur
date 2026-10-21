//! The router: routing policy (§1.3, cloud-stt §3), PCM buffer + replay on
//! fallback, circuit breakers, route bookkeeping, cloud usage.
//!
//! One policy walks an ordered list of routes (`server`, `client`,
//! `cloud`): the first route that starts is the primary, the remaining
//! routes form the fallback chain. With `RouterConfig::order` empty the
//! list is the legacy D17 `prefer` order, which never contains the cloud.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use flow_core::dictionary::Dictionary;
use flow_core::learn::Candidate;
use flow_core::{CommitResult, CommitStatus, RouteInfo};

use crate::cloud::{month_of, CloudProviderConfig, CloudUsage, Route, SecretString};
use crate::{
    now_unix_ms, Backend, BackendError, Capabilities, CircuitStatus, CloudCapabilities,
    CloudFactory, EngineKind, Failure, HealthReport, Prefer, RemoteFactory, RouteStatus,
    RouterConfig, RouterError, ServerConfig,
};

const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Headroom added to the local ring when a fallback replays the buffer, so
/// the replay plus live audio never overflows it.
const REPLAY_HEADROOM: usize = SAMPLE_RATE * 30;
/// Cloud commit deadline cap (cloud-stt §3.3).
const CLOUD_DEADLINE_MAX: Duration = Duration::from_secs(90);

struct Circuit {
    open_until: Option<Instant>,
    backoff: Duration,
    reason: Option<Failure>,
    /// Auth/proto (server) or auth/quota (cloud) failures: open until the
    /// config (or key) changes.
    sticky: bool,
    /// Cloud rules: quota is sticky, provider errors back off, 429 opens
    /// for 5 s.
    cloud: bool,
}

impl Circuit {
    fn new(cloud: bool) -> Self {
        Self {
            open_until: None,
            backoff: BACKOFF_MIN,
            reason: None,
            sticky: false,
            cloud,
        }
    }

    fn is_open(&self, now: Instant) -> bool {
        self.sticky || self.open_until.is_some_and(|t| t > now)
    }

    fn record_failure(&mut self, kind: Failure, now: Instant) {
        match kind {
            Failure::AuthFailed | Failure::ProtoMismatch => {
                self.sticky = true;
                self.reason = Some(kind);
            }
            Failure::QuotaExceeded if self.cloud => {
                self.sticky = true;
                self.reason = Some(kind);
            }
            Failure::ConnectFailed | Failure::Timeout | Failure::Disconnected => {
                self.back_off(kind, now)
            }
            Failure::ProviderError if self.cloud => self.back_off(kind, now),
            Failure::Busy if self.cloud => {
                self.open_until = Some(now + BACKOFF_MIN);
                self.reason = Some(kind);
            }
            // The server is up (busy) or there is nothing to retry.
            _ => {}
        }
    }

    fn back_off(&mut self, kind: Failure, now: Instant) {
        self.open_until = Some(now + self.backoff);
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        self.reason = Some(kind);
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
    /// Cloud keys by provider id (memory only; never serialised).
    cloud_keys: HashMap<String, SecretString>,
    /// Cloud backend built for this provider config + key.
    cloud: Option<(CloudProviderConfig, Arc<dyn Backend>)>,
    cloud_circuit: Circuit,
    cloud_usage: CloudUsage,
    cloud_adapters: Vec<String>,
}

impl State {
    /// The active cloud provider and its key, or why the cloud route must
    /// be skipped (C1: consent + key + budget; `allow_cloud` and `order`
    /// are checked by the caller).
    fn cloud_gate(&mut self) -> Result<(CloudProviderConfig, SecretString), BackendError> {
        let not_cfg = |m: &str| BackendError::new(Failure::CloudNotConfigured, m);
        let p = self
            .cfg
            .cloud
            .active_provider()
            .cloned()
            .ok_or_else(|| not_cfg("no active cloud provider"))?;
        p.validate().map_err(|e| not_cfg(&e))?;
        if !p.consent_ok() {
            return Err(not_cfg("no consent recorded for this provider's host"));
        }
        let key = self
            .cloud_keys
            .get(&p.id)
            .filter(|k| !k.is_empty())
            .cloned()
            .ok_or_else(|| not_cfg("no API key set for the cloud provider"))?;
        if let Some(b) = self.cfg.cloud.monthly_budget_usd {
            self.cloud_usage.roll(&month_of(now_unix_ms()));
            if self.cloud_usage.total_cost() >= b.0 {
                return Err(BackendError::new(
                    Failure::BudgetExceeded,
                    "monthly cloud budget reached",
                ));
            }
        }
        Ok((p, key))
    }
}

/// One step of the route list, resolved to a backend.
struct Hop {
    kind: EngineKind,
    backend: Arc<dyn Backend>,
}

struct Active {
    cfg: SessionConfig,
    /// Backend currently fed with audio; `None` = the session is dead.
    engine: Option<Arc<dyn Backend>>,
    kind: EngineKind,
    /// Backends to switch to, in order, if the current one fails.
    fallbacks: VecDeque<Hop>,
    /// Session PCM (only while a fallback exists), capped at `max_samples`.
    buffer: Option<Vec<i16>>,
    samples: u64,
    last_ctx: Option<FieldContext>,
    route: RouteInfo,
    /// Error the commit reports for a dead session.
    dead: Option<String>,
    /// The primary already failed over once (later hops keep its reason).
    switched: bool,
}

/// The §9 session API over local, remote and cloud backends. Thread-safe;
/// one active session at a time (like `FlowCore`).
pub struct Router {
    state: Mutex<State>,
    session: Mutex<Option<Active>>,
    remote_factory: Option<RemoteFactory>,
    cloud_factory: Option<CloudFactory>,
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

/// Outcome of trying one route at session start.
enum Try {
    Started(Arc<dyn Backend>),
    Skip(BackendError),
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
                circuit: Circuit::new(false),
                last_route: None,
                health: None,
                last_dictionary: Dictionary::default(),
                cloud_keys: HashMap::new(),
                cloud: None,
                cloud_circuit: Circuit::new(true),
                cloud_usage: CloudUsage {
                    month: month_of(now_unix_ms()),
                    providers: Vec::new(),
                },
                cloud_adapters: Vec::new(),
            }),
            session: Mutex::new(None),
            remote_factory: remote,
            cloud_factory: None,
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

    /// Enable the cloud route (D18): `f` builds a provider backend
    /// (`flow_cloud::factory`). Without it the cloud is never used and
    /// `capabilities().cloud` is false. Additive.
    pub fn with_cloud(mut self, f: CloudFactory) -> Self {
        self.cloud_factory = Some(f);
        self
    }

    /// Cloud adapters for `capabilities().cloud_adapters`. Additive.
    pub fn with_cloud_adapters(self, adapters: Vec<String>) -> Self {
        self.state.lock().unwrap().cloud_adapters = adapters;
        self
    }

    /// Takes effect at the next `start_session`. A changed server config
    /// drops the cached remote backend and resets the circuit; a changed
    /// active cloud provider does the same for the cloud route.
    pub fn update_config(&self, cfg: RouterConfig) {
        let mut st = self.state.lock().unwrap();
        if st.cfg.server != cfg.server {
            st.remote = None;
            st.circuit = Circuit::new(false);
            st.health = None;
        }
        if st.cfg.cloud.active_provider() != cfg.cloud.active_provider() {
            st.cloud = None;
            st.cloud_circuit = Circuit::new(true);
        }
        st.cfg = cfg;
    }

    /// Set (or with `None` forget) the API key of a cloud provider. Keys
    /// only ever enter the router here: never through config JSON, and
    /// nothing the router returns contains them. A changed key resets the
    /// cloud circuit (auth/quota failures are retried). Additive (D18).
    pub fn set_cloud_key(&self, provider_id: &str, key: Option<SecretString>) {
        let mut st = self.state.lock().unwrap();
        match key.filter(|k| !k.is_empty()) {
            Some(k) => {
                st.cloud_keys.insert(provider_id.to_string(), k);
            }
            None => {
                st.cloud_keys.remove(provider_id);
            }
        }
        if st.cloud.as_ref().is_some_and(|(p, _)| p.id == provider_id)
            || st.cfg.cloud.active == provider_id
        {
            st.cloud = None;
            st.cloud_circuit = Circuit::new(true);
        }
    }

    /// Whether a key is set for `provider_id` (Developer page). Additive.
    pub fn has_cloud_key(&self, provider_id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cloud_keys
            .get(provider_id)
            .is_some_and(|k| !k.is_empty())
    }

    /// This month's cloud counters (the shell persists them). Additive.
    pub fn cloud_usage(&self) -> CloudUsage {
        let mut st = self.state.lock().unwrap();
        st.cloud_usage.roll(&month_of(now_unix_ms()));
        st.cloud_usage.clone()
    }

    /// Restore persisted counters at startup; counters of another month are
    /// dropped. Additive.
    pub fn set_cloud_usage(&self, mut usage: CloudUsage) {
        usage.roll(&month_of(now_unix_ms()));
        self.state.lock().unwrap().cloud_usage = usage;
    }

    /// Developer "Reset counters". Additive.
    pub fn reset_cloud_usage(&self) {
        self.state.lock().unwrap().cloud_usage = CloudUsage {
            month: month_of(now_unix_ms()),
            providers: Vec::new(),
        };
    }

    /// The shell (re)loaded (or unloaded) its local models. Also drops the
    /// cached cloud backend, which shares the local LLM cleaner, so the next
    /// cloud session is built with the current one.
    pub fn set_local(&self, local: Option<Arc<dyn Backend>>) {
        let mut st = self.state.lock().unwrap();
        st.cloud = None;
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

    /// Cloud STT support of this build (D18). Additive.
    pub fn cloud_capabilities(&self) -> CloudCapabilities {
        let st = self.state.lock().unwrap();
        let cloud = self.cloud_factory.is_some();
        CloudCapabilities {
            cloud,
            adapters: if cloud {
                st.cloud_adapters.clone()
            } else {
                Vec::new()
            },
        }
    }

    // ---- §9 mirror -------------------------------------------------------

    pub fn start_session(&self, cfg: SessionConfig) -> Result<(), RouterError> {
        self.cancel();
        let mut st = self.state.lock().unwrap();
        st.last_dictionary = cfg.dictionary.clone();
        let (active, res) = self.start_routes(&mut st, cfg);
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
        if a.kind == EngineKind::Local {
            engine.push_audio(pcm);
            return true;
        }
        let ok = engine.push_audio(pcm);
        let failure = a.relevant_failure(engine.session_failure()).or_else(|| {
            (!ok).then(|| BackendError::new(Failure::Disconnected, "audio not accepted"))
        });
        if let Some(e) = failure {
            let e = normalize_mid_session(e);
            self.note_failure(a.kind, e.kind);
            self.switch_next(a, e);
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
                st.circuit = Circuit::new(false);
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

    /// Developer "Test provider" (D18): transcribe the bundled test clip
    /// with an (unsaved) provider config. `key: None` = the stored key for
    /// `p.id`. Blocking; never panics. Does not need consent (the clip is
    /// public-domain audio, not the user's). A success for the active
    /// provider closes the cloud circuit. Additive.
    pub fn probe_cloud(&self, p: &CloudProviderConfig, key: Option<SecretString>) -> HealthReport {
        let fail = |k: Failure, m: &str| {
            let mut h = HealthReport::failed(EngineKind::Cloud, &BackendError::new(k, m));
            h.provider = Some(p.id.clone());
            h.model = Some(p.model.clone());
            h
        };
        let Some(f) = &self.cloud_factory else {
            return fail(
                Failure::CloudNotConfigured,
                "this build has no cloud support",
            );
        };
        if let Err(e) = p.validate() {
            return fail(Failure::CloudNotConfigured, &e);
        }
        let key = key
            .filter(|k| !k.is_empty())
            .or_else(|| self.state.lock().unwrap().cloud_keys.get(&p.id).cloned());
        let Some(key) = key.filter(|k| !k.is_empty()) else {
            return fail(Failure::CloudNotConfigured, "no API key");
        };
        let mut h = f(p, key).probe();
        h.engine = EngineKind::Cloud;
        h.provider.get_or_insert_with(|| p.id.clone());
        h.model.get_or_insert_with(|| p.model.clone());
        let mut st = self.state.lock().unwrap();
        if st.cfg.cloud.active_provider() == Some(p) {
            if h.ok {
                st.cloud_circuit = Circuit::new(true);
            } else if let Some(k) = h.error.as_deref().and_then(failure_from_str) {
                st.cloud_circuit.record_failure(k, Instant::now());
            }
        }
        h
    }

    pub fn route_status(&self) -> RouteStatus {
        let st = self.state.lock().unwrap();
        let now = Instant::now();
        RouteStatus {
            last: st.last_route.clone(),
            circuit: st.circuit.status(now),
            health: st.health.clone(),
            cloud_circuit: self
                .cloud_factory
                .is_some()
                .then(|| st.cloud_circuit.status(now)),
        }
    }

    /// Close the circuit breakers (Developer "Reset circuit"). Additive.
    pub fn reset_circuit(&self) {
        let mut st = self.state.lock().unwrap();
        st.circuit = Circuit::new(false);
        st.cloud_circuit = Circuit::new(true);
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

    fn cloud_for(
        &self,
        st: &mut State,
        p: CloudProviderConfig,
        key: SecretString,
    ) -> Arc<dyn Backend> {
        if let Some((c, b)) = &st.cloud {
            if *c == p {
                return b.clone();
            }
        }
        let f = self.cloud_factory.as_ref().expect("cloud factory");
        let b = f(&p, key);
        st.cloud = Some((p, b.clone()));
        b
    }

    fn note_failure(&self, kind: EngineKind, f: Failure) {
        let mut st = self.state.lock().unwrap();
        let now = Instant::now();
        match kind {
            EngineKind::Remote => st.circuit.record_failure(f, now),
            EngineKind::Cloud => st.cloud_circuit.record_failure(f, now),
            _ => {}
        }
    }

    /// Routes this build + the user toggles allow, in order.
    fn routes(&self, st: &State) -> Vec<Route> {
        let server_eff = self.remote_factory.is_some() && st.cfg.allow_server;
        let client_eff = st.local_capable && st.cfg.allow_client;
        let cloud_eff = self.cloud_factory.is_some() && st.cfg.allow_cloud;
        let list = if st.cfg.order.is_empty() {
            // Legacy D17 policy: never the cloud.
            match st.cfg.prefer {
                Prefer::Client => vec![Route::Client, Route::Server],
                _ => vec![Route::Server, Route::Client],
            }
        } else {
            st.cfg.order.clone()
        };
        list.into_iter()
            .filter(|r| match r {
                Route::Server => server_eff,
                Route::Client => client_eff,
                Route::Cloud => cloud_eff,
            })
            .collect()
    }

    /// Is some route after `i` likely to serve the session (a ready local
    /// engine, or a closed-circuit server/cloud)? Open circuits are only
    /// honoured when it is.
    fn viable_after(&self, st: &State, routes: &[Route], i: usize) -> bool {
        let now = Instant::now();
        routes[i + 1..].iter().any(|r| match r {
            Route::Client => st.local.as_ref().is_some_and(|l| l.ready()),
            Route::Server => st.cfg.server.is_configured() && !st.circuit.is_open(now),
            Route::Cloud => !st.cloud_circuit.is_open(now),
        })
    }

    /// Resolve route `r` to a backend for the fallback chain (no network).
    fn hop(&self, st: &mut State, r: Route) -> Option<Hop> {
        let (kind, backend) = match r {
            Route::Client => (EngineKind::Local, st.local.clone()?),
            Route::Server => {
                if !st.cfg.server.is_configured() || st.circuit.sticky {
                    return None;
                }
                (EngineKind::Remote, self.remote_for(st))
            }
            Route::Cloud => {
                if st.cloud_circuit.sticky {
                    return None;
                }
                let (p, key) = st.cloud_gate().ok()?;
                (EngineKind::Cloud, self.cloud_for(st, p, key))
            }
        };
        Some(Hop { kind, backend })
    }

    fn start_routes(
        &self,
        st: &mut State,
        cfg: SessionConfig,
    ) -> (Active, Result<(), RouterError>) {
        if let Err(e) = st.cfg.validate_order() {
            return (Active::dead(cfg, e.to_string(), None), Err(e));
        }
        let routes = self.routes(st);
        if routes.is_empty() {
            return (
                Active::dead(cfg, RouterError::NoEngineEnabled.to_string(), None),
                Err(RouterError::NoEngineEnabled),
            );
        }
        let mut route = RouteInfo::default();
        // Last non-local failure (engine, error, index) for the error path.
        let mut last_fail: Option<(EngineKind, BackendError, usize)> = None;
        let mut local_not_ready = false;
        for (i, r) in routes.iter().copied().enumerate() {
            let kind = r.engine();
            let tried = match r {
                Route::Client => match st.local.clone().filter(|l| l.ready()) {
                    Some(l) => match l.start_session(&cfg) {
                        Ok(_) => Try::Started(l),
                        Err(e) => {
                            // A broken local engine is final, as in D17.
                            let msg = format!("local engine: {}", e.message);
                            route.engine = EngineKind::Local.as_str().into();
                            return (
                                Active::dead(cfg, msg.clone(), Some(route)),
                                Err(RouterError::ServerUnavailable(msg)),
                            );
                        }
                    },
                    None => {
                        local_not_ready = true;
                        Try::Skip(BackendError::new(
                            Failure::LocalNotReady,
                            "local model not loaded",
                        ))
                    }
                },
                Route::Server => {
                    route.server = Some(st.cfg.server.addr());
                    self.try_server(st, &cfg, &routes, i)
                }
                Route::Cloud => self.try_cloud(st, &cfg, &routes, i, &mut route),
            };
            match tried {
                Try::Started(backend) => {
                    route.engine = kind.as_str().into();
                    if kind != EngineKind::Cloud {
                        route.provider = None;
                        route.model = None;
                    }
                    let mut fallbacks = VecDeque::new();
                    if kind != EngineKind::Local {
                        for r in &routes[i + 1..] {
                            if let Some(h) = self.hop(st, *r) {
                                fallbacks.push_back(h);
                            }
                        }
                    }
                    let mut a = Active::live(cfg, backend, kind, route);
                    if !fallbacks.is_empty() {
                        a.buffer = Some(Vec::new());
                    }
                    a.fallbacks = fallbacks;
                    return (a, Ok(()));
                }
                Try::Skip(e) => {
                    route.fallback.get_or_insert_with(|| e.kind.as_str().into());
                    if kind != EngineKind::Local {
                        last_fail = Some((kind, e, i));
                    }
                }
            }
        }
        // Nothing started.
        let Some((kind, e, i)) = last_fail else {
            // Only local routes, none ready.
            return (
                Active::dead(cfg, RouterError::LocalNotReady.to_string(), None),
                Err(RouterError::LocalNotReady),
            );
        };
        route.engine = kind.as_str().into();
        route.fallback = Some(e.kind.as_str().into());
        let local_after = routes[i + 1..].contains(&Route::Client) && st.local.is_some();
        let err = if local_after && local_not_ready {
            RouterError::ServerUnavailable(format!("{e}; local model not ready"))
        } else if kind == EngineKind::Cloud
            && matches!(
                e.kind,
                Failure::CloudNotConfigured | Failure::BudgetExceeded
            )
            && routes.iter().all(|r| *r == Route::Cloud)
        {
            // The only listed route is the cloud and it is not usable.
            RouterError::NoEngineEnabled
        } else if e.kind == Failure::ServerNotConfigured {
            RouterError::ServerNotConfigured
        } else {
            RouterError::ServerUnavailable(e.to_string())
        };
        let msg = match &err {
            RouterError::NoEngineEnabled => format!("{err} ({e})"),
            _ => err.to_string(),
        };
        (Active::dead(cfg, msg, Some(route)), Err(err))
    }

    fn try_server(&self, st: &mut State, cfg: &SessionConfig, routes: &[Route], i: usize) -> Try {
        if !st.cfg.server.is_configured() {
            return Try::Skip(BackendError::new(
                Failure::ServerNotConfigured,
                "no server host set",
            ));
        }
        let viable = self.viable_after(st, routes, i);
        if viable && st.circuit.is_open(Instant::now()) {
            return Try::Skip(BackendError::new(Failure::CircuitOpen, "circuit open"));
        }
        // Auth/proto failures are not retried until the config changes,
        // even without a fallback.
        if st.circuit.sticky {
            let kind = st.circuit.reason.unwrap_or(Failure::AuthFailed);
            return Try::Skip(BackendError::new(
                kind,
                "not retried until the server settings change",
            ));
        }
        let remote = self.remote_for(st);
        match remote.start_session(cfg) {
            Ok(info) if info.queued && viable => {
                remote.cancel();
                Try::Skip(BackendError::new(
                    Failure::Busy,
                    "server queued the session",
                ))
            }
            Ok(_) => Try::Started(remote),
            Err(e) => {
                st.circuit.record_failure(e.kind, Instant::now());
                Try::Skip(e)
            }
        }
    }

    fn try_cloud(
        &self,
        st: &mut State,
        cfg: &SessionConfig,
        routes: &[Route],
        i: usize,
        route: &mut RouteInfo,
    ) -> Try {
        let (p, key) = match st.cloud_gate() {
            Ok(x) => x,
            Err(e) => return Try::Skip(e),
        };
        route.provider = Some(p.id.clone());
        route.model = Some(p.model.clone());
        if st.cloud_circuit.is_open(Instant::now()) && self.viable_after(st, routes, i) {
            return Try::Skip(BackendError::new(
                Failure::CircuitOpen,
                "cloud circuit open",
            ));
        }
        if st.cloud_circuit.sticky {
            let kind = st.cloud_circuit.reason.unwrap_or(Failure::AuthFailed);
            return Try::Skip(BackendError::new(
                kind,
                "not retried until the cloud key or provider changes",
            ));
        }
        let cloud = self.cloud_for(st, p, key);
        match cloud.start_session(cfg) {
            Ok(_) => Try::Started(cloud),
            Err(e) => Try::Skip(e),
        }
    }

    fn commit_active(&self, a: &mut Active) -> CommitResult {
        loop {
            if let Some(msg) = a.dead.clone() {
                return error_result(msg, Some(a.route.clone()));
            }
            let engine = a.engine.clone().expect("live session has an engine");
            if a.kind == EngineKind::Local {
                return finish(engine.commit(far_deadline()), &a.route);
            }
            if let Some(e) = a.relevant_failure(engine.session_failure()) {
                let e = normalize_mid_session(e);
                self.note_failure(a.kind, e.kind);
                self.switch_next(a, e);
                continue;
            }
            let audio_ms = a.samples * 1000 / SAMPLE_RATE as u64;
            let deadline = self.commit_deadline(a.kind, audio_ms, &a.cfg);
            match engine.commit(deadline) {
                Ok(r) => return self.finish_remote(a, r),
                Err(e) => {
                    // `switch_next` cancels the failed session.
                    self.note_failure(a.kind, e.kind);
                    self.switch_next(a, e);
                }
            }
        }
    }

    fn commit_deadline(&self, kind: EngineKind, audio_ms: u64, cfg: &SessionConfig) -> Instant {
        let st = self.state.lock().unwrap();
        let ms = match kind {
            EngineKind::Cloud => {
                let base = st.cloud.as_ref().map_or(6000, |(p, _)| p.timeout_base_ms);
                let d = Duration::from_millis(base + audio_ms / 2 + cfg.llm_timeout_ms);
                return Instant::now() + d.min(CLOUD_DEADLINE_MAX);
            }
            _ => st.cfg.server.commit_timeout_base_ms + audio_ms * 3 / 10 + cfg.llm_timeout_ms,
        };
        Instant::now() + Duration::from_millis(ms)
    }

    /// A remote/cloud backend produced the (final) result.
    fn finish_remote(&self, a: &Active, mut r: CommitResult) -> CommitResult {
        let mut st = self.state.lock().unwrap();
        let mut route = a.route.clone();
        let rr = r.route.take();
        match a.kind {
            EngineKind::Cloud => {
                // Success closes a half-open circuit.
                if !st.cloud_circuit.sticky {
                    st.cloud_circuit = Circuit::new(true);
                }
                if let Some(rr) = rr {
                    route.provider = rr.provider.or(route.provider);
                    route.model = rr.model.or(route.model);
                    route.billed_audio_ms = rr.billed_audio_ms;
                    route.cost_usd = rr.cost_usd;
                    route.cost_estimated = rr.cost_estimated;
                }
                if let (Some(id), Some(ms)) = (route.provider.clone(), route.billed_audio_ms) {
                    st.cloud_usage.roll(&month_of(now_unix_ms()));
                    st.cloud_usage.record(
                        &id,
                        ms,
                        route.cost_usd,
                        route.cost_estimated.unwrap_or(false),
                    );
                }
            }
            _ => {
                if !st.circuit.sticky {
                    st.circuit = Circuit::new(false);
                }
                if let Some(rr) = rr {
                    route.rtt_ms = rr.rtt_ms.or(route.rtt_ms);
                    route.queue_ms = rr.queue_ms.or(route.queue_ms);
                    route.server_core = rr.server_core.or(route.server_core);
                    route.server = rr.server.or(route.server);
                }
            }
        }
        r.route = Some(route);
        r
    }

    /// The current backend failed: continue on the next one in the chain
    /// with the buffered audio, or die.
    fn switch_next(&self, a: &mut Active, e: BackendError) {
        if let Some(r) = a.engine.take() {
            r.cancel();
        }
        // The primary's failure explains the route; later hops keep it.
        if !a.switched {
            a.route.fallback = Some(e.kind.as_str().into());
            a.switched = true;
        }
        loop {
            let Some(hop) = a.fallbacks.pop_front() else {
                a.dead = Some(format!("server_unavailable: {e}"));
                return;
            };
            if hop.kind == EngineKind::Local && !hop.backend.ready() {
                if a.fallbacks.is_empty() {
                    a.dead = Some(format!("server_unavailable: {e}; local model not ready"));
                    return;
                }
                continue;
            }
            let more = !a.fallbacks.is_empty();
            let buf = if more {
                a.buffer.clone().unwrap_or_default()
            } else {
                a.buffer.take().unwrap_or_default()
            };
            let mut cfg = a.cfg.clone();
            cfg.ring_capacity_samples = cfg.ring_capacity_samples.max(buf.len() + REPLAY_HEADROOM);
            match hop.backend.start_session(&cfg) {
                Ok(_) => {}
                Err(le) if hop.kind == EngineKind::Local => {
                    a.dead = Some(format!(
                        "server_unavailable: {e}; local engine: {}",
                        le.message
                    ));
                    return;
                }
                Err(le) => {
                    self.note_failure(hop.kind, le.kind);
                    continue;
                }
            }
            for chunk in buf.chunks(flow_protocol::RECOMMENDED_FRAME_SAMPLES * 10) {
                hop.backend.push_audio(chunk);
            }
            if let Some(ctx) = &a.last_ctx {
                hop.backend.set_context(ctx);
            }
            if hop.kind == EngineKind::Remote {
                let st = self.state.lock().unwrap();
                a.route.server.get_or_insert_with(|| st.cfg.server.addr());
            }
            if hop.kind != EngineKind::Cloud {
                a.route.provider = None;
                a.route.model = None;
            }
            a.engine = Some(hop.backend);
            a.kind = hop.kind;
            a.route.engine = hop.kind.as_str().into();
            return;
        }
    }
}

impl Active {
    /// A queued (busy) server is only a failure when there is a ready local
    /// engine (or another remote route) to go to; otherwise the session
    /// waits for a server worker.
    fn relevant_failure(&self, e: Option<BackendError>) -> Option<BackendError> {
        e.filter(|e| {
            e.kind != Failure::Busy
                || self
                    .fallbacks
                    .iter()
                    .any(|h| h.kind != EngineKind::Local || h.backend.ready())
        })
    }

    fn live(
        cfg: SessionConfig,
        engine: Arc<dyn Backend>,
        kind: EngineKind,
        route: RouteInfo,
    ) -> Self {
        Self {
            cfg,
            engine: Some(engine),
            kind,
            fallbacks: VecDeque::new(),
            buffer: None,
            samples: 0,
            last_ctx: None,
            route,
            dead: None,
            switched: false,
        }
    }

    fn dead(cfg: SessionConfig, msg: String, route: Option<RouteInfo>) -> Self {
        Self {
            cfg,
            engine: None,
            kind: EngineKind::Local,
            fallbacks: VecDeque::new(),
            buffer: None,
            samples: 0,
            last_ctx: None,
            route: route.unwrap_or_default(),
            dead: Some(msg),
            switched: false,
        }
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
}
