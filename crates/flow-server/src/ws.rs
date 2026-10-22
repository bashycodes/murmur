//! Per-connection task: hello, message loop, worker lease, partial poller.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use flow_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use flow_core::CommitResult;
use flow_protocol::{
    code, decode_pcm, negotiate, samples_to_ms, ClientMsg, FrameError, ServerMsg, ServerTimings,
    CLOSE_PROTO_MISMATCH, PROTO_MAX, PROTO_MIN,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::pool::{Lease, LeaseError};
use crate::AppState;

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const PARTIAL_POLL: Duration = Duration::from_millis(100);

enum Event {
    Leased(u64, Result<Lease, LeaseError>),
    Result(u64, Box<CommitResult>),
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
    queue_ms: u64,
    commit_requested: bool,
    committing: bool,
    last_partial: Option<String>,
    /// The task waiting for a worker while queued (aborted with the session).
    waiter: Option<tokio::task::AbortHandle>,
}

impl Drop for Sess {
    fn drop(&mut self) {
        if let Some(w) = self.waiter.take() {
            w.abort();
        }
    }
}

struct Conn {
    state: Arc<AppState>,
    peer: SocketAddr,
    out: mpsc::UnboundedSender<Message>,
    ev: mpsc::UnboundedSender<Event>,
    sess: Option<Sess>,
    dropped_audio: u64,
    /// Last client message other than ping/pong (idle close).
    last_activity: Instant,
    /// Last frame of any kind, pongs included (dead-peer detection).
    last_rx: Instant,
}

pub(crate) async fn run(socket: WebSocket, state: Arc<AppState>, peer: SocketAddr) {
    let (mut sink, mut stream) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
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
    let send = |m: &ServerMsg| {
        let _ = out_tx.send(Message::Text(m.to_json().into()));
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
    let Some(ClientMsg::Hello(h)) = hello else {
        send(&ServerMsg::error(
            None,
            code::BAD_REQUEST,
            "expected hello",
            true,
        ));
        let _ = out_tx.send(Message::Close(None));
        let _ = writer.await;
        return;
    };
    let Some(proto) = negotiate(h.proto_min, h.proto_max, PROTO_MIN, PROTO_MAX) else {
        let msg = format!(
            "client speaks {}..{}, server {PROTO_MIN}..{PROTO_MAX}",
            h.proto_min, h.proto_max
        );
        send(&ServerMsg::error(None, code::PROTO_MISMATCH, msg, true));
        let _ = out_tx.send(Message::Close(Some(CloseFrame {
            code: CLOSE_PROTO_MISMATCH,
            reason: "proto_mismatch".into(),
        })));
        let _ = writer.await;
        return;
    };
    send(&ServerMsg::HelloOk(crate::health::hello_ok(&state, proto)));
    tracing::info!(%peer, client = %h.client, core = %h.core, "connected");

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Event>();
    let mut c = Conn {
        state: state.clone(),
        peer,
        out: out_tx.clone(),
        ev: ev_tx,
        sess: None,
        dropped_audio: 0,
        last_activity: Instant::now(),
        last_rx: Instant::now(),
    };
    let mut tick = tokio::time::interval(PARTIAL_POLL);
    let idle = Duration::from_secs(state.cfg.idle_timeout_s.max(1));
    let ping_every = Duration::from_millis(state.cfg.ws_ping_ms.max(10));
    let dead = Duration::from_millis(state.cfg.ws_dead_ms.max(1));
    let max_wall = Duration::from_secs(state.cfg.max_session_s);
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + ping_every, ping_every);
    loop {
        tokio::select! {
            _ = kill.changed() => break,
            m = stream.next() => {
                c.last_rx = Instant::now();
                match m {
                    Some(Ok(Message::Text(t))) => {
                        c.last_activity = c.last_rx;
                        c.on_text(&t)
                    }
                    Some(Ok(Message::Binary(b))) => {
                        c.last_activity = c.last_rx;
                        c.on_audio(&b)
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                }
            }
            Some(ev) = ev_rx.recv() => c.on_event(ev),
            _ = ping.tick() => {
                let _ = out_tx.send(Message::Ping(Default::default()));
            }
            _ = tick.tick() => {
                // C5: a peer that has sent nothing, not even a pong, for the
                // dead interval is gone (half-open, NAT drop, app killed).
                // Dropping `c` drops the session and frees its worker.
                if c.last_rx.elapsed() > dead {
                    tracing::info!(%peer, "no frame within the dead interval; closing");
                    let _ = out_tx.send(Message::Close(None));
                    break;
                }
                c.enforce_wall_clock(max_wall);
                c.poll_partial();
                if c.sess.is_none() && c.last_activity.elapsed() > idle {
                    let _ = out_tx.send(Message::Close(None));
                    break;
                }
            }
        }
    }
    if let Some(s) = c.sess.take() {
        tracing::info!(sid = s.sid, %peer, "session ended by disconnect");
    }
    if c.dropped_audio > 0 {
        tracing::debug!(%peer, frames = c.dropped_audio, "audio frames without a session");
    }
    if *kill.borrow() {
        writer.abort();
    } else {
        drop(c);
        drop(out_tx);
        let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    }
}

impl Conn {
    fn send(&self, m: &ServerMsg) {
        let _ = self.out.send(Message::Text(m.to_json().into()));
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
                    self.sess = None;
                    tracing::info!(sid, peer = %self.peer, "cancelled");
                }
                self.send(&ServerMsg::Cancelled { sid });
            }
            ClientMsg::Ping { n } => self.send(&ServerMsg::Pong {
                n,
                busy: self.state.pool.busy(),
            }),
        }
    }

    fn start(&mut self, sid: u64, cfg: SessionConfig) {
        // A start while a session is active cancels it (FlowCore semantics).
        self.sess = None;
        let cfg = match crate::limits::session(cfg, self.state.cfg.max_session_samples()) {
            Ok(c) => c,
            Err(e) => return self.err(Some(sid), code::TOO_LARGE, e),
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
            queue_ms: 0,
            commit_requested: false,
            committing: false,
            last_partial: None,
            waiter: None,
        };
        if let Some(l) = self.state.pool.try_lease() {
            let _ = l.core.start_session(s.cfg.clone());
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
            self.err(
                Some(sid),
                code::BUSY,
                "all workers busy and the queue is full",
            );
            return;
        }
        self.sess = Some(s);
    }

    fn on_audio(&mut self, b: &[u8]) {
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
            self.sess = None;
            return self.err(
                Some(sid),
                code::TOO_LARGE,
                "session longer than max_session_s",
            );
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
        tokio::task::spawn_blocking(move || {
            let r = lease.core.commit();
            drop(lease); // the worker is free only after its commit finished
            let _ = ev.send(Event::Result(sid, Box::new(r)));
        });
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
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
                        let _ = l.core.start_session(cfg);
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
                        self.sess = None;
                        self.err(
                            Some(sid),
                            code::BUSY,
                            "no worker freed within queue_timeout_ms",
                        );
                    }
                }
            }
            Event::Result(sid, r) => {
                let Some(s) = self.sess.take() else { return };
                if s.sid != sid {
                    self.sess = Some(s);
                    return;
                }
                let t = &r.timings;
                tracing::info!(
                    sid,
                    peer = %self.peer,
                    audio_ms = t.audio_ms,
                    asr_ms = t.asr_ms,
                    llm_ms = t.llm_ms,
                    total_ms = t.total_ms,
                    queue_ms = s.queue_ms,
                    status = ?r.status,
                    empty_reason = r.empty_reason.as_deref().unwrap_or(""),
                    fallback_reason = r.fallback_reason.as_deref().unwrap_or(""),
                    "result"
                );
                if self.state.cfg.log.log_text {
                    tracing::debug!(sid, text = %r.final_text, "result text");
                }
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

    /// End a session that has been open longer than `max_session_s` of
    /// wall-clock time without committing (C5: pongs alone must not hold a
    /// worker forever). A running commit is left to finish.
    fn enforce_wall_clock(&mut self, max: Duration) {
        let Some(s) = &self.sess else { return };
        if s.committing || s.created.elapsed() <= max {
            return;
        }
        let sid = s.sid;
        self.sess = None;
        tracing::info!(sid, peer = %self.peer, "session over max_session_s wall-clock");
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
            let _ = self.out.send(Message::Text(m.to_json().into()));
        }
    }
}
