//! flow-server: runs the unmodified flow-core pipeline for hybrid clients
//! over WebSocket (D17; `docs/protocol.md`, `docs/hybrid-architecture.md` §3).
//!
//! Privacy: audio, transcripts, contexts and dictionaries live only in memory
//! for the duration of a session. None of it is written to disk; logs carry
//! sizes and timings only (unless `log_text = true`). The key store
//! (`keys.rs`, D38) writes key hashes and per-key counters, never text or
//! audio. The one opt-in exception is `[debug] capture` (`capture.rs`): received audio and the
//! result, local, mode 0600, pruned to 30 files / 72 h. The server only
//! listens; it never opens an outbound connection.

pub mod capture;
pub mod config;
mod health;
pub mod keys;
pub mod library;
pub mod limits;
pub mod logging;
pub mod net;
pub mod pool;
mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
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
    pub kill: watch::Receiver<bool>,
    pub connections: AtomicU64,
    pub sessions: AtomicU64,
    pub cancels: AtomicU64,
    /// S7: open-connection caps (global permits, per-IP counts).
    conn_slots: Arc<Semaphore>,
    per_ip: Mutex<HashMap<IpAddr, usize>>,
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
            tracing::warn!("no active keys: every request gets 401 until `flow-server keys add`");
        }
    }
    let wide = cfg.check_bind()?;
    if cfg.insecure_bind_any {
        let list: Vec<String> = wide.iter().map(|a| a.to_string()).collect();
        tracing::warn!(
            "WARNING: --insecure-bind-any: accepting peers from any address{}{}. Audio, transcripts and the bearer token may cross untrusted networks{}",
            if list.is_empty() { "" } else { " on " },
            list.join(", "),
            if cfg.tls.is_some() { "" } else { " in cleartext (no [tls])" },
        );
    }
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
    let cfg_max_conns = cfg.max_connections.max(1);
    let state = Arc::new(AppState {
        pool,
        keys,
        cfg,
        hooks: Arc::new(Hooks::default()),
        capture,
        library,
        kill: kill_rx,
        connections: AtomicU64::new(0),
        sessions: AtomicU64::new(0),
        cancels: AtomicU64::new(0),
        conn_slots: Arc::new(Semaphore::new(cfg_max_conns)),
        per_ip: Mutex::new(HashMap::new()),
    });
    let app = Router::new()
        .route(flow_protocol::PATH_STREAM, get(stream_handler))
        .route(flow_protocol::PATH_HEALTH, get(health::handler))
        .merge(library::routes())
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
        if !state.cfg.insecure_bind_any && !net::classify(peer.ip()).is_local() {
            tracing::info!(%peer, "rejected: peer outside loopback/LAN/VPN");
            continue;
        }
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

/// The bearer check (D38): `Ok(None)` when auth is off, `Ok(Some(key))`
/// for an active key, `Err(())` otherwise. The key store compares hashes
/// in constant time and re-reads `keys.json` when the CLI changed it.
pub(crate) fn authorize(state: &AppState, headers: &HeaderMap) -> Result<Option<keys::KeyRef>, ()> {
    let Some(store) = &state.keys else {
        return Ok(None);
    };
    let got = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if got.is_empty() {
        return Err(());
    }
    store.authenticate(got).map(Some).ok_or(())
}

async fn stream_handler(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    Extension(slot): Extension<Arc<ConnSlot>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Ok(key) = authorize(&state, &headers) else {
        tracing::info!(%peer, "rejected: bad or missing token");
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    };
    ws.protocols([flow_protocol::SUBPROTOCOL])
        .max_message_size(limits::MAX_WS_MESSAGE_BYTES)
        .max_frame_size(limits::MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let _slot = slot; // the WebSocket keeps its connection slot
            ws::run(socket, state, peer, key).await
        })
}
