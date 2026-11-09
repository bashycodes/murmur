//! Per-connection task: hello, message loop, worker lease, partial poller.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use murmur_core::cleanup::CleanRequest;
use murmur_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use murmur_core::{CommitResult, MurmurCore};
use murmur_protocol::{
    code, decode_pcm, negotiate, samples_to_ms, Cleaned, ClientMsg, FrameError, ServerMsg,
    ServerTimings, CLOSE_PROTO_MISMATCH, PROTO_MAX, PROTO_MIN,
};
use tokio::sync::mpsc;

use crate::pool::{Lease, LeaseError};
use crate::AppState;

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PARTIAL_POLL: Duration = Duration::from_millis(100);
/// Outbound messages waiting for the socket (C17). A client that stops
/// reading fills it and is disconnected instead of growing server memory.
const OUT_QUEUE: usize = 256;

enum Event {
    Leased(u64, Result<Lease, LeaseError>),
    Result(u64, Box<CommitResult>),
    /// A text-only cleanup finished (D36).
    Cleaned(Cleaned),
}

struct Sess {
    sid: u64,
    cfg: SessionConfig,
    lease: Option<Lease>,
    /// Audio received while queued (replayed when a worker frees).
    queued_audio: Vec<i16>,
    ctx: Option<FieldContext>,
    samples: u64,
    created: Instant,
    /// Wall clock at `start` (the capture's `ts` / WAV name).
    started: SystemTime,
    /// `[debug] capture`: every sample received, for the WAV.
    captured: Option<Vec<i16>>,
    queue_ms: u64,
    /// `start` → `commit` arrival on this connection. Against the client's
    /// recording length it shows an upload backlog (commit queued behind
    /// audio on a slow / fresh link). 2026-10-24.
    start_to_commit_ms: Option<u64>,
    commit_requested: bool,
    committing: bool,
    last_partial: Option<String>,
    /// The task waiting for a worker while queued (aborted with the session).
    waiter: Option<tokio::task::AbortHandle>,
    /// The core running this session's commit, reachable until the job
    /// ends so a cancel or disconnect can abort it (C4).
    inflight: Arc<Mutex<Option<Arc<MurmurCore>>>>,
}

impl Sess {
    /// Ask the core to abort a running commit. The lock is what the commit
    /// task takes before returning the worker, so this can never reach a
    /// core that has been leased to another session.
    fn abort_commit(&self) {
        if let Some(core) = self
            .inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            core.cancel();
        }
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        if let Some(w) = self.waiter.take() {
            w.abort();
        }
        // Cancel, a replacing start, a wall-clock cap or a disconnect while
        // committing: the result is no longer wanted.
        self.abort_commit();
    }
}

/// Why a connection ended: the `reason` of its `disconnected` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndReason {
    /// No valid `hello`: a timeout after `HELLO_TIMEOUT`, or a close,
    /// error or other message where the hello should have been.
    NoHello,
    ProtoMismatch,
    /// Server shutdown, or the task dropped before it chose a reason.
    Shutdown,
    /// A close frame or end of stream from the client.
    ClientClosed,
    SocketError,
    /// Nothing, not even a pong, within `ws_dead_ms` (C5).
    DeadPeer,
    /// No session and no message for `idle_timeout_s`.
    Idle,
    /// The client stopped reading and its outbound queue filled (C17).
    NotReading,
    /// The connection task panicked.
    Panic,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoHello => "no_hello",
            Self::ProtoMismatch => "proto_mismatch",
            Self::Shutdown => "shutdown",
            Self::ClientClosed => "client_closed",
            Self::SocketError => "socket_error",
            Self::DeadPeer => "dead_peer",
            Self::Idle => "idle",
            Self::NotReading => "not_reading",
            Self::Panic => "panic",
        }
    }
}

/// What one connection did, logged once as `disconnected` when it is
/// dropped, so every way a connection ends (including a task dropped at
/// shutdown) leaves exactly one line. Counts and timings only, never text.
struct EndLog {
    peer: SocketAddr,
    key: Option<String>,
    /// The client's `hello.client` (its version string, [`clip`]ped);
    /// `-` before hello.
    client: String,
    opened: Instant,
    /// Text (control) messages after the hello, pings included: a health
    /// probe (hello, pings, close) has some, an idle client none.
    msgs: u64,
    /// Binary (audio) frame bytes received, including frames dropped for
    /// lack of a session.
    audio_bytes: u64,
    /// `start`s received, refused ones included.
    sessions: u64,
    results: u64,
    /// Sessions that ended without a result for another reason than the
    /// client's (each has its own `session ended` line), plus failed
    /// cleanups.
    errors: u64,
    /// Sessions the client ended itself: `cancel`, or a replacing `start`.
    cancels: u64,
    /// Text-only cleanups answered with text.
    cleans: u64,
    /// The session still open when the connection ended (no result sent).
    open_sid: Option<u64>,
    reason: EndReason,
}

impl EndLog {
    fn new(peer: SocketAddr, key: Option<String>) -> Self {
        Self {
            peer,
            key,
            client: "-".into(),
            opened: Instant::now(),
            msgs: 0,
            audio_bytes: 0,
            sessions: 0,
            results: 0,
            errors: 0,
            cancels: 0,
            cleans: 0,
            open_sid: None,
            reason: EndReason::Shutdown,
        }
    }

    /// A one-word summary, the worst thing that happened first: `errors`
    /// if any session or cleanup failed (even next to results), else
    /// `result`, `cleaned`, `no_audio` (nothing was sent) or `no_result`
    /// (audio but no result, e.g. only cancelled sessions).
    fn outcome(&self) -> &'static str {
        if self.errors > 0 {
            "errors"
        } else if self.results > 0 {
            "result"
        } else if self.cleans > 0 {
            "cleaned"
        } else if self.audio_bytes == 0 {
            "no_audio"
        } else {
            "no_result"
        }
    }
}

impl Drop for EndLog {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.reason = EndReason::Panic;
        }
        tracing::info!(
            peer = %self.peer,
            key = self.key.as_deref().unwrap_or("-"),
            client = ?self.client,
            duration_ms = self.opened.elapsed().as_millis() as u64,
            msgs = self.msgs,
            audio_bytes = self.audio_bytes,
            sessions = self.sessions,
            results = self.results,
            errors = self.errors,
            cancels = self.cancels,
            cleans = self.cleans,
            open_sid = self.open_sid,
            reason = self.reason.as_str(),
            outcome = self.outcome(),
            "disconnected"
        );
    }
}

/// A client-supplied string for the log: at most 64 characters. Log it with
/// `?` (Debug) so a newline or quote cannot forge a log line.
fn clip(s: &str) -> String {
    s.chars().take(64).collect()
}

/// Why a session ended without a result (its `session ended` line).
const CANCELLED: &str = "cancelled";
const REPLACED: &str = "replaced";

struct Conn {
    state: Arc<AppState>,
    peer: SocketAddr,
    /// The key this connection authenticated with (D38; never the secret).
    key: Option<String>,
    out: mpsc::Sender<Message>,
    /// Set when `out` was full: the client is not reading.
    overflow: std::cell::Cell<bool>,
    ev: mpsc::UnboundedSender<Event>,
    sess: Option<Sess>,
    dropped_audio: u64,
    /// Last client message other than ping/pong (idle close).
    last_activity: Instant,
    /// Last frame of any kind, pongs included (dead-peer detection).
    last_rx: Instant,
    /// Logs the `disconnected` line when the connection is dropped.
    end: EndLog,
}

pub(crate) async fn run(
    socket: WebSocket,
    state: Arc<AppState>,
    peer: SocketAddr,
    key: Option<crate::keys::KeyRef>,
) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(OUT_QUEUE);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut kill = state.kill.clone();
    let key_id = key.map(|k| k.id);
    // Dropped (and so logged) on every return below, or with the task.
    let mut end = EndLog::new(peer, key_id.clone());
    let send = |m: &ServerMsg| {
        let _ = out_tx.try_send(Message::Text(m.to_json().into()));
    };

    // ---- hello ----
    let first = tokio::select! {
        r = tokio::time::timeout(HELLO_TIMEOUT, stream.next()) => r,
        _ = kill.changed() => { writer.abort(); return; }
    };
    let hello = match first {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<ClientMsg>(&t).ok(),
        _ => None,
    };
    end.reason = EndReason::NoHello;
    let Some(ClientMsg::Hello(h)) = hello else {
        send(&ServerMsg::error(
            None,
            code::BAD_REQUEST,
            "expected hello",
            true,
        ));
        let _ = out_tx.try_send(Message::Close(None));
        let _ = writer.await;
        return;
    };
    end.client = clip(&h.client);
    let Some(proto) = negotiate(h.proto_min, h.proto_max, PROTO_MIN, PROTO_MAX) else {
        end.reason = EndReason::ProtoMismatch;
        let msg = format!(
            "client speaks {}..{}, server {PROTO_MIN}..{PROTO_MAX}",
            h.proto_min, h.proto_max
        );
        send(&ServerMsg::error(None, code::PROTO_MISMATCH, msg, true));
        let _ = out_tx.try_send(Message::Close(Some(CloseFrame {
            code: CLOSE_PROTO_MISMATCH,
            reason: "proto_mismatch".into(),
        })));
        let _ = writer.await;
        return;
    };
    send(&ServerMsg::HelloOk(crate::health::hello_ok(&state, proto)));
    end.reason = EndReason::Shutdown;
    if let (Some(store), Some(id)) = (&state.keys, &key_id) {
        store.record_client(id, &h.client);
    }
    tracing::info!(%peer, key = key_id.as_deref().unwrap_or("-"), client = ?clip(&h.client), core = ?clip(&h.core), "connected");

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Event>();
    let mut c = Conn {
        state: state.clone(),
        peer,
        key: key_id,
        out: out_tx.clone(),
        overflow: Default::default(),
        ev: ev_tx,
        sess: None,
        dropped_audio: 0,
        last_activity: Instant::now(),
        last_rx: Instant::now(),
        end,
    };
    let mut tick = tokio::time::interval(PARTIAL_POLL);
    let idle = Duration::from_secs(state.cfg.idle_timeout_s.max(1));
    let ping_every = Duration::from_millis(state.cfg.ws_ping_ms.max(10));
    let dead = Duration::from_millis(state.cfg.ws_dead_ms.max(1));
    let max_wall = Duration::from_secs(state.cfg.max_session_s);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    let reason = loop {
        if c.overflow.get() {
            tracing::info!(%peer, "client is not reading its messages; closing");
            break EndReason::NotReading;
        }
        tokio::select! {
            _ = kill.changed() => break EndReason::Shutdown,
            m = stream.next() => {
                c.last_rx = Instant::now();
                match m {
                    Some(Ok(Message::Text(t))) => {
                        c.last_activity = c.last_rx;
                        c.end.msgs += 1;
                        c.on_text(&t)
                    }
                    Some(Ok(Message::Binary(b))) => {
                        c.last_activity = c.last_rx;
                        c.on_audio(&b)
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => break EndReason::ClientClosed,
                    Some(Err(_)) => break EndReason::SocketError,
                }
            }
            Some(ev) = ev_rx.recv() => c.on_event(ev),
            _ = ping.tick() => {
                c.push(Message::Ping(Default::default()));
            }
            _ = tick.tick() => {
                // C5: a peer that has sent nothing, not even a pong, for the
                // dead interval is gone (half-open, NAT drop, app killed).
                // Dropping `c` drops the session and frees its worker.
                if c.last_rx.elapsed() > dead {
                    tracing::info!(%peer, "no frame within the dead interval; closing");
                    c.push(Message::Close(None));
                    break EndReason::DeadPeer;
                }
                c.enforce_wall_clock(max_wall);
                c.poll_partial();
                if c.sess.is_none() && c.last_activity.elapsed() > idle {
                    c.push(Message::Close(None));
                    break EndReason::Idle;
                }
            }
        }
    };
    c.end.reason = reason;
    if c.dropped_audio > 0 {
        tracing::debug!(%peer, frames = c.dropped_audio, "audio frames without a session");
    }
    if *kill.borrow() || c.overflow.get() {
        writer.abort();
    } else {
        drop(c);
        drop(out_tx);
        let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    }
}

impl Drop for Conn {
    /// A session still open when the connection ends (however it ends) gets
    /// its `session ended` line before the connection's `disconnected`.
    fn drop(&mut self) {
        self.end.open_sid = self.sess.as_ref().map(|s| s.sid);
        self.end_session("disconnect");
    }
}

impl Conn {
    /// End the current session (if any) without a result.
    fn end_session(&mut self, reason: &'static str) {
        if let Some(s) = self.sess.take() {
            self.session_ended(s.sid, reason, Some(&s));
        }
    }

    /// The one `session ended` line of a session that produced no result
    /// (`s` is `None` for a `start` refused before it began). Counts and
    /// timings only, never text.
    fn session_ended(&mut self, sid: u64, reason: &'static str, s: Option<&Sess>) {
        if reason == CANCELLED || reason == REPLACED {
            self.end.cancels += 1;
        } else {
            self.end.errors += 1;
        }
        tracing::info!(
            sid,
            peer = %self.peer,
            key = self.key.as_deref().unwrap_or("-"),
            reason,
            duration_ms = s.map_or(0, |s| s.created.elapsed().as_millis() as u64),
            audio_bytes = s.map_or(0, |s| s.samples * 2),
            "session ended"
        );
    }

    fn send(&self, m: &ServerMsg) {
        self.push(Message::Text(m.to_json().into()));
    }

    fn push(&self, m: Message) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.out.try_send(m) {
            self.overflow.set(true);
        }
    }

    fn err(&self, sid: Option<u64>, c: &str, msg: impl Into<String>) {
        self.send(&ServerMsg::error(sid, c, msg, false));
    }

    /// The current session if `sid` names it; else reply `stale_sid` /
    /// `no_session`.
    fn check_sid(&mut self, sid: u64) -> bool {
        match &self.sess {
            Some(s) if s.sid == sid => true,
            Some(_) => {
                self.err(Some(sid), code::STALE_SID, "not the current session");
                false
            }
            None => {
                self.err(Some(sid), code::NO_SESSION, "no active session");
                false
            }
        }
    }

    fn on_text(&mut self, t: &str) {
        let m = match serde_json::from_str::<ClientMsg>(t) {
            Ok(m) => m,
            Err(e) => return self.err(None, code::BAD_REQUEST, format!("bad message: {e}")),
        };
        match m {
            ClientMsg::Hello(_) => self.err(None, code::BAD_REQUEST, "hello already done"),
            ClientMsg::Start { sid, config } => self.start(sid, config),
            ClientMsg::Context { sid, ctx } => {
                if let Err(e) = crate::limits::context(&ctx) {
                    return self.err(Some(sid), code::TOO_LARGE, e);
                }
                if self.check_sid(sid) {
                    let s = self.sess.as_mut().unwrap();
                    if let Some(l) = &s.lease {
                        l.core.set_context(ctx.clone());
                    }
                    s.ctx = Some(ctx);
                }
            }
            ClientMsg::Commit { sid } => {
                if self.check_sid(sid) {
                    let s = self.sess.as_mut().unwrap();
                    s.start_to_commit_ms
                        .get_or_insert(s.created.elapsed().as_millis() as u64);
                    if s.lease.is_some() {
                        self.commit();
                    } else {
                        s.commit_requested = true;
                    }
                }
            }
            ClientMsg::Cancel { sid } => {
                self.state.cancels.fetch_add(1, Ordering::SeqCst);
                if self.sess.as_ref().is_some_and(|s| s.sid == sid) {
                    self.end_session(CANCELLED);
                }
                self.send(&ServerMsg::Cancelled { sid });
            }
            ClientMsg::Ping { n } => self.send(&ServerMsg::Pong {
                n,
                busy: self.state.pool.busy(),
            }),
            ClientMsg::Clean { id, req } => self.clean(id, *req),
        }
    }

    /// D36 text-only cleanup, independent of the session: lease a worker
    /// (queueing like a session), run its LLM on the request off the async
    /// threads, answer `cleaned` with the same `id`.
    fn clean(&mut self, id: u64, req: CleanRequest) {
        let refuse = |error: &str| {
            ServerMsg::Cleaned(Cleaned {
                id,
                error: Some(error.into()),
                ..Default::default()
            })
        };
        if !self.state.pool.llm {
            self.end.errors += 1;
            return self.send(&refuse("no_llm"));
        }
        if let Err(e) = crate::limits::clean(&req) {
            tracing::info!(id, peer = %self.peer, "clean refused: {e}");
            self.end.errors += 1;
            return self.send(&refuse(code::TOO_LARGE));
        }
        let pool = self.state.pool.clone();
        let ev = self.ev.clone();
        let peer = self.peer;
        let queue_timeout = Duration::from_millis(self.state.cfg.queue_timeout_ms);
        let t0 = Instant::now();
        let lease = match pool.try_lease() {
            Some(l) => Some(l),
            None if pool.enqueue() => None,
            None => {
                self.end.errors += 1;
                return self.send(&refuse(code::BUSY));
            }
        };
        tokio::spawn(async move {
            let lease = match lease {
                Some(l) => l,
                None => match pool.wait_lease(queue_timeout).await {
                    Ok(l) => l,
                    Err(_) => {
                        let _ = ev.send(Event::Cleaned(Cleaned {
                            id,
                            error: Some(code::BUSY.into()),
                            queue_ms: t0.elapsed().as_millis() as u64,
                            ..Default::default()
                        }));
                        return;
                    }
                },
            };
            let queue_ms = t0.elapsed().as_millis() as u64;
            let chars = req.text.chars().count();
            let out = tokio::task::spawn_blocking(move || {
                let Some(cl) = lease.core.cleaner() else {
                    return Cleaned {
                        id,
                        error: Some("no_llm".into()),
                        queue_ms,
                        ..Default::default()
                    };
                };
                let (r, usage, took) =
                    murmur_core::cleanup::clean_text(cl, &req, crate::limits::MAX_LLM_TIMEOUT_MS);
                drop(lease); // the worker is free once its LLM call ended
                let (text, error) = match r {
                    Ok(t) => (Some(t), None),
                    Err(e) => (None, Some(e)),
                };
                Cleaned {
                    id,
                    text,
                    error,
                    llm_ms: took.as_millis() as u64,
                    queue_ms,
                    model: usage.model,
                }
            })
            .await
            .unwrap_or_else(|_| Cleaned {
                id,
                error: Some(code::SERVER_ERROR.into()),
                ..Default::default()
            });
            // Timings and sizes only, never text.
            tracing::info!(
                id,
                peer = %peer,
                chars,
                llm_ms = out.llm_ms,
                queue_ms = out.queue_ms,
                error = out.error.as_deref().unwrap_or(""),
                "cleaned"
            );
            let _ = ev.send(Event::Cleaned(out));
        });
    }

    fn start(&mut self, sid: u64, cfg: SessionConfig) {
        // A start while a session is active cancels it (MurmurCore semantics).
        self.end_session(REPLACED);
        self.end.sessions += 1;
        let cfg = match crate::limits::session(cfg, self.state.cfg.max_session_samples()) {
            Ok(c) => crate::limits::llm_scale(
                c,
                self.state.cfg.llm_timeout_per_char_ms,
                self.state.cfg.llm_timeout_max_ms,
            ),
            Err(e) => {
                self.session_ended(sid, "too_large", None);
                return self.err(Some(sid), code::TOO_LARGE, e);
            }
        };
        self.state.sessions.fetch_add(1, Ordering::SeqCst);
        let mut s = Sess {
            sid,
            cfg,
            lease: None,
            queued_audio: Vec::new(),
            ctx: None,
            samples: 0,
            created: Instant::now(),
            started: SystemTime::now(),
            captured: self.state.capture.as_ref().map(|_| Vec::new()),
            queue_ms: 0,
            start_to_commit_ms: None,
            commit_requested: false,
            committing: false,
            last_partial: None,
            waiter: None,
            inflight: Arc::default(),
        };
        if let Some(l) = self.state.pool.try_lease() {
            if let Err(e) = l.core.start_session(s.cfg.clone()) {
                self.session_ended(sid, "worker_failed", Some(&s));
                return self.worker_failed(sid, e);
            }
            s.lease = Some(l);
            self.send(&ServerMsg::Started { sid, queued: false });
        } else if self.state.pool.enqueue() {
            let pool = self.state.pool.clone();
            let ev = self.ev.clone();
            let t = Duration::from_millis(self.state.cfg.queue_timeout_ms);
            let task = tokio::spawn(async move {
                let r = pool.wait_lease(t).await;
                let _ = ev.send(Event::Leased(sid, r));
            });
            s.waiter = Some(task.abort_handle());
            self.send(&ServerMsg::Started { sid, queued: true });
        } else {
            self.session_ended(sid, "busy", Some(&s));
            self.err(
                Some(sid),
                code::BUSY,
                "all workers busy and the queue is full",
            );
            return;
        }
        self.sess = Some(s);
    }

    /// `start_session` failed (the core's worker thread is gone): report
    /// it instead of a `started` that would never produce a result.
    fn worker_failed(&self, sid: u64, e: murmur_core::CoreError) {
        tracing::error!(sid, peer = %self.peer, kind = e.kind(), "start_session failed: {e}");
        self.send(&ServerMsg::error(
            Some(sid),
            code::SERVER_ERROR,
            format!("worker failed: {}", e.kind()),
            false,
        ));
    }

    fn on_audio(&mut self, b: &[u8]) {
        self.end.audio_bytes += b.len() as u64;
        let pcm = match decode_pcm(b) {
            Ok(p) => p,
            Err(FrameError::TooLarge) => {
                return self.err(
                    self.sess.as_ref().map(|s| s.sid),
                    code::TOO_LARGE,
                    "audio frame over 64 KiB",
                )
            }
            Err(FrameError::OddLength) => {
                return self.err(
                    self.sess.as_ref().map(|s| s.sid),
                    code::BAD_REQUEST,
                    "odd-length audio frame",
                )
            }
        };
        let max_session = self.state.cfg.max_session_samples() as u64;
        let Some(s) = self
            .sess
            .as_mut()
            .filter(|s| !s.committing && !s.commit_requested)
        else {
            self.dropped_audio += 1;
            return;
        };
        s.samples += pcm.len() as u64;
        if s.samples > max_session {
            let sid = s.sid;
            self.end_session("too_large");
            return self.err(
                Some(sid),
                code::TOO_LARGE,
                "session longer than max_session_s",
            );
        }
        if let Some(c) = &mut s.captured {
            c.extend_from_slice(&pcm);
        }
        match &s.lease {
            Some(l) => {
                l.core.push_audio(&pcm);
            }
            None => {
                let room = s.cfg.max_samples.saturating_sub(s.queued_audio.len());
                s.queued_audio
                    .extend_from_slice(&pcm[..pcm.len().min(room)]);
            }
        }
    }

    fn commit(&mut self) {
        let s = self.sess.as_mut().unwrap();
        s.committing = true;
        if self.state.hooks.stall_commits.load(Ordering::SeqCst) {
            return; // test hook: never answer
        }
        let lease = s.lease.take().expect("leased");
        let (sid, ev) = (s.sid, self.ev.clone());
        *s.inflight.lock().unwrap_or_else(|e| e.into_inner()) = Some(lease.core.clone());
        let inflight = s.inflight.clone();
        tokio::task::spawn_blocking(move || {
            let r = lease.core.commit();
            // Unpublish the core before the worker goes back to the pool.
            *inflight.lock().unwrap_or_else(|e| e.into_inner()) = None;
            drop(lease); // the worker is free only after its commit finished
            let _ = ev.send(Event::Result(sid, Box::new(r)));
        });
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Cleaned(c) => {
                // A clean succeeded when it answered with text.
                if c.text.is_some() {
                    self.end.cleans += 1;
                    if let (Some(store), Some(id)) = (&self.state.keys, &self.key) {
                        store.record_clean(id);
                    }
                } else {
                    self.end.errors += 1;
                }
                self.send(&ServerMsg::Cleaned(c))
            }
            Event::Leased(sid, r) => {
                let Some(s) = self
                    .sess
                    .as_mut()
                    .filter(|s| s.sid == sid && s.lease.is_none())
                else {
                    return; // session gone: the lease drops here
                };
                match r {
                    Ok(l) => {
                        s.queue_ms = s.created.elapsed().as_millis() as u64;
                        let mut cfg = s.cfg.clone();
                        cfg.ring_capacity_samples = cfg
                            .ring_capacity_samples
                            .max(s.queued_audio.len() + SAMPLE_RATE * 30);
                        if let Err(e) = l.core.start_session(cfg) {
                            self.end_session("worker_failed");
                            return self.worker_failed(sid, e);
                        }
                        l.core.push_audio(&std::mem::take(&mut s.queued_audio));
                        if let Some(ctx) = &s.ctx {
                            l.core.set_context(ctx.clone());
                        }
                        s.lease = Some(l);
                        if s.commit_requested {
                            self.commit();
                        }
                    }
                    Err(_) => {
                        self.end_session("busy");
                        self.err(
                            Some(sid),
                            code::BUSY,
                            "no worker freed within queue_timeout_ms",
                        );
                    }
                }
            }
            Event::Result(sid, r) => {
                let Some(mut s) = self.sess.take() else {
                    return;
                };
                if s.sid != sid {
                    self.sess = Some(s);
                    return;
                }
                log_result(
                    sid,
                    self.peer,
                    self.key.as_deref(),
                    &r,
                    s.queue_ms,
                    s.start_to_commit_ms,
                );
                if let (Some(store), Some(id)) = (&self.state.keys, &self.key) {
                    store.record_dictation(id, samples_to_ms(s.samples));
                }
                if self.state.cfg.log.log_text {
                    tracing::debug!(sid, text = %r.final_text, "result text");
                }
                self.capture(&mut s, &r);
                self.end.results += 1;
                self.send(&ServerMsg::Result {
                    sid,
                    result: r,
                    server_timings: ServerTimings {
                        queue_ms: s.queue_ms,
                        recv_audio_ms: samples_to_ms(s.samples),
                    },
                });
            }
        }
    }

    /// `[debug] capture`: save the session's audio and result off the
    /// async threads. Failures are logged, never sent to the client.
    fn capture(&self, s: &mut Sess, r: &CommitResult) {
        let (Some(store), Some(pcm)) = (self.state.capture.clone(), s.captured.take()) else {
            return;
        };
        let (sid, peer, started) = (s.sid, self.peer, s.started);
        let (queue_ms, start_to_commit_ms) = (s.queue_ms, s.start_to_commit_ms);
        // The model that decoded this session (its language route, D33).
        let (asr, model) = self.state.pool.asr_for(s.cfg.forced_language());
        let (asr, model) = (asr.to_string(), model.to_string());
        let r = r.clone();
        tokio::task::spawn_blocking(move || {
            let rec = crate::capture::Record {
                sid,
                peer,
                result: &r,
                asr: &asr,
                model: &model,
                queue_ms,
                start_to_commit_ms,
            };
            match store.save(started, &pcm, &rec) {
                Ok(name) => tracing::debug!(sid, audio = %name, "debug capture saved"),
                Err(e) => tracing::warn!(sid, "debug capture failed: {e}"),
            }
        });
    }

    /// End a session that has been open longer than `max_session_s` of
    /// wall-clock time without committing (C5: pongs alone must not hold a
    /// worker forever). A running commit is left to finish.
    fn enforce_wall_clock(&mut self, max: Duration) {
        let Some(s) = &self.sess else { return };
        if s.committing || s.created.elapsed() <= max {
            return;
        }
        let sid = s.sid;
        self.end_session("wall_clock");
        self.err(
            Some(sid),
            code::TOO_LARGE,
            "session longer than max_session_s",
        );
    }

    fn poll_partial(&mut self) {
        let Some(s) = self.sess.as_mut() else { return };
        if s.committing || s.cfg.partial_interval_ms.is_none() {
            return;
        }
        let Some(l) = &s.lease else { return };
        let p = l.core.partial();
        if let Some(text) = p.clone().filter(|_| p != s.last_partial) {
            s.last_partial = p;
            let m = ServerMsg::Partial { sid: s.sid, text };
            self.push(Message::Text(m.to_json().into()));
        }
    }
}

/// The per-dictation `result` log line (timings only, never text). Optional
/// values print as bare numbers and are omitted when unknown (tracing
/// records `Option<T>` only when `Some`), not as `Some(7200)`.
fn log_result(
    sid: u64,
    peer: SocketAddr,
    key: Option<&str>,
    r: &CommitResult,
    queue_ms: u64,
    start_to_commit_ms: Option<u64>,
) {
    let t = &r.timings;
    tracing::info!(
        sid,
        peer = %peer,
        key,
        audio_ms = t.audio_ms,
        asr_ms = t.asr_ms,
        asr_audio_ms = t.asr_audio_ms,
        wait_ms = t.wait_ms,
        segments = t.segments,
        pre_asr_ms = t.pre_asr_ms,
        llm_ms = t.llm_ms,
        total_ms = t.total_ms,
        queue_ms,
        start_to_commit_ms,
        speech_ms = t.speech_ms,
        peak_dbfs = t.peak_dbfs,
        status = ?r.status,
        language = r.language.as_deref(),
        empty_reason = r.empty_reason.as_deref().unwrap_or(""),
        fallback_reason = r.fallback_reason.as_deref().unwrap_or(""),
        "result"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> CommitResult {
        serde_json::from_value(serde_json::json!({
            "status": "EMPTY", "asr_text": "", "final_text": "", "language": null,
            "timings": serde_json::to_value(murmur_core::Timings::default()).unwrap(),
            "press_enter": false, "fallback_reason": null,
            "truncated": false, "error": null
        }))
        .unwrap()
    }

    /// Captures what `f` logs with the server's fmt layer.
    fn logged(f: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Buf(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Buf::default();
        let out = buf.clone();
        let sub = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || out.clone())
            .finish();
        tracing::subscriber::with_default(sub, f);
        let v = buf.0.lock().unwrap().clone();
        String::from_utf8(v).unwrap()
    }

    /// What `log_result` prints.
    fn captured(r: &CommitResult) -> String {
        logged(|| {
            log_result(
                7,
                "100.64.0.2:5000".parse().unwrap(),
                Some("k-0123456789abcdef"),
                r,
                12,
                Some(7400),
            );
        })
    }

    fn end_log() -> EndLog {
        let mut e = EndLog::new(
            "100.64.0.2:5000".parse().unwrap(),
            Some("k-0123456789abcdef".into()),
        );
        e.client = "murmur-ffi 1.0.0+abc".into();
        e
    }

    #[test]
    fn disconnected_line_names_key_client_bytes_and_reason() {
        let line = logged(|| {
            let mut e = end_log();
            e.audio_bytes = 64_000;
            e.sessions = 1;
            e.open_sid = Some(3);
            e.reason = EndReason::ClientClosed;
        });
        assert_eq!(line.matches("disconnected").count(), 1, "{line}");
        assert!(line.contains("key=\"k-0123456789abcdef\""), "{line}");
        assert!(line.contains("client=\"murmur-ffi 1.0.0+abc\""), "{line}");
        assert!(line.contains("duration_ms="), "{line}");
        assert!(line.contains("audio_bytes=64000"), "{line}");
        assert!(
            line.contains(
                "msgs=0 audio_bytes=64000 sessions=1 results=0 errors=0 cancels=0 cleans=0"
            ),
            "{line}"
        );
        assert!(line.contains("open_sid=3"), "{line}");
        assert!(line.contains("reason=\"client_closed\""), "{line}");
        assert!(line.contains("outcome=\"no_result\""), "{line}");
        assert!(!line.contains("Some("), "{line}");
    }

    #[test]
    fn a_client_string_cannot_forge_a_log_line() {
        let line = logged(|| {
            let mut e = end_log();
            e.client = clip(&format!(
                "evil\nINFO murmur_server::ws: forged {}",
                "x".repeat(100)
            ));
        });
        assert_eq!(line.lines().count(), 1, "{line}");
        assert!(line.contains("client=\"evil\\nINFO"), "{line}");
        assert!(
            !line.contains(&"x".repeat(60)),
            "clipped to 64 chars: {line}"
        );
    }

    #[test]
    fn disconnected_outcome_summarises_what_the_connection_did() {
        let mut e = end_log();
        assert_eq!(e.outcome(), "no_audio");
        e.audio_bytes = 2;
        e.cancels = 1;
        assert_eq!(e.outcome(), "no_result");
        e.cleans = 1;
        assert_eq!(e.outcome(), "cleaned");
        e.results = 1;
        assert_eq!(e.outcome(), "result");
        // A failure is not hidden by results next to it.
        e.errors = 1;
        assert_eq!(e.outcome(), "errors");
        // An unset reason is a dropped task: the server shut down.
        assert_eq!(end_log().reason, EndReason::Shutdown);
        let line = logged(|| drop(e));
        assert!(!line.contains("open_sid"), "{line}");
    }

    #[test]
    fn result_line_prints_bare_numbers_not_option_debug() {
        let mut r = result();
        r.timings.speech_ms = Some(7200);
        r.timings.peak_dbfs = Some(-2);
        let line = captured(&r);
        assert!(line.contains("speech_ms=7200"), "{line}");
        assert!(line.contains("peak_dbfs=-2"), "{line}");
        assert!(line.contains("start_to_commit_ms=7400"), "{line}");
        assert!(line.contains("key=\"k-0123456789abcdef\""), "{line}");
        assert!(!line.contains("Some("), "{line}");
        r.language = Some("he".into());
        assert!(captured(&r).contains("language=\"he\""));
    }

    #[test]
    fn result_line_omits_unknown_values() {
        let line = captured(&result());
        assert!(line.contains(" result"), "{line}");
        assert!(!line.contains("speech_ms"), "{line}");
        assert!(!line.contains("peak_dbfs"), "{line}");
        assert!(!line.contains("language"), "{line}");
        assert!(!line.contains("None"), "{line}");
    }
}
