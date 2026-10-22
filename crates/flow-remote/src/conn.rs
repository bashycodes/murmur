//! One WebSocket connection: handshake, reader task, writer task, and the
//! shared state the sync `Backend` methods wait on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use flow_core::CommitResult;
use flow_protocol::{
    code, ClientMsg, ErrorMsg, Hello, HelloOk, ServerMsg, ServerTimings, CLOSE_PROTO_MISMATCH,
    CLOSE_UNAUTHORIZED, IDLE_KEEPALIVE_S, PATH_STREAM, PROTO_MAX, PROTO_MIN, SUBPROTOCOL,
    WS_DEAD_S, WS_PING_S,
};
use flow_router::{net, BackendError, Failure, ServerConfig};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    HeaderValue, AUTHORIZATION, SEC_WEBSOCKET_PROTOCOL,
};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::WebSocketStream;

use crate::{failure_for_code, WRITER_QUEUE};

pub enum Outgoing {
    Text(String),
    Audio(Vec<u8>),
    Close,
}

#[derive(Default)]
struct State {
    sid: Option<u64>,
    started_at: Option<Instant>,
    /// `Some(queued)` once `started` arrived.
    started: Option<bool>,
    partial: Option<String>,
    result: Option<(CommitResult, ServerTimings)>,
    error: Option<ErrorMsg>,
    close_reason: Option<String>,
    pongs: Vec<u64>,
    last_busy: Option<u32>,
}

pub struct Conn {
    tx: mpsc::Sender<Outgoing>,
    st: Mutex<State>,
    cv: Condvar,
    closed: AtomicBool,
    hello: HelloOk,
    rtt_ms: u32,
    last_used: Mutex<Instant>,
}

fn connect_failed(m: impl std::fmt::Display) -> BackendError {
    BackendError::new(Failure::ConnectFailed, m.to_string())
}

/// TCP (+TLS) + upgrade + hello/hello_ok. The caller bounds it with
/// `connect_timeout_ms`.
pub async fn connect(cfg: &ServerConfig, client: &str) -> Result<Arc<Conn>, BackendError> {
    let host = net::bare_host(&cfg.host).to_string();
    // Resolve first; plain ws:// only if *every* address is private /
    // loopback / link-local / tailnet-via-tailnet (review S6), and then
    // connect to exactly those addresses (no second lookup to rebind).
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), cfg.port))
        .await
        .map_err(|e| connect_failed(format!("{}: cannot resolve: {e}", cfg.addr())))?
        .collect();
    if !cfg.tls {
        let ips: Vec<std::net::IpAddr> = addrs.iter().map(|a| a.ip()).collect();
        net::plaintext_ok_resolved(&ips, &net::route_source)
            .map_err(|e| connect_failed(format!("{}: plaintext refused: {e}", cfg.addr())))?;
    }
    let tcp = tokio::net::TcpStream::connect(&addrs[..])
        .await
        .map_err(|e| connect_failed(format!("{}: {e}", cfg.addr())))?;
    let _ = tcp.set_nodelay(true);
    let scheme = if cfg.tls { "wss" } else { "ws" };
    let mut req = format!("{scheme}://{}{PATH_STREAM}", cfg.addr())
        .into_client_request()
        .map_err(connect_failed)?;
    let h = req.headers_mut();
    h.insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(SUBPROTOCOL),
    );
    if !cfg.token.is_empty() {
        let v = HeaderValue::from_str(&format!("Bearer {}", cfg.token))
            .map_err(|_| BackendError::new(Failure::AuthFailed, "token has invalid characters"))?;
        h.insert(AUTHORIZATION, v);
    }
    #[cfg(feature = "tls")]
    if cfg.tls {
        let s = crate::tls::connect(cfg, &host, tcp).await?;
        return handshake(tokio_tungstenite::client_async(req, s).await, client).await;
    }
    handshake(tokio_tungstenite::client_async(req, tcp).await, client).await
}

type Upgrade<S> = Result<
    (
        WebSocketStream<S>,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    WsError,
>;

async fn handshake<S>(up: Upgrade<S>, client: &str) -> Result<Arc<Conn>, BackendError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws, _resp) = up.map_err(|e| match e {
        WsError::Http(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
            BackendError::new(
                Failure::AuthFailed,
                format!("server rejected the token (HTTP {})", r.status().as_u16()),
            )
        }
        e => connect_failed(e),
    })?;
    let hello = ClientMsg::Hello(Hello {
        proto_min: PROTO_MIN,
        proto_max: PROTO_MAX,
        client: client.into(),
        ..Default::default()
    });
    let t0 = Instant::now();
    ws.send(Message::Text(hello.to_json().into()))
        .await
        .map_err(connect_failed)?;
    let hello_ok = loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<ServerMsg>(&t) {
                Ok(ServerMsg::HelloOk(h)) => break h,
                Ok(ServerMsg::Error(e)) => {
                    return Err(BackendError::new(failure_for_code(&e.code), e.message));
                }
                _ => return Err(connect_failed("unexpected reply to hello")),
            },
            Some(Ok(Message::Close(Some(f)))) => {
                let kind = match f.code {
                    CloseCode::Library(CLOSE_UNAUTHORIZED) => Failure::AuthFailed,
                    CloseCode::Library(CLOSE_PROTO_MISMATCH) => Failure::ProtoMismatch,
                    _ => Failure::ConnectFailed,
                };
                return Err(BackendError::new(kind, format!("closed: {}", f.reason)));
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(_)) => return Err(connect_failed("unexpected frame before hello_ok")),
            Some(Err(e)) => return Err(connect_failed(e)),
            None => return Err(connect_failed("closed during hello")),
        }
    };
    let rtt_ms = t0.elapsed().as_millis() as u32;
    let (tx, rx) = mpsc::channel(WRITER_QUEUE);
    let conn = Arc::new(Conn {
        tx,
        st: Mutex::new(State::default()),
        cv: Condvar::new(),
        closed: AtomicBool::new(false),
        hello: hello_ok,
        rtt_ms,
        last_used: Mutex::new(Instant::now()),
    });
    let (sink, stream) = ws.split();
    tokio::spawn(reader(conn.clone(), stream));
    tokio::spawn(writer(conn.clone(), sink, rx));
    Ok(conn)
}

async fn reader<S>(
    conn: Arc<Conn>,
    mut stream: futures_util::stream::SplitStream<WebSocketStream<S>>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let reason = loop {
        match tokio::time::timeout(Duration::from_secs(WS_DEAD_S), stream.next()).await {
            Err(_) => break format!("no frame for {WS_DEAD_S} s"),
            Ok(None) => break "connection closed".to_string(),
            Ok(Some(Err(e))) => break format!("connection error: {e}"),
            Ok(Some(Ok(Message::Close(f)))) => {
                break f.map_or("closed by server".into(), |f| {
                    format!("closed by server: {} {}", u16::from(f.code), f.reason)
                })
            }
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(m) = serde_json::from_str::<ServerMsg>(&t) {
                    conn.on_message(m);
                }
            }
            Ok(Some(Ok(_))) => {}
        }
    };
    conn.mark_closed(reason);
}

async fn writer<S>(
    conn: Arc<Conn>,
    mut sink: futures_util::stream::SplitSink<WebSocketStream<S>, Message>,
    mut rx: mpsc::Receiver<Outgoing>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut ping = tokio::time::interval(Duration::from_secs(WS_PING_S));
    ping.tick().await;
    loop {
        let r = tokio::select! {
            m = rx.recv() => match m {
                Some(Outgoing::Text(t)) => sink.send(Message::Text(t.into())).await,
                Some(Outgoing::Audio(b)) => sink.send(Message::Binary(b.into())).await,
                Some(Outgoing::Close) | None => {
                    let _ = sink.close().await;
                    break;
                }
            },
            _ = ping.tick() => {
                if conn.idle_expired() {
                    let _ = sink.close().await;
                    conn.mark_closed("idle".into());
                    break;
                }
                sink.send(Message::Ping(Vec::new().into())).await
            }
        };
        if let Err(e) = r {
            conn.mark_closed(format!("send failed: {e}"));
            break;
        }
    }
}

impl Conn {
    pub fn hello(&self) -> &HelloOk {
        &self.hello
    }

    pub fn rtt_ms(&self) -> u32 {
        self.rtt_ms
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    fn idle_expired(&self) -> bool {
        self.st.lock().unwrap().sid.is_none()
            && self.last_used.lock().unwrap().elapsed() >= Duration::from_secs(IDLE_KEEPALIVE_S)
    }

    /// Open, idle and inside the keep-alive window.
    pub fn usable(&self) -> bool {
        !self.is_closed() && !self.idle_expired()
    }

    pub fn try_send(&self, m: Outgoing) -> bool {
        !self.is_closed() && self.tx.try_send(m).is_ok()
    }

    pub fn close(&self) {
        let _ = self.tx.try_send(Outgoing::Close);
        self.mark_closed("closed by client".into());
    }

    fn mark_closed(&self, reason: String) {
        self.closed.store(true, Ordering::SeqCst);
        let mut st = self.st.lock().unwrap();
        st.close_reason.get_or_insert(reason);
        self.cv.notify_all();
    }

    pub fn begin_session(&self, sid: u64) {
        let mut st = self.st.lock().unwrap();
        let pongs = std::mem::take(&mut st.pongs);
        let close_reason = st.close_reason.take();
        let last_busy = st.last_busy;
        *st = State {
            sid: Some(sid),
            started_at: Some(Instant::now()),
            pongs,
            close_reason,
            last_busy,
            ..Default::default()
        };
        drop(st);
        self.touch();
    }

    /// Also wakes a `wait_result` for the old session (it returns).
    pub fn end_session(&self) {
        self.st.lock().unwrap().sid = None;
        self.cv.notify_all();
        self.touch();
    }

    fn on_message(&self, m: ServerMsg) {
        let mut st = self.st.lock().unwrap();
        let cur = st.sid;
        match m {
            ServerMsg::Started { sid, queued } if Some(sid) == cur => st.started = Some(queued),
            ServerMsg::Partial { sid, text } if Some(sid) == cur => st.partial = Some(text),
            ServerMsg::Result {
                sid,
                result,
                server_timings,
            } if Some(sid) == cur => st.result = Some((*result, server_timings)),
            ServerMsg::Error(e) if e.sid.is_none() || e.sid == cur => {
                if e.fatal {
                    st.close_reason
                        .get_or_insert(format!("{}: {}", e.code, e.message));
                }
                st.error = Some(e);
            }
            ServerMsg::Pong { n, busy } => {
                st.pongs.push(n);
                st.last_busy = Some(busy);
            }
            _ => {}
        }
        self.cv.notify_all();
    }

    pub fn partial(&self, sid: u64) -> Option<String> {
        let st = self.st.lock().unwrap();
        (st.sid == Some(sid)).then(|| st.partial.clone()).flatten()
    }

    pub fn last_busy(&self) -> Option<u32> {
        self.st.lock().unwrap().last_busy
    }

    /// Errors that end the current session.
    fn session_error(st: &State) -> Option<BackendError> {
        let e = st.error.as_ref()?;
        match e.code.as_str() {
            code::BUSY => Some(BackendError::new(Failure::Busy, e.message.clone())),
            code::NO_SESSION | code::STALE_SID | code::SERVER_ERROR => Some(BackendError::new(
                Failure::Disconnected,
                format!("{}: {}", e.code, e.message),
            )),
            _ if e.fatal => Some(BackendError::new(
                failure_for_code(&e.code),
                e.message.clone(),
            )),
            _ => None,
        }
    }

    pub fn session_failure(&self, sid: u64, started_within: Duration) -> Option<BackendError> {
        let st = self.st.lock().unwrap();
        if st.sid != Some(sid) {
            return None;
        }
        if st.result.is_some() {
            return None;
        }
        if self.is_closed() {
            let why = st
                .close_reason
                .clone()
                .unwrap_or_else(|| "connection closed".into());
            return Some(BackendError::new(Failure::Disconnected, why));
        }
        if let Some(e) = Self::session_error(&st) {
            return Some(e);
        }
        match st.started {
            Some(true) => Some(BackendError::new(
                Failure::Busy,
                "server queued the session",
            )),
            Some(false) => None,
            None => st
                .started_at
                .filter(|t| t.elapsed() > started_within)
                .map(|_| BackendError::new(Failure::Disconnected, "no `started` from the server")),
        }
    }

    pub fn wait_result(
        &self,
        sid: u64,
        deadline: Instant,
    ) -> Result<(CommitResult, ServerTimings), BackendError> {
        let mut st = self.st.lock().unwrap();
        loop {
            if st.sid != Some(sid) {
                return Err(BackendError::new(Failure::Error, "session replaced"));
            }
            if let Some(r) = st.result.take() {
                return Ok(r);
            }
            if self.is_closed() {
                let why = st
                    .close_reason
                    .clone()
                    .unwrap_or_else(|| "connection closed".into());
                return Err(BackendError::new(Failure::Disconnected, why));
            }
            if let Some(e) = Self::session_error(&st) {
                return Err(e);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(BackendError::new(
                    Failure::Timeout,
                    "no result before the deadline",
                ));
            }
            st = self.cv.wait_timeout(st, deadline - now).unwrap().0;
        }
    }

    pub fn wait_pong(&self, n: u64, deadline: Instant) -> Option<()> {
        let mut st = self.st.lock().unwrap();
        loop {
            if st.pongs.contains(&n) {
                return Some(());
            }
            let now = Instant::now();
            if self.is_closed() || now >= deadline {
                return None;
            }
            st = self.cv.wait_timeout(st, deadline - now).unwrap().0;
        }
    }
}
