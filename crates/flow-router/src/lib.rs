//! flow-router: one §9 session API over a local `FlowCore` and an optional
//! remote `flow-server` backend (D17, `docs/hybrid-architecture.md` §1).
//!
//! Shells talk only to [`Router`]. The remote backend is injected as a
//! [`RemoteFactory`] (from `flow-remote`) and the opt-in cloud STT backend
//! (D18) as a [`CloudFactory`] (from `flow-cloud`), so a client-only build
//! links this crate without any network code.

pub mod cloud;
mod local;
pub mod net;
mod router;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use cloud::{
    month_of, CloudConfig, CloudProviderConfig, CloudUsage, ProviderUsage, Route, SecretString, Usd,
};
pub use flow_core::config::{FieldContext, SessionConfig};
pub use flow_core::learn::Candidate;
pub use flow_core::{CommitResult, CommitStatus, RouteInfo, SessionPoll};
pub use router::Router;

/// Which engine ran (or would run) a session. Non-exhaustive: match with a
/// `_` arm. `Cloud` = opt-in third-party STT (D18; `RouteInfo::provider`
/// names the provider).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineKind {
    #[default]
    Local,
    Remote,
    Cloud,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::Local => "local",
            EngineKind::Remote => "remote",
            EngineKind::Cloud => "cloud",
        }
    }
}

/// Primary engine when both are allowed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Prefer {
    #[default]
    Server,
    Client,
}

/// Router settings (Developer page; §5). Every field has a default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouterConfig {
    pub allow_server: bool,
    pub allow_client: bool,
    pub prefer: Prefer,
    pub server: ServerConfig,
    /// Shell hint: load local models at app start even when the server is
    /// preferred, so a fallback costs only local ASR time.
    pub keep_local_warm: bool,
    /// Opt-in third-party cloud STT (D18). Off by default; even when on,
    /// the cloud is used only if `order` lists `cloud`, the active provider
    /// has a recorded consent and a key (`Router::set_cloud_key`), and the
    /// monthly budget is not used up. Omitted from JSON while false.
    #[serde(skip_serializing_if = "is_false")]
    pub allow_cloud: bool,
    /// Routes in the order they are tried. Empty = legacy D17 policy from
    /// `prefer` (which never includes the cloud). Omitted while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<Route>,
    /// Cloud providers (no keys). Omitted while default.
    #[serde(skip_serializing_if = "CloudConfig::is_default")]
    pub cloud: CloudConfig,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            allow_server: true,
            allow_client: true,
            prefer: Prefer::Server,
            server: ServerConfig::default(),
            keep_local_warm: true,
            allow_cloud: false,
            order: Vec::new(),
            cloud: CloudConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Host or IP literal; empty = not configured.
    pub host: String,
    pub port: u16,
    /// `wss://` instead of `ws://` (required for public addresses).
    pub tls: bool,
    /// SHA-256 of the server's leaf certificate DER, 64 hex chars.
    pub tls_pin_sha256: Option<String>,
    /// Bearer token (>= 16 chars). Never logged.
    pub token: String,
    pub connect_timeout_ms: u64,
    pub commit_timeout_base_ms: u64,
    pub health_ttl_s: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: flow_protocol::DEFAULT_PORT,
            tls: false,
            tls_pin_sha256: None,
            token: String::new(),
            connect_timeout_ms: 800,
            commit_timeout_base_ms: 3000,
            health_ttl_s: 30,
        }
    }
}

impl ServerConfig {
    /// `"host:port"` (IPv6 hosts bracketed).
    pub fn addr(&self) -> String {
        let h = net::bare_host(&self.host);
        if h.contains(':') {
            format!("[{h}]:{}", self.port)
        } else {
            format!("{h}:{}", self.port)
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.host.trim().is_empty()
    }

    /// The §5 field rules. An empty host is valid (not configured).
    pub fn validate(&self) -> Result<(), String> {
        if self.is_configured() {
            net::validate_host(&self.host)?;
            if !self.tls && !net::classify(&self.host).plaintext_ok() {
                return Err("public address: TLS (wss) is required".into());
            }
        }
        if self.port == 0 {
            return Err("port must be 1-65535".into());
        }
        if let Some(pin) = self.tls_pin_sha256.as_deref() {
            let hex: String = pin.chars().filter(|c| *c != ':').collect();
            if !hex.is_empty() && (hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit())) {
                return Err("certificate pin must be 64 hex characters (SHA-256)".into());
            }
        }
        if !self.token.is_empty() && self.token.len() < flow_protocol::MIN_TOKEN_LEN {
            return Err(format!(
                "token must be at least {} characters",
                flow_protocol::MIN_TOKEN_LEN
            ));
        }
        let ranges = [
            ("connect_timeout_ms", self.connect_timeout_ms, 100, 10_000),
            (
                "commit_timeout_base_ms",
                self.commit_timeout_base_ms,
                500,
                60_000,
            ),
            ("health_ttl_s", self.health_ttl_s, 5, 600),
        ];
        for (name, v, lo, hi) in ranges {
            if !(lo..=hi).contains(&v) {
                return Err(format!("{name} must be {lo}-{hi}"));
            }
        }
        Ok(())
    }

    /// The pin as 64 lowercase hex chars (colons stripped), if set.
    pub fn pin_hex(&self) -> Option<String> {
        let p: String = self
            .tls_pin_sha256
            .as_deref()?
            .chars()
            .filter(|c| *c != ':')
            .collect::<String>()
            .to_ascii_lowercase();
        (!p.is_empty()).then_some(p)
    }
}

impl RouterConfig {
    /// Allowed engines in the order the router tries them (first =
    /// primary): `order` when set, else derived from `prefer` + the two
    /// toggles (never the cloud). Build capability, keys, consent and
    /// budget are not applied here (see `Router::capabilities`).
    pub fn route_order(&self) -> Vec<EngineKind> {
        let routes = if self.order.is_empty() {
            match self.prefer {
                Prefer::Server => vec![Route::Server, Route::Client],
                Prefer::Client => vec![Route::Client, Route::Server],
            }
        } else {
            self.order.clone()
        };
        routes
            .into_iter()
            .filter(|r| self.route_allowed(*r))
            .map(Route::engine)
            .collect()
    }

    /// The user toggle for a route.
    pub fn route_allowed(&self, r: Route) -> bool {
        match r {
            Route::Server => self.allow_server,
            Route::Client => self.allow_client,
            Route::Cloud => self.allow_cloud,
        }
    }

    /// Validate before `Router::update_config` / `flow_open_router`.
    pub fn validate(&self) -> Result<(), RouterError> {
        self.server.validate().map_err(RouterError::BadConfig)?;
        self.validate_order()?;
        self.cloud.validate().map_err(RouterError::BadConfig)
    }

    pub(crate) fn validate_order(&self) -> Result<(), RouterError> {
        for (i, r) in self.order.iter().enumerate() {
            if self.order[..i].contains(r) {
                return Err(RouterError::BadConfig(format!(
                    "route order lists {:?} twice",
                    r
                )));
            }
        }
        Ok(())
    }
}

/// What this build (and its loaded models) can do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    pub local: bool,
    pub remote: bool,
    pub asr_engines: Vec<String>,
    pub llm: bool,
    pub proto: u32,
    pub core_version: String,
}

/// Cloud STT support of this build (D18, `Router::cloud_capabilities`).
/// Separate from [`Capabilities`] so that type's fields stay as they were.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudCapabilities {
    /// The build can reach a third-party provider (`Router::with_cloud`).
    pub cloud: bool,
    /// Adapters compiled in (`openai_compat`, `openrouter`, …).
    pub adapters: Vec<String>,
}

/// Why a backend could not serve a session (`RouteInfo::fallback` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Failure {
    ConnectFailed,
    AuthFailed,
    ProtoMismatch,
    Busy,
    Timeout,
    Disconnected,
    CircuitOpen,
    ServerNotConfigured,
    LocalNotReady,
    /// Anything else (e.g. a local engine error).
    Error,
    /// Cloud route skipped: no active provider, no consent for its host,
    /// or no key (D18).
    CloudNotConfigured,
    /// Cloud route skipped: the monthly budget is used up.
    BudgetExceeded,
    /// Provider says the account is out of credits / quota (HTTP 402 …).
    QuotaExceeded,
    /// Provider answered with an error (5xx, 4xx other than auth/quota/
    /// rate limit) or an unparseable body.
    ProviderError,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Failure::ConnectFailed => "connect_failed",
            Failure::AuthFailed => "auth_failed",
            Failure::ProtoMismatch => "proto_mismatch",
            Failure::Busy => "busy",
            Failure::Timeout => "timeout",
            Failure::Disconnected => "disconnected",
            Failure::CircuitOpen => "circuit_open",
            Failure::ServerNotConfigured => "server_not_configured",
            Failure::LocalNotReady => "local_not_ready",
            Failure::Error => "error",
            Failure::CloudNotConfigured => "cloud_not_configured",
            Failure::BudgetExceeded => "budget_exceeded",
            Failure::QuotaExceeded => "quota_exceeded",
            Failure::ProviderError => "provider_error",
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind}: {message}")]
pub struct BackendError {
    pub kind: Failure,
    pub message: String,
}

impl BackendError {
    pub fn new(kind: Failure, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Result of a successful `Backend::start_session` and of
/// `Router::start_session_info`. JSON: `{"queued":bool,"route"?:SessionRoute}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartInfo {
    /// The server accepted the session but has no free worker yet.
    pub queued: bool,
    /// The route the session started on. Set by the router
    /// (`Router::start_session_info`); backends leave it `None`. Omitted
    /// from JSON while `None`. Added 2026-10-22 (additive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<SessionRoute>,
}

/// The route a live session is using right now: what a shell shows as
/// "Transcribing via …" while recording. A projection of the session's
/// [`RouteInfo`]: once the session commits,
/// `SessionRoute::from_route_info(result.route)` equals the last value
/// `Router::poll` reported. Added 2026-10-22 (additive).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionRoute {
    /// `local` / `remote` (own server) / `cloud`. Always serialised.
    pub engine: EngineKind,
    /// Cloud provider id (`CloudProviderConfig::id`); cloud only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Cloud model id; cloud only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `"host:port"` of the server; remote only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Why the first route is not the one in use (a [`Failure`] string,
    /// same as `RouteInfo::fallback`): set at start when a route was
    /// skipped, or mid-session after a switch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

impl SessionRoute {
    /// The live-route view of a `RouteInfo` (`CommitResult.route`).
    /// `server` is kept only for the remote engine and `provider`/`model`
    /// only for the cloud, so a start-time server address that was tried
    /// and skipped does not leak into a local route.
    pub fn from_route_info(r: &RouteInfo) -> Self {
        let engine = match r.engine.as_str() {
            "remote" => EngineKind::Remote,
            "cloud" => EngineKind::Cloud,
            _ => EngineKind::Local,
        };
        let cloud = engine == EngineKind::Cloud;
        Self {
            engine,
            provider: r.provider.clone().filter(|_| cloud),
            model: r.model.clone().filter(|_| cloud),
            server: r.server.clone().filter(|_| engine == EngineKind::Remote),
            fallback: r.fallback.clone(),
        }
    }
}

/// `Router::poll`: the core [`SessionPoll`] (partial + hands-free endpoint,
/// D19) plus the live route. JSON = the `SessionPoll` fields flattened, plus
/// `route` (omitted while no session is live, so the idle JSON is
/// byte-identical to `SessionPoll`'s). Derefs to `SessionPoll`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouterPoll {
    #[serde(flatten)]
    pub session: SessionPoll,
    /// Route in use by the live session; `None` with no session, after
    /// `commit`/`cancel`, or when every route failed (the commit will be
    /// an ERROR).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<SessionRoute>,
}

impl std::ops::Deref for RouterPoll {
    type Target = SessionPoll;
    fn deref(&self) -> &SessionPoll {
        &self.session
    }
}

impl From<RouterPoll> for SessionPoll {
    fn from(p: RouterPoll) -> Self {
        p.session
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rtt {
    pub min: u32,
    pub median: u32,
    pub max: u32,
}

/// Outcome of a health probe (Developer page "Test connection").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthReport {
    pub ok: bool,
    pub engine: EngineKind,
    /// Failure code (`Failure::as_str`) when not ok.
    pub error: Option<String>,
    pub message: Option<String>,
    /// `"host:port"` probed (remote).
    pub server: Option<String>,
    pub rtt_ms: Option<Rtt>,
    /// e.g. `"flow-server 0.1.0"`.
    pub server_version: Option<String>,
    pub server_core: Option<String>,
    pub postprocess_rev: Option<String>,
    /// Server pipeline revision == this build's `flow_core::POSTPROCESS_REV`.
    pub postprocess_match: Option<bool>,
    pub proto: Option<u32>,
    pub asr_backend: Option<String>,
    pub asr_model: Option<String>,
    pub llm: Option<bool>,
    pub workers: Option<u32>,
    pub busy: Option<u32>,
    /// Unix time of the probe (ms).
    pub checked_at_ms: u64,
    /// Cloud "Test provider" (D18): provider id, model, the transcript of
    /// the test clip, wall time and cost. Omitted while `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Usd>,
}

impl HealthReport {
    pub fn failed(engine: EngineKind, e: &BackendError) -> Self {
        Self {
            ok: false,
            engine,
            error: Some(e.kind.as_str().into()),
            message: Some(e.message.clone()),
            checked_at_ms: now_unix_ms(),
            ..Default::default()
        }
    }
}

/// Circuit-breaker state for the remote backend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CircuitStatus {
    pub open: bool,
    /// Failure that opened it.
    pub reason: Option<String>,
    /// Remaining backoff; `None` while closed or until the config changes.
    pub retry_in_ms: Option<u64>,
    /// Open until the server config changes (auth/proto failures).
    pub until_config_change: bool,
}

/// Developer page "Active engine" + "Circuit" + "Server info" rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteStatus {
    /// Route of the last committed session.
    pub last: Option<RouteInfo>,
    pub circuit: CircuitStatus,
    /// Last probe result (`probe_server`), if any.
    pub health: Option<HealthReport>,
    /// Cloud route circuit (D18); present only in builds with cloud
    /// support. Auth/quota failures keep it open until the key or the
    /// provider config changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_circuit: Option<CircuitStatus>,
}

/// Errors from `Router::start_session` (stable snake_case kinds).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouterError {
    #[error("no_engine_enabled: turn on Allow server or Allow client in Developer settings")]
    NoEngineEnabled,
    #[error("local_not_ready: local model not loaded")]
    LocalNotReady,
    #[error("server_not_configured: no server host set")]
    ServerNotConfigured,
    #[error("server_unavailable: {0}")]
    ServerUnavailable(String),
    #[error("bad_config: {0}")]
    BadConfig(String),
}

impl RouterError {
    pub fn kind(&self) -> &'static str {
        match self {
            RouterError::NoEngineEnabled => "no_engine_enabled",
            RouterError::LocalNotReady => "local_not_ready",
            RouterError::ServerNotConfigured => "server_not_configured",
            RouterError::ServerUnavailable(_) => "server_unavailable",
            RouterError::BadConfig(_) => "bad_config",
        }
    }
}

/// One engine behind the router: in-process `FlowCore` or a `flow-server`.
pub trait Backend: Send + Sync {
    fn kind(&self) -> EngineKind;
    /// Local: models loaded.
    fn ready(&self) -> bool;
    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError>;
    /// Non-blocking. `false` = no active session, or (remote) the session is
    /// dead; see [`Backend::session_failure`].
    fn push_audio(&self, pcm: &[i16]) -> bool;
    fn set_context(&self, ctx: &FieldContext);
    fn partial(&self) -> Option<String>;
    /// Blocks until the result or `deadline`. An `Ok` result is final.
    fn commit(&self, deadline: std::time::Instant) -> Result<CommitResult, BackendError>;
    fn cancel(&self);
    /// Local: ready; remote: hello + 3 pings.
    fn probe(&self) -> HealthReport;
    /// Non-blocking: why the current session can no longer produce a result
    /// (remote: connection lost, server busy, no `started` in time). Polled
    /// by the router on every push. Added to §1.2 (additive, default `None`).
    fn session_failure(&self) -> Option<BackendError> {
        None
    }
}

pub type RemoteFactory = Box<dyn Fn(&ServerConfig) -> Arc<dyn Backend> + Send + Sync>;

/// Builds the cloud backend for a provider + its key (`flow-cloud`).
/// Called at most once per (provider config, key) and cached.
pub type CloudFactory =
    Box<dyn Fn(&CloudProviderConfig, SecretString) -> Arc<dyn Backend> + Send + Sync>;

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
