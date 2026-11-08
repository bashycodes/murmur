//! flow-router: one §9 session API over a local `FlowCore` and an optional
//! remote `flow-server` backend (D17, `docs/hybrid-architecture.md` §1).
//!
//! Shells talk only to [`Router`]. The remote backend is injected as a
//! [`RemoteFactory`] (from `flow-remote`) and the opt-in cloud STT backend
//! (D18) as a [`CloudFactory`] (from `flow-cloud`), so a client-only build
//! links this crate without any network code.

pub mod cloud;
mod gate;
mod local;
pub mod meeting;
pub mod migrate;
pub mod net;
mod router;
mod server_clean;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use cloud::{
    month_of, CleanupConfig, CleanupProvider, CleanupRoute, CloudConfig, CloudProviderConfig,
    CloudUsage, CustomCleanupConfig, OpenRouterConfig, Privacy, ProviderUsage, Route, SecretString,
    SttConfig, Usd,
};
pub use flow_core::cleanup::{CleanUsage, PromptScope};
pub use flow_core::config::{FieldContext, SessionConfig};
pub use flow_core::learn::Candidate;
pub use flow_core::{CommitResult, CommitStatus, RouteInfo, SessionPoll};
pub use local::{install_cleanup_plan, LocalLoader};
pub use migrate::{MigrationReport, RemovedProvider};
pub use router::Router;
pub use server_clean::Clock;

/// Default `ServerConfig::connect_timeout_ms` (D24). Was 800 ms, too tight
/// for a phone reaching the server through a VPN relay (TCP +
/// WS upgrade + hello = 3+ round trips at 150-300 ms each).
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 2500;
/// The pre-D24 default; shells migrate a stored value equal to it.
pub const LEGACY_CONNECT_TIMEOUT_MS: u64 = 800;
/// Adaptive connect timeout: `max(configured, clamp(rtt_median × K, MIN, MAX))`
/// once a probe has measured the round trip (D24).
pub const ADAPTIVE_RTT_FACTOR: u64 = 6;
pub const ADAPTIVE_MIN_MS: u64 = 1500;
pub const ADAPTIVE_MAX_MS: u64 = 10_000;

/// The connect timeout the router gives the remote backend: the configured
/// one, raised to `rtt_median × 6` (1.5-10 s) when the last probe measured a
/// slow path. Never lowers the configured value. Additive (D24).
pub fn effective_connect_timeout_ms(configured: u64, rtt_median_ms: Option<u32>) -> u64 {
    match rtt_median_ms {
        Some(m) => {
            configured.max((m as u64 * ADAPTIVE_RTT_FACTOR).clamp(ADAPTIVE_MIN_MS, ADAPTIVE_MAX_MS))
        }
        None => configured,
    }
}

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
    /// Opt-in OpenRouter text cleanup (D28), separate from `allow_cloud`
    /// and with its own consent (`cloud.openrouter.cleanup.consent_*`).
    /// Used only when `cleanup_order` lists `cloud`. Omitted while false.
    #[serde(skip_serializing_if = "is_false")]
    pub allow_cloud_cleanup: bool,
    /// Enabled cleanup engines in the order they are tried (D28, D36):
    /// each dictation is cleaned by the first that can (on-device LLM
    /// loaded; server reachable with an LLM; cloud allowed, consented,
    /// keyed, in budget), falling back down the list on a failure or
    /// timeout, whatever route transcribed it. An engine not listed never
    /// cleans. `cloud` also needs `allow_cloud_cleanup`. Empty = pre-D36
    /// (reported as `[local, server]`): on-device dictations use the
    /// on-device LLM only, server dictations the server's own; no
    /// text-only server cleanup.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cleanup_order: Vec<CleanupRoute>,
    /// Entries of `cleanup_order` the user switched off (D36): they keep
    /// their place in the list but never clean. Lets a shell send the whole
    /// list, so "everything off" is not the empty (pre-D36) order.
    /// Omitted while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cleanup_off: Vec<CleanupRoute>,
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
            allow_cloud_cleanup: false,
            cleanup_order: Vec::new(),
            cleanup_off: Vec::new(),
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
            connect_timeout_ms: DEFAULT_CONNECT_TIMEOUT_MS,
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
        for (i, r) in self.cleanup_order.iter().enumerate() {
            if self.cleanup_order[..i].contains(r) {
                return Err(RouterError::BadConfig(format!(
                    "cleanup order lists {:?} twice",
                    r
                )));
            }
        }
        Ok(())
    }

    /// Cloud cleanup is switched on by the user (toggle + order). Build
    /// capability, consent, key, budget and circuit are checked per call.
    pub fn cloud_cleanup_wanted(&self) -> bool {
        self.allow_cloud_cleanup
            && self
                .effective_cleanup_order()
                .contains(&CleanupRoute::Cloud)
    }

    /// The engines that may clean, in order (D36): `cleanup_order` minus
    /// `cleanup_off`, with the empty (pre-D36) order spelled out.
    pub fn effective_cleanup_order(&self) -> Vec<CleanupRoute> {
        if self.cleanup_order.is_empty() {
            vec![CleanupRoute::Local, CleanupRoute::Server]
        } else {
            self.cleanup_order
                .iter()
                .copied()
                .filter(|r| !self.cleanup_off.contains(r))
                .collect()
        }
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
    /// D28: OpenRouter found no provider meeting the privacy settings
    /// (HTTP 503 with `zdr`/`data_collection`). Sticky until the model or
    /// privacy settings change.
    NoPrivateRoute,
    /// D28: cloud cleanup skipped (no build support, toggle off, no
    /// cleanup consent, or no key).
    CloudCleanupNotConfigured,
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
            Failure::NoPrivateRoute => "no_private_route",
            Failure::CloudCleanupNotConfigured => "cloud_cleanup_not_configured",
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
    /// The session fell back to the on-device engine, which is being loaded
    /// now (D24): show "Loading phone model…". Audio is buffered meanwhile.
    /// Omitted while false. Additive.
    #[serde(skip_serializing_if = "is_false")]
    pub loading: bool,
}

/// How the last session was routed, for the debug log (D21/D24). JSON:
/// every field omitted while unset/false. Additive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteDiag {
    /// Wall time `start_session` spent trying the server (connect + hello
    /// + start); `None` when the server was not tried (off, circuit open).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_ms: Option<u32>,
    /// The on-device engine was loaded on demand for this session.
    #[serde(skip_serializing_if = "is_false")]
    pub local_cold: bool,
    /// How long that load took.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_load_ms: Option<u32>,
    /// Why that load failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_load_error: Option<String>,
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
    /// The on-device engine is being loaded for this session (D24).
    /// Omitted while false. Additive.
    #[serde(skip_serializing_if = "is_false")]
    pub loading: bool,
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

/// One dictation route as a start would see it now ([`Router::route_checks`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCheck {
    pub kind: EngineKind,
    /// Why a start would skip this route without trying it; `None` = it
    /// would be tried (a server may still fail to answer).
    pub skip: Option<BackendError>,
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
    /// Routing details of the last committed session (D24). Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_diag: Option<RouteDiag>,
    /// Cloud cleanup circuit (D28); present only in builds with cloud
    /// cleanup support. Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_circuit: Option<CircuitStatus>,
}

/// `Router::cleanup_status` (D28 settings page "Text cleanup" card).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanupStatus {
    /// This build can clean up text with OpenRouter.
    pub supported: bool,
    /// Toggle on and `cloud` in `cleanup_order`.
    pub enabled: bool,
    /// The effective cleanup order (D36: never empty).
    pub order: Vec<CleanupRoute>,
    /// D36: whether the configured server can clean text (`clean` in its
    /// `hello_ok.features`); `None` = not known yet (no connection) or no
    /// server configured. Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_llm: Option<bool>,
    /// D36: the server cleanup circuit, when `server` is in the order and a
    /// server is configured. Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_circuit: Option<CircuitStatus>,
    /// D52: `openrouter` or `custom` (the service behind `cloud`). Additive.
    pub provider: crate::cloud::CleanupProvider,
    /// D52: the custom server's host (consent, status line); `""` for
    /// OpenRouter. Additive.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// Cleanup consent recorded for the selected provider's host.
    pub consent: bool,
    /// Why a call now would be refused (`Failure` string), `None` = ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub circuit: Option<CircuitStatus>,
    pub model: String,
    pub timeout_ms: u64,
    /// The last cloud cleanup call (engine, model, cost, ids, TTFT).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<flow_core::cleanup::CleanUsage>,
    /// This month's cleanup cost (USD), both providers.
    pub month_cost_usd: Usd,
    /// D52: the part of it the custom server reported (its cost header);
    /// never counted against the OpenRouter budget.
    #[serde(default)]
    pub custom_month_cost_usd: Usd,
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
    /// Open (or keep) a warm connection without a session, so the next
    /// `start_session` skips the handshake (remote; D24 pre-warm). Blocking;
    /// the router calls it on a helper thread. Default: nothing to warm.
    fn warm(&self) -> Result<(), BackendError> {
        Ok(())
    }
    /// Install (or with `None` remove) the router's cleanup plan (D28):
    /// an in-process engine calls `plan(its built-in cleaner)` and uses
    /// the returned cleaner for its next commits
    /// (`FlowCore::set_cleaner`). Default: ignored, which is what the
    /// server route does (its own pipeline cleans; never cloud cleanup).
    fn install_cleanup(&self, _plan: Option<CleanupPlan>) {}
    /// The on-device LLM this engine was built with (`FlowCore::cleaner`),
    /// for one-off requests outside a dictation (Command Mode,
    /// [`Router::command`]). Default: none (remote / cloud STT backends).
    /// Additive.
    fn llm_cleaner(&self) -> Option<Arc<dyn flow_core::cleanup::Cleaner>> {
        None
    }
    /// Load 3: hold this engine's on-device LLM for the next local load
    /// (`FlowCore::keep_llm`); whether there was one. Default: none.
    /// Additive.
    fn keep_llm(&self) -> bool {
        false
    }
    /// D36 (remote): the server's `hello_ok` from the current or last
    /// connection, `None` before any. Default: none.
    fn server_hello(&self) -> Option<flow_protocol::HelloOk> {
        None
    }
    /// D36 (remote): text-only cleanup on the server (`clean` → `cleaned`),
    /// blocking until the answer or `deadline`. Default: unsupported.
    fn clean_text(
        &self,
        _req: &flow_core::cleanup::CleanRequest,
        _deadline: std::time::Instant,
    ) -> Result<flow_protocol::Cleaned, BackendError> {
        Err(BackendError::new(
            Failure::ServerNotConfigured,
            "this backend cannot clean text",
        ))
    }
}

/// Builds the cleaner an in-process engine should use from its built-in
/// one (the on-device LLM, if loaded): `RoutedCleaner(cloud, local)` etc.
pub type CleanupPlan = Arc<
    dyn Fn(
            Option<Arc<dyn flow_core::cleanup::Cleaner>>,
        ) -> Option<Arc<dyn flow_core::cleanup::Cleaner>>
        + Send
        + Sync,
>;

pub type RemoteFactory = Box<dyn Fn(&ServerConfig) -> Arc<dyn Backend> + Send + Sync>;

/// Builds the cloud STT backend for the OpenRouter config + key
/// (`flow-cloud`). Called at most once per (privacy, stt config, key) and
/// cached. D28: was `&CloudProviderConfig`.
pub type CloudFactory =
    Box<dyn Fn(&OpenRouterConfig, SecretString) -> Arc<dyn Backend> + Send + Sync>;

/// Builds the OpenRouter cleaner for the config + key (`flow-cloud`'s
/// `OpenRouterCleaner`, D28). Called once per (privacy, cleanup config,
/// key) and cached. The router wraps it with consent / circuit / budget
/// checks; it never sees a call the gate refused.
pub type CleanupFactory = Box<
    dyn Fn(&OpenRouterConfig, SecretString) -> Arc<dyn flow_core::cleanup::Cleaner> + Send + Sync,
>;

/// Builds the custom-server cleaner (D52, `flow-cloud`'s `CustomCleaner`)
/// for the server config, the shared prompt scope and the optional key.
/// Cached like [`CleanupFactory`] and wrapped by the same gate.
pub type CustomCleanupFactory = Box<
    dyn Fn(
            &crate::cloud::CustomCleanupConfig,
            PromptScope,
            Option<SecretString>,
        ) -> Arc<dyn flow_core::cleanup::Cleaner>
        + Send
        + Sync,
>;

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
