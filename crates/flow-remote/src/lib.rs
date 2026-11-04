//! `RemoteBackend`: the `flow_router::Backend` that streams a session to a
//! self-hosted `flow-server` over WebSocket (protocol: `docs/protocol.md`).
//!
//! This crate is the only client-side network code. Client-only builds do
//! not link it; hybrid / server-only builds pass [`factory`] to
//! `flow_router::Router::new`.
//!
//! Threading: each backend owns a small tokio runtime (one worker). The
//! sync `Backend` methods talk to per-connection reader/writer tasks through
//! a bounded channel (writer, 64 frames) and a mutex + condvar (reader
//! state), so `push_audio` never blocks.

pub mod admin;
mod conn;
#[cfg(feature = "library")]
pub mod library;
#[cfg(feature = "tls")]
mod tls;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flow_core::config::{FieldContext, SessionConfig};
use flow_core::{CommitResult, RouteInfo};
use flow_protocol::{code, encode_pcm, ClientMsg, MAX_FRAME_BYTES};
use flow_router::{
    net, Backend, BackendError, EngineKind, Failure, HealthReport, RemoteFactory, Rtt,
    ServerConfig, StartInfo,
};

use conn::{Conn, Outgoing};

/// Writer queue bound: 64 × 100 ms frames ≈ 6.4 s of backlog.
pub const WRITER_QUEUE: usize = 64;

/// The factory the router needs: one `RemoteBackend` per server config.
/// `client` names this app in `hello` (e.g. `"flow-windows 0.2.0"`).
pub fn factory(client: &str) -> RemoteFactory {
    let client = client.to_string();
    Box::new(move |cfg: &ServerConfig| {
        Arc::new(RemoteBackend::new(cfg.clone(), &client)) as Arc<dyn Backend>
    })
}

pub struct RemoteBackend {
    inner: Arc<Inner>,
}

struct Session {
    sid: u64,
    conn: Arc<Conn>,
    /// Local failure (writer queue full, send error).
    failed: Option<BackendError>,
}

/// `(sid, connection, cancelled)` of the commit being awaited.
type Committing = (u64, Arc<Conn>, Arc<AtomicBool>);

struct Inner {
    cfg: ServerConfig,
    client: String,
    rt: Option<tokio::runtime::Runtime>,
    conn: Mutex<Option<Arc<Conn>>>,
    session: Mutex<Option<Session>>,
    /// The session whose commit is being awaited (`cancel` reaches it,
    /// review C4) and whether it was cancelled.
    committing: Mutex<Option<Committing>>,
    next_sid: AtomicU64,
    /// D36 `clean` request ids.
    next_clean: AtomicU64,
    /// The `hello_ok` of the last connection (D36 feature check).
    hello: Mutex<Option<flow_protocol::HelloOk>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(c) = self.conn.lock().unwrap().take() {
            c.close();
        }
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

impl RemoteBackend {
    pub fn new(cfg: ServerConfig, client: &str) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("flow-remote")
            .enable_all()
            .build()
            .expect("tokio runtime");
        Self {
            inner: Arc::new(Inner {
                cfg,
                client: client.into(),
                rt: Some(rt),
                conn: Mutex::new(None),
                session: Mutex::new(None),
                committing: Mutex::new(None),
                next_sid: AtomicU64::new(1),
                next_clean: AtomicU64::new(1),
                hello: Mutex::new(None),
            }),
        }
    }

    fn rt(&self) -> &tokio::runtime::Runtime {
        self.inner.rt.as_ref().expect("runtime alive")
    }

    fn connect_new(&self) -> Result<Arc<Conn>, BackendError> {
        check_address(&self.inner.cfg)?;
        let cfg = self.inner.cfg.clone();
        let client = self.inner.client.clone();
        let timeout = Duration::from_millis(cfg.connect_timeout_ms);
        let rt = self.rt();
        rt.block_on(async move {
            match tokio::time::timeout(timeout, conn::connect(&cfg, &client)).await {
                Ok(r) => r,
                Err(_) => Err(BackendError::new(
                    Failure::ConnectFailed,
                    format!("no handshake within {} ms", cfg.connect_timeout_ms),
                )),
            }
        })
    }

    /// The warm connection if it is still usable, else a new one.
    fn connection(&self) -> Result<Arc<Conn>, BackendError> {
        let mut g = self.inner.conn.lock().unwrap();
        if let Some(c) = g.as_ref() {
            if c.usable() {
                return Ok(c.clone());
            }
            c.close();
        }
        let c = self.connect_new()?;
        *self.inner.hello.lock().unwrap() = Some(c.hello().clone());
        *g = Some(c.clone());
        Ok(c)
    }

    fn send_ctl(conn: &Conn, m: &ClientMsg) -> bool {
        conn.try_send(Outgoing::Text(m.to_json()))
    }

    fn route(&self, conn: &Conn, queue_ms: Option<u32>) -> RouteInfo {
        RouteInfo {
            engine: EngineKind::Remote.as_str().into(),
            server: Some(self.inner.cfg.addr()),
            rtt_ms: Some(conn.rtt_ms()),
            queue_ms,
            server_core: Some(conn.hello().core.clone()),
            ..Default::default()
        }
    }
}

impl Backend for RemoteBackend {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }

    fn ready(&self) -> bool {
        self.inner.cfg.is_configured()
    }

    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.cancel();
        let start = |conn: &Arc<Conn>, sid: u64| {
            conn.begin_session(sid);
            Self::send_ctl(
                conn,
                &ClientMsg::Start {
                    sid,
                    config: cfg.clone(),
                },
            )
        };
        let sid = self.inner.next_sid.fetch_add(1, Ordering::Relaxed);
        let mut conn = self.connection()?;
        if !start(&conn, sid) {
            // The warm connection died between the check and the send.
            conn.close();
            *self.inner.conn.lock().unwrap() = None;
            conn = self.connection()?;
            if !start(&conn, sid) {
                return Err(BackendError::new(
                    Failure::ConnectFailed,
                    "connection closed at start",
                ));
            }
        }
        *self.inner.session.lock().unwrap() = Some(Session {
            sid,
            conn,
            failed: None,
        });
        // Pipelined: `started` is not awaited (§1.3); a queued/busy answer
        // shows up in `session_failure`.
        Ok(StartInfo::default())
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        let mut g = self.inner.session.lock().unwrap();
        let Some(s) = g.as_mut() else { return false };
        if s.failed.is_some() {
            return false;
        }
        for chunk in pcm.chunks(MAX_FRAME_BYTES / 2) {
            if !s.conn.try_send(Outgoing::Audio(encode_pcm(chunk))) {
                let why = if s.conn.is_closed() {
                    "connection closed"
                } else {
                    "writer queue full"
                };
                s.failed = Some(BackendError::new(Failure::Disconnected, why));
                return false;
            }
        }
        true
    }

    fn set_context(&self, ctx: &FieldContext) {
        let g = self.inner.session.lock().unwrap();
        if let Some(s) = g.as_ref() {
            Self::send_ctl(
                &s.conn,
                &ClientMsg::Context {
                    sid: s.sid,
                    ctx: ctx.clone(),
                },
            );
        }
    }

    fn partial(&self) -> Option<String> {
        let g = self.inner.session.lock().unwrap();
        let s = g.as_ref()?;
        s.conn.partial(s.sid)
    }

    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        let Some(s) = self.inner.session.lock().unwrap().take() else {
            return Err(BackendError::new(Failure::Error, "no active session"));
        };
        if let Some(e) = s.failed {
            return Err(e);
        }
        if !Self::send_ctl(&s.conn, &ClientMsg::Commit { sid: s.sid }) {
            return Err(BackendError::new(
                Failure::Disconnected,
                "connection closed at commit",
            ));
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *self.inner.committing.lock().unwrap() = Some((s.sid, s.conn.clone(), cancelled.clone()));
        let out = s.conn.wait_result(s.sid, deadline);
        *self.inner.committing.lock().unwrap() = None;
        s.conn.end_session();
        if cancelled.load(Ordering::SeqCst) {
            return Err(BackendError::new(Failure::Error, "cancelled"));
        }
        match out {
            Ok((mut r, t)) => {
                r.route = Some(self.route(&s.conn, Some(t.queue_ms.min(u32::MAX as u64) as u32)));
                Ok(r)
            }
            Err(e) => {
                if e.kind == Failure::Timeout {
                    // Free the server worker; the router falls back.
                    Self::send_ctl(&s.conn, &ClientMsg::Cancel { sid: s.sid });
                }
                Err(e)
            }
        }
    }

    /// Cancels the open session, or (none open) the commit being awaited:
    /// the server is sent `cancel` (frees its worker) and `commit` stops
    /// waiting at once with an error (review C4).
    fn cancel(&self) {
        if let Some(s) = self.inner.session.lock().unwrap().take() {
            Self::send_ctl(&s.conn, &ClientMsg::Cancel { sid: s.sid });
            s.conn.end_session();
        } else if let Some((sid, conn, cancelled)) = self.inner.committing.lock().unwrap().as_ref()
        {
            cancelled.store(true, Ordering::SeqCst);
            Self::send_ctl(conn, &ClientMsg::Cancel { sid: *sid });
            conn.end_session();
        }
    }

    fn probe(&self) -> HealthReport {
        let t0 = Instant::now();
        let conn = match self.connect_new() {
            Ok(c) => c,
            Err(e) => {
                let mut h = HealthReport::failed(EngineKind::Remote, &e);
                h.server = Some(self.inner.cfg.addr());
                return h;
            }
        };
        let deadline = t0 + Duration::from_millis(self.inner.cfg.connect_timeout_ms.max(500) * 3);
        let mut rtts = vec![conn.rtt_ms()];
        for n in 1..=3u64 {
            let sent = Instant::now();
            if !Self::send_ctl(&conn, &ClientMsg::Ping { n }) {
                break;
            }
            match conn.wait_pong(n, deadline) {
                Some(_) => rtts.push(sent.elapsed().as_millis() as u32),
                None => break,
            }
        }
        conn.close();
        let hello = conn.hello().clone();
        *self.inner.hello.lock().unwrap() = Some(hello.clone());
        let mut sorted = rtts.clone();
        sorted.sort_unstable();
        if rtts.len() < 4 {
            let e = BackendError::new(Failure::Timeout, "no pong from the server");
            let mut h = HealthReport::failed(EngineKind::Remote, &e);
            h.server = Some(self.inner.cfg.addr());
            return h;
        }
        HealthReport {
            ok: true,
            engine: EngineKind::Remote,
            server: Some(self.inner.cfg.addr()),
            rtt_ms: Some(Rtt {
                min: sorted[0],
                median: sorted[sorted.len() / 2],
                max: *sorted.last().unwrap(),
            }),
            server_version: Some(hello.server.clone()),
            server_core: Some(hello.core.clone()),
            postprocess_match: Some(hello.postprocess_rev == flow_core::POSTPROCESS_REV),
            postprocess_rev: Some(hello.postprocess_rev),
            proto: Some(hello.proto),
            asr_backend: Some(hello.asr_backend),
            asr_model: Some(hello.asr_model),
            llm: Some(hello.llm),
            workers: Some(hello.workers),
            busy: Some(conn.last_busy().unwrap_or(hello.busy)),
            checked_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            ..Default::default()
        }
    }

    /// D24 pre-warm: open (or keep) the warm connection, no session.
    fn warm(&self) -> Result<(), BackendError> {
        self.connection().map(|_| ())
    }

    fn server_hello(&self) -> Option<flow_protocol::HelloOk> {
        self.inner.hello.lock().unwrap().clone()
    }

    /// D36: on the warm connection (independent of a session on it).
    fn clean_text(
        &self,
        req: &flow_core::cleanup::CleanRequest,
        deadline: Instant,
    ) -> Result<flow_protocol::Cleaned, BackendError> {
        let conn = self.connection()?;
        if let Some(error) = clean_refusal(conn.hello(), req) {
            return Ok(flow_protocol::Cleaned {
                error: Some(error.into()),
                ..Default::default()
            });
        }
        let id = self.inner.next_clean.fetch_add(1, Ordering::Relaxed);
        conn.clean(id, req, deadline)
    }

    fn session_failure(&self) -> Option<BackendError> {
        let g = self.inner.session.lock().unwrap();
        let s = g.as_ref()?;
        if let Some(e) = &s.failed {
            return Some(e.clone());
        }
        s.conn.session_failure(
            s.sid,
            Duration::from_millis(self.inner.cfg.connect_timeout_ms),
        )
    }
}

/// Why a `clean` request is not sent to a server with this hello:
/// `no_llm` (no `clean` feature), `no_summary` (a meeting task, D53, to a
/// server without the `summary` feature, which would clean the text
/// instead), or `None` = send it.
pub(crate) fn clean_refusal(
    hello: &flow_protocol::HelloOk,
    req: &flow_core::cleanup::CleanRequest,
) -> Option<&'static str> {
    let has = |f: &str| hello.features.iter().any(|x| x == f);
    if !has(flow_protocol::FEATURE_CLEAN) {
        Some("no_llm")
    } else if !req.task.is_cleanup() && !has(flow_protocol::FEATURE_SUMMARY) {
        Some("no_summary")
    } else {
        None
    }
}

/// Refuse plaintext to public addresses before any socket exists (§2.6).
pub(crate) fn check_address(c: &ServerConfig) -> Result<(), BackendError> {
    if !c.is_configured() {
        return Err(BackendError::new(
            Failure::ServerNotConfigured,
            "no server host set",
        ));
    }
    if !c.tls && !net::classify(&c.host).plaintext_ok() {
        return Err(BackendError::new(
            Failure::ConnectFailed,
            "plaintext to public address (enable TLS)",
        ));
    }
    #[cfg(not(feature = "tls"))]
    if c.tls {
        return Err(BackendError::new(
            Failure::ConnectFailed,
            "TLS requested but this build has no TLS support",
        ));
    }
    Ok(())
}

/// Map a server `error` code to the router's failure kinds.
pub(crate) fn failure_for_code(c: &str) -> Failure {
    match c {
        code::BUSY => Failure::Busy,
        code::UNAUTHORIZED => Failure::AuthFailed,
        code::PROTO_MISMATCH => Failure::ProtoMismatch,
        _ => Failure::Disconnected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_core::cleanup::CleanRequest;
    use flow_core::cleanup::CleanTask;

    fn hello(features: &[&str]) -> flow_protocol::HelloOk {
        flow_protocol::HelloOk {
            features: features.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn meeting_tasks_need_the_summary_feature() {
        let clean = CleanRequest::default();
        let notes = CleanRequest {
            task: CleanTask::MeetingNotes,
            ..Default::default()
        };
        let (c, s) = (flow_protocol::FEATURE_CLEAN, flow_protocol::FEATURE_SUMMARY);
        assert_eq!(clean_refusal(&hello(&[]), &clean), Some("no_llm"));
        assert_eq!(clean_refusal(&hello(&[]), &notes), Some("no_llm"));
        // An older server: cleans, but would clean a meeting task too.
        assert_eq!(clean_refusal(&hello(&[c]), &clean), None);
        assert_eq!(clean_refusal(&hello(&[c]), &notes), Some("no_summary"));
        assert_eq!(clean_refusal(&hello(&[c, s]), &notes), None);
    }
}
