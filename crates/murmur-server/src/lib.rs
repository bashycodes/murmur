//! murmur-server: runs the unmodified murmur-core pipeline for hybrid clients
//! over WebSocket (D17; `docs/protocol.md`, `docs/hybrid-architecture.md` §3).
//!
//! Privacy: audio, transcripts, contexts and dictionaries live only in memory
//! for the duration of a session. None of it is written to disk; logs carry
//! sizes and timings only (unless `log_text = true`). The key store
//! (`keys.rs`, D38; managed over `admin.rs`, D39) writes key hashes and
//! per-key counters, never text or audio; the team dictionary
//! (`team_dict.rs`) writes the admin's shared word list. The one opt-in exception is `[debug] capture` (`capture.rs`): received audio and the
//! result, local, mode 0600, pruned to 30 files / 72 h. The server only
//! listens; it never opens an outbound connection.

mod admin;
pub mod auth_throttle;
pub mod capture;
pub mod config;
mod health;
pub mod keys;
pub mod library;
pub mod limits;
pub mod logging;
pub mod net;
pub mod pool;
pub mod team_dict;
mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

pub use config::Config;
pub use pool::Pool;

/// Test hooks (contract tests): simulate a busy or stalling server.
#[derive(Default)]
pub struct Hooks {
    /// Commits never get a `result`.
    pub stall_commits: AtomicBool,
}

pub(crate) struct AppState {
    pub pool: Arc<Pool>,
    /// Named keys (D38); `None` = auth off (`auth = "none"` on loopback).
    pub keys: Option<Arc<keys::KeyStore>>,
    pub cfg: Config,
    pub hooks: Arc<Hooks>,
    /// `[debug] capture`: where finished sessions are saved (None = off).
    pub capture: Option<Arc<capture::CaptureStore>>,
    /// The model library (D40); `None` without a library dir.
    pub library: Option<Arc<library::Library>>,
    /// The team dictionary; `None` = auth off (no keys, no team list).
    pub team_dict: Option<Arc<team_dict::TeamDictStore>>,
    /// `hello_ok.models` (D98), named once at start.
    pub models: murmur_protocol::ServerModels,
    pub kill: watch::Receiver<bool>,
    pub connections: AtomicU64,
    pub sessions: AtomicU64,
    pub cancels: AtomicU64,
    /// S7: open-connection caps (global permits, per-IP counts).
    conn_slots: Arc<Semaphore>,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
    /// Failed-auth lockouts per peer IP (D112).
    throttle: auth_throttle::AuthThrottle,
}

/// One open connection's share of the caps; released when the HTTP
/// connection and (after an upgrade) the WebSocket task both drop it.
pub(crate) struct ConnSlot {
    state: Arc<AppState>,
    ip: IpAddr,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        let mut m = self.state.per_ip.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = m.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.ip);
            }
        }
    }
}

impl AppState {
    fn take_slot(self: &Arc<Self>, ip: IpAddr) -> Option<ConnSlot> {
        let permit = self.conn_slots.clone().try_acquire_owned().ok()?;
        let mut m = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
        let n = m.entry(ip).or_insert(0);
        if *n >= self.cfg.max_connections_per_ip {
            return None;
        }
        *n += 1;
        Some(ConnSlot {
            state: self.clone(),
            ip,
            _permit: permit,
        })
    }
}

/// A running server. Dropping it does not stop it; call [`ServerHandle::kill`].
pub struct ServerHandle {
    pub addrs: Vec<SocketAddr>,
    state: Arc<AppState>,
    kill_tx: watch::Sender<bool>,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addrs[0]
    }
    /// Stop accepting and drop every connection at once (simulates a crash
    /// or `systemctl stop`).
    pub fn kill(&self) {
        let _ = self.kill_tx.send(true);
        self.flush_usage();
    }
    /// Write the per-key usage counters now (they are otherwise written
    /// every [`USAGE_FLUSH`] and at [`ServerHandle::kill`]).
    pub fn flush_usage(&self) {
        if let Some(k) = &self.state.keys {
            if let Err(e) = k.flush() {
                tracing::warn!("key usage: {e}");
            }
        }
    }
    /// The model library (D40), for tests and `library list`.
    pub fn library(&self) -> Option<&Arc<library::Library>> {
        self.state.library.as_ref()
    }
    /// The key store (`None` when auth is off).
    pub fn keys(&self) -> Option<&Arc<keys::KeyStore>> {
        self.state.keys.as_ref()
    }
    /// The team dictionary (`None` when auth is off).
    pub fn team_dictionary(&self) -> Option<&Arc<team_dict::TeamDictStore>> {
        self.state.team_dict.as_ref()
    }
    /// TCP connections accepted so far.
    pub fn connections(&self) -> u64 {
        self.state.connections.load(Ordering::SeqCst)
    }
    /// Sessions started so far.
    pub fn sessions(&self) -> u64 {
        self.state.sessions.load(Ordering::SeqCst)
    }
    /// `cancel` messages received so far.
    pub fn cancels(&self) -> u64 {
        self.state.cancels.load(Ordering::SeqCst)
    }
    /// Connections currently open (HTTP or upgraded WebSocket).
    pub fn open_connections(&self) -> usize {
        self.state.cfg.max_connections.max(1) - self.state.conn_slots.available_permits()
    }
    pub fn hooks(&self) -> &Hooks {
        &self.state.hooks
    }
    /// Pretend every worker is leased (new sessions queue).
    pub fn set_busy(&self, busy: bool) {
        self.state.pool.force_busy.store(busy, Ordering::SeqCst);
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.state.pool
    }
}

#[cfg(feature = "tls")]
type Acceptor = Option<tokio_rustls::TlsAcceptor>;
#[cfg(not(feature = "tls"))]
type Acceptor = Option<()>;

#[cfg(feature = "tls")]
fn acceptor(cfg: &Config) -> Result<Acceptor, String> {
    let Some(t) = &cfg.tls else { return Ok(None) };
    let read = |p: &std::path::Path| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let certs = rustls_pemfile::certs(&mut &read(&t.cert)?[..])
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("tls cert: {e}"))?;
    let key = rustls_pemfile::private_key(&mut &read(&t.key)?[..])
        .map_err(|e| format!("tls key: {e}"))?
        .ok_or("tls key: no private key in file")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let sc = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("tls: {e}"))?;
    Ok(Some(tokio_rustls::TlsAcceptor::from(Arc::new(sc))))
}

#[cfg(not(feature = "tls"))]
fn acceptor(cfg: &Config) -> Result<Acceptor, String> {
    match cfg.tls {
        Some(_) => Err("[tls] set but built without the `tls` feature".into()),
        None => Ok(None),
    }
}

/// How often changed per-key usage counters are written (D38).
pub const USAGE_FLUSH: Duration = Duration::from_secs(30);

/// `hello_ok.models` (D98): the cleanup model (when one is loaded), the
/// default speech model and each language's, named by [`library::model_name`].
fn server_models(
    m: &murmur_core::config::ModelConfig,
    llm: bool,
    lib: Option<&library::Library>,
) -> murmur_protocol::ServerModels {
    let dir = lib.map(library::Library::dir);
    murmur_protocol::ServerModels {
        cleanup: m
            .llm_path
            .as_deref()
            .filter(|_| llm)
            .and_then(|p| library::model_name(dir, p)),
        speech: library::model_name(dir, &m.asr_path),
        languages: m
            .languages
            .iter()
            .filter_map(|(k, l)| Some((k.clone(), library::model_name(dir, &l.asr_path)?)))
            .collect(),
    }
}

/// Serve on already-bound listeners with an already-loaded pool.
pub async fn serve_with_pool(
    cfg: Config,
    pool: Arc<Pool>,
    listeners: Vec<TcpListener>,
) -> Result<ServerHandle, String> {
    let token = cfg.resolve_token()?;
    let keys = cfg.open_keys()?.map(Arc::new);
    if let Some(k) = &keys {
        let path = k.keys_path();
        let n = k.active();
        let file = path.as_ref().map(|p| p.display().to_string());
        tracing::info!(
            keys = n,
            file = file.as_deref().unwrap_or("(memory)"),
            "auth: named keys"
        );
        if n == 0 {
            tracing::warn!("no active keys: every request gets 401 until `murmur-server keys add`");
        }
    }
    let wide = cfg.wide_binds()?;
    if !wide.is_empty() && cfg.tls.is_none() {
        let list: Vec<String> = wide.iter().map(|a| a.to_string()).collect();
        tracing::warn!(
            "bind {}: wildcard or public address, plain ws:// (no [tls]); on a machine reachable from the internet, set up [tls] or firewall the port",
            list.join(", "),
        );
    }
    // Next to keys.json, or in memory with an in-memory key store.
    let team_dict = keys.as_ref().map(|k| {
        let dir = k
            .keys_path()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        Arc::new(team_dict::TeamDictStore::open(dir.as_deref()))
    });
    let tls = acceptor(&cfg)?;
    if cfg.log.log_text {
        tracing::warn!("log_text = true: transcripts will be written to the log at debug level");
    }
    let capture = if cfg.debug.capture {
        let store = capture::CaptureStore::from_config(&cfg.debug, token.as_deref())?;
        tracing::warn!(
            dir = %store.dir().display(),
            max_files = cfg.debug.max_files,
            max_age_h = cfg.debug.max_age_h,
            "[debug] capture = true: received audio and transcripts are saved to disk"
        );
        let s = Arc::new(store);
        let p = s.clone();
        // Prune at start (off the async threads).
        tokio::task::spawn_blocking(move || {
            if let Err(e) = p.prune(std::time::SystemTime::now()) {
                tracing::warn!("debug capture prune: {e}");
            }
        });
        Some(s)
    } else {
        None
    };
    let library = library::Library::from_config(&cfg).map(Arc::new);
    let (kill_tx, kill_rx) = watch::channel(false);
    if let Some(l) = &library {
        let every = Duration::from_secs(cfg.library.rescan_s);
        library::spawn_rescan(l.clone(), every, kill_rx.clone());
    }
    let models = server_models(pool.model_config(), pool.llm, library.as_deref());
    let cfg_max_conns = cfg.max_connections.max(1);
    let throttle = auth_throttle::AuthThrottle::new(cfg.auth_throttle.clone());
    let state = Arc::new(AppState {
        pool,
        keys,
        cfg,
        hooks: Arc::new(Hooks::default()),
        capture,
        library,
        team_dict,
        models,
        kill: kill_rx,
        connections: AtomicU64::new(0),
        sessions: AtomicU64::new(0),
        cancels: AtomicU64::new(0),
        conn_slots: Arc::new(Semaphore::new(cfg_max_conns)),
        per_ip: Mutex::new(HashMap::new()),
        throttle,
    });
    let app = Router::new()
        .route(murmur_protocol::PATH_STREAM, get(stream_handler))
        .route(murmur_protocol::PATH_HEALTH, get(health::handler))
        .merge(library::routes())
        .merge(admin::routes())
        .merge(team_dict::routes())
        .with_state(state.clone());
    if let Some(k) = state.keys.clone() {
        let mut kill = state.kill.clone();
        tokio::spawn(async move {
            let mut t =
                tokio::time::interval_at(tokio::time::Instant::now() + USAGE_FLUSH, USAGE_FLUSH);
            loop {
                tokio::select! {
                    _ = t.tick() => {}
                    _ = kill.changed() => return,
                }
                let k = k.clone();
                let r = tokio::task::spawn_blocking(move || k.flush()).await;
                if let Ok(Err(e)) = r {
                    tracing::warn!("key usage: {e}");
                }
            }
        });
    }
    let mut addrs = Vec::new();
    for l in listeners {
        addrs.push(l.local_addr().map_err(|e| e.to_string())?);
        tokio::spawn(accept_loop(l, app.clone(), state.clone(), tls.clone()));
    }
    Ok(ServerHandle {
        addrs,
        state,
        kill_tx,
    })
}

/// Load models, then serve on the given listeners.
pub async fn serve(cfg: Config, listeners: Vec<TcpListener>) -> Result<ServerHandle, String> {
    cfg.check_auth()?;
    let (m, w, q) = (cfg.model.clone(), cfg.workers, cfg.max_queue);
    let pool = tokio::task::spawn_blocking(move || Pool::load(&m, w, q))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{} ({})", e, e.kind()))?;
    serve_with_pool(cfg, Arc::new(pool), listeners).await
}

async fn accept_loop(l: TcpListener, app: Router, state: Arc<AppState>, tls: Acceptor) {
    let mut kill = state.kill.clone();
    loop {
        let (tcp, peer) = tokio::select! {
            r = l.accept() => match r {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!("accept: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            _ = kill.changed() => return,
        };
        let Some(slot) = state.take_slot(peer.ip()) else {
            tracing::debug!(%peer, "rejected: connection limit");
            continue;
        };
        let _ = tcp.set_nodelay(true);
        state.connections.fetch_add(1, Ordering::SeqCst);
        let slot = Arc::new(slot);
        let svc = TowerToHyperService::new(
            app.clone()
                .layer(Extension(slot.clone()))
                .layer(Extension(ConnectInfo(peer))),
        );
        let mut kill = state.kill.clone();
        let tls = tls.clone();
        let handshake = Duration::from_millis(state.cfg.handshake_timeout_ms.max(1));
        tokio::spawn(async move {
            let _slot = slot;
            let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            // S7: every request head (the first, and any later one on an
            // idle keep-alive socket) must arrive within the handshake
            // timeout.
            builder
                .http1()
                .timer(TokioTimer::new())
                .header_read_timeout(handshake);
            #[cfg(feature = "tls")]
            if let Some(acc) = tls {
                let s = tokio::select! {
                    r = tokio::time::timeout(handshake, acc.accept(tcp)) => match r {
                        Ok(Ok(s)) => s,
                        _ => return,
                    },
                    _ = kill.changed() => return,
                };
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(s), svc);
                tokio::select! { _ = conn => {}, _ = kill.changed() => {} }
                return;
            }
            #[cfg(not(feature = "tls"))]
            let _ = tls;
            // hyper arms header_read_timeout only once the first byte is in;
            // a peer that sends nothing at all is dropped here.
            let mut byte = [0u8; 1];
            let first = tokio::select! {
                r = tokio::time::timeout(handshake, tcp.peek(&mut byte)) => r,
                _ = kill.changed() => return,
            };
            if !matches!(first, Ok(Ok(n)) if n > 0) {
                return;
            }
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(tcp), svc);
            tokio::select! { _ = conn => {}, _ = kill.changed() => {} }
        });
    }
}

/// Why [`authorize`] refused a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Denied {
    /// Missing, unknown or revoked key (401).
    Unauthorized,
    /// Too many failed attempts from this peer; retry after (429, D112).
    Throttled(Duration),
}

impl Denied {
    /// `Retry-After` in whole seconds (at least 1).
    pub(crate) fn retry_after_secs(d: Duration) -> u64 {
        (d.as_millis().div_ceil(1000) as u64).max(1)
    }
}

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        match self {
            Denied::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
            Denied::Throttled(d) => (
                StatusCode::TOO_MANY_REQUESTS,
                [(
                    axum::http::header::RETRY_AFTER,
                    Denied::retry_after_secs(d).to_string(),
                )],
                "too many failed attempts",
            )
                .into_response(),
        }
    }
}

/// The bearer check (D38) for a request from `peer`: `Ok(None)` when auth
/// is off, `Ok(Some(key))` for an active key, else why not. A peer locked
/// out for failed attempts (D112) is refused before its key is compared.
/// The key store compares hashes in constant time and re-reads
/// `keys.json` when the CLI changed it.
pub(crate) fn authorize(
    state: &AppState,
    peer: IpAddr,
    headers: &HeaderMap,
) -> Result<Option<keys::KeyRef>, Denied> {
    let Some(store) = &state.keys else {
        return Ok(None);
    };
    state.throttle.check(peer).map_err(Denied::Throttled)?;
    let got = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let key = if got.is_empty() {
        None
    } else {
        store.authenticate(got)
    };
    match key {
        Some(key) => {
            state.throttle.succeeded(peer);
            Ok(Some(key))
        }
        None => {
            state.throttle.failed(peer);
            Err(Denied::Unauthorized)
        }
    }
}

async fn stream_handler(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    Extension(slot): Extension<Arc<ConnSlot>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let key = match authorize(&state, peer.ip(), &headers) {
        Ok(key) => key,
        Err(d) => {
            tracing::info!(%peer, "rejected: {}", match d {
                Denied::Unauthorized => "bad or missing token",
                Denied::Throttled(_) => "locked out after failed attempts",
            });
            return d.into_response();
        }
    };
    ws.protocols([murmur_protocol::SUBPROTOCOL])
        .max_message_size(limits::MAX_WS_MESSAGE_BYTES)
        .max_frame_size(limits::MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let _slot = slot; // the WebSocket keeps its connection slot
            ws::run(socket, state, peer, key).await
        })
}
