//! flow-server: runs the unmodified flow-core pipeline for hybrid clients
//! over WebSocket (D17; `docs/protocol.md`, `docs/hybrid-architecture.md` §3).
//!
//! Privacy: audio, transcripts, contexts and dictionaries live only in memory
//! for the duration of a session. Nothing is written to disk; logs carry
//! sizes and timings only (unless `log_text = true`). The server only
//! listens; it never opens an outbound connection.

pub mod config;
mod health;
pub mod pool;
mod ws;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::watch;

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
    pub token: Option<String>,
    pub cfg: Config,
    pub hooks: Arc<Hooks>,
    pub kill: watch::Receiver<bool>,
    pub connections: AtomicU64,
    pub sessions: AtomicU64,
    pub cancels: AtomicU64,
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

/// Serve on already-bound listeners with an already-loaded pool.
pub async fn serve_with_pool(
    cfg: Config,
    pool: Arc<Pool>,
    listeners: Vec<TcpListener>,
) -> Result<ServerHandle, String> {
    let token = cfg.resolve_token()?;
    let tls = acceptor(&cfg)?;
    if cfg.log.log_text {
        tracing::warn!("log_text = true: transcripts will be written to the log at debug level");
    }
    let (kill_tx, kill_rx) = watch::channel(false);
    let state = Arc::new(AppState {
        pool,
        token,
        cfg,
        hooks: Arc::new(Hooks::default()),
        kill: kill_rx,
        connections: AtomicU64::new(0),
        sessions: AtomicU64::new(0),
        cancels: AtomicU64::new(0),
    });
    let app = Router::new()
        .route(flow_protocol::PATH_STREAM, get(stream_handler))
        .route(flow_protocol::PATH_HEALTH, get(health::handler))
        .with_state(state.clone());
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
    cfg.resolve_token()?;
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
        let _ = tcp.set_nodelay(true);
        state.connections.fetch_add(1, Ordering::SeqCst);
        let svc = TowerToHyperService::new(app.clone().layer(Extension(ConnectInfo(peer))));
        let mut kill = state.kill.clone();
        let tls = tls.clone();
        tokio::spawn(async move {
            let builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
            #[cfg(feature = "tls")]
            if let Some(acc) = tls {
                let Ok(s) = acc.accept(tcp).await else { return };
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(s), svc);
                tokio::select! { _ = conn => {}, _ = kill.changed() => {} }
                return;
            }
            #[cfg(not(feature = "tls"))]
            let _ = tls;
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(tcp), svc);
            tokio::select! { _ = conn => {}, _ = kill.changed() => {} }
        });
    }
}

/// Constant-time bearer check.
pub(crate) fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(want) = &state.token else {
        return true;
    };
    let got = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let (a, b) = (got.as_bytes(), want.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for (i, bi) in b.iter().enumerate() {
        diff |= a.get(i).copied().unwrap_or(0) ^ bi;
    }
    diff == 0 && a.len() == b.len()
}

async fn stream_handler(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorized(&state, &headers) {
        tracing::info!(%peer, "rejected: bad or missing token");
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.protocols([flow_protocol::SUBPROTOCOL])
        .max_message_size(flow_protocol::MAX_FRAME_BYTES * 2)
        .on_upgrade(move |socket| ws::run(socket, state, peer))
}
