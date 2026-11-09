//! C5: a half-open or vanished client must not hold a worker forever.
//! The server pings, closes a connection that has been silent (not even a
//! pong) for the dead interval, and caps a session's wall-clock time.

mod common;

use std::time::{Duration, Instant};

use common::*;
use futures_util::StreamExt;
use murmur_core::config::SessionConfig;
use murmur_protocol::{code, ClientMsg, ServerMsg};
use tokio_tungstenite::tungstenite::Message;

fn fast(f: impl FnOnce(&mut murmur_server::Config)) -> murmur_server::Config {
    let mut c = mock_cfg("x");
    c.ws_ping_ms = 100;
    c.ws_dead_ms = 400;
    f(&mut c);
    c
}

async fn start_leased(ws: &mut Ws, sid: u64) {
    send(
        ws,
        &ClientMsg::Start {
            sid,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(ws).await,
        Some(ServerMsg::Started { sid, queued: false })
    );
}

async fn wait_idle(s: &murmur_server::ServerHandle, limit: Duration) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < limit {
        if s.pool().busy() == 0 {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_client_with_a_session_loses_its_worker() {
    let s = start(fast(|_| {})).await;
    let mut a = session(s.addr()).await;
    start_leased(&mut a, 1).await;
    assert_eq!(s.pool().busy(), 1);
    // `a` is never polled again: it answers no pings (half-open).
    assert!(
        wait_idle(&s, Duration::from_secs(3)).await,
        "worker freed after the dead interval"
    );
    let mut b = session(s.addr()).await;
    start_leased(&mut b, 1).await;
    drop(a);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_answers_pings_stays_connected() {
    let s = start(fast(|_| {})).await;
    let mut ws = session(s.addr()).await;
    start_leased(&mut ws, 1).await;
    // Read (and so auto-answer pings) for 3x the dead interval without
    // sending anything ourselves.
    let mut pings = 0;
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(1200) {
        match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            Err(_) => {}
            Ok(Some(Ok(Message::Ping(_)))) => pings += 1,
            Ok(Some(Ok(Message::Text(_)))) => {}
            other => panic!("connection dropped: {other:?}"),
        }
    }
    assert!(pings >= 5, "server pinged {pings} times");
    assert_eq!(s.pool().busy(), 1, "the session kept its worker");
    send_audio(&mut ws, &speech(300)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 1, .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_is_capped_in_wall_clock_time() {
    // Pongs keep the connection alive, but a session that never commits
    // must still end at max_session_s.
    let s = start(fast(|c| c.max_session_s = 1)).await;
    let mut ws = session(s.addr()).await;
    start_leased(&mut ws, 7).await;
    let t0 = Instant::now();
    let e = loop {
        assert!(t0.elapsed() < Duration::from_secs(4), "no wall-clock cap");
        match tokio::time::timeout(Duration::from_millis(50), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if let Ok(ServerMsg::Error(e)) = serde_json::from_str(&t) {
                    break e;
                }
            }
            Ok(None) | Ok(Some(Err(_))) => panic!("closed"),
            _ => {}
        }
    };
    assert_eq!((e.code.as_str(), e.sid), (code::TOO_LARGE, Some(7)));
    assert!(t0.elapsed() >= Duration::from_millis(900));
    assert!(wait_idle(&s, Duration::from_secs(1)).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_client_without_a_session_is_closed_too() {
    let s = start(fast(|_| {})).await;
    let mut ws = session(s.addr()).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    // The server gave up on us while we weren't reading: the buffered
    // pings are followed by a close (or a reset).
    let t0 = Instant::now();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => break,
            Ok(Some(Ok(_))) => assert!(t0.elapsed() < Duration::from_secs(2)),
            Err(_) => panic!("still open"),
        }
    }
}
