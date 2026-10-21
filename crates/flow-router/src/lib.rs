//! flow-router: one §9 session API over a local `FlowCore` and an optional
//! remote `flow-server` backend (D17, `docs/hybrid-architecture.md` §1).
//!
//! Shells talk only to [`Router`]. The remote backend is injected as a
//! [`RemoteFactory`] (from `flow-remote`), so a client-only build links this
//! crate without any network code.

mod local;
pub mod net;
mod router;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use flow_core::config::{FieldContext, SessionConfig};
pub use flow_core::learn::Candidate;
pub use flow_core::{CommitResult, CommitStatus, RouteInfo};
pub use router::Router;

/// Which engine ran (or would run) a session. Non-exhaustive: an opt-in
/// third-party cloud STT route is planned (`RouteInfo::provider` then names
/// the provider); match with a `_` arm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineKind {
    #[default]
    Local,
    Remote,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::Local => "local",
            EngineKind::Remote => "remote",
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
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            allow_server: true,
            allow_client: true,
            prefer: Prefer::Server,
            server: ServerConfig::default(),
            keep_local_warm: true,
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
    /// primary). Today derived from `prefer` + the two toggles; a future
    /// third route (cloud STT) will extend this list rather than add flags.
    /// Build capability is not applied here (see `Router::capabilities`).
    pub fn route_order(&self) -> Vec<EngineKind> {
        let mut v = match self.prefer {
            Prefer::Server => vec![EngineKind::Remote, EngineKind::Local],
            Prefer::Client => vec![EngineKind::Local, EngineKind::Remote],
        };
        v.retain(|k| match k {
            EngineKind::Remote => self.allow_server,
            EngineKind::Local => self.allow_client,
        });
        v
    }

    /// Validate before `Router::update_config` / `flow_open_router`.
    pub fn validate(&self) -> Result<(), RouterError> {
        self.server.validate().map_err(RouterError::BadConfig)
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

/// Result of a successful `Backend::start_session`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartInfo {
    /// The server accepted the session but has no free worker yet.
    pub queued: bool,
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

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
