//! S7: unauthenticated clients cannot hold sockets open (slowloris) or
//! exhaust connections.

mod common;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn cfg(f: impl FnOnce(&mut murmur_server::Config)) -> murmur_server::Config {
    let mut c = mock_cfg("x");
    f(&mut c);
    c
}

/// Wait until the server closes `s` (EOF or reset); returns how long it
/// took, or `None` if it stayed open for `limit`.
async fn closed_within(s: &mut TcpStream, limit: Duration) -> Option<Duration> {
    let t0 = Instant::now();
    let mut buf = [0u8; 1024];
    loop {
        match tokio::time::timeout_at((t0 + limit).into(), s.read(&mut buf)).await {
            Err(_) => return None,
            Ok(Ok(0)) | Ok(Err(_)) => return Some(t0.elapsed()),
            Ok(Ok(_)) => continue, // e.g. a 408 before the close
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn silent_and_trickling_connections_are_closed_after_the_handshake_timeout() {
    let s = start(cfg(|c| c.handshake_timeout_ms = 300)).await;
    let mut silent = TcpStream::connect(s.addr()).await.unwrap();
    let mut trickle = TcpStream::connect(s.addr()).await.unwrap();
    trickle
        .write_all(b"GET /v1/stream HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    let keep_trickling = tokio::spawn(async move {
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if trickle.write_all(b"X-A: b\r\n").await.is_err() {
                return Some(trickle);
            }
        }
        Some(trickle)
    });
    assert!(
        closed_within(&mut silent, Duration::from_secs(3))
            .await
            .is_some(),
        "a connection that never sends a request is closed"
    );
    let mut trickle = keep_trickling.await.unwrap().unwrap();
    assert!(
        closed_within(&mut trickle, Duration::from_millis(500))
            .await
            .is_some(),
        "headers trickled past the timeout are cut off"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_http_request_does_not_keep_the_socket() {
    let s = start(cfg(|c| c.handshake_timeout_ms = 300)).await;
    let mut t = TcpStream::connect(s.addr()).await.unwrap();
    // keep-alive request with a wrong token: 401, then the server closes.
    t.write_all(b"GET /v1/health HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    assert!(closed_within(&mut t, Duration::from_secs(2))
        .await
        .is_some());
}

async fn hold(addr: SocketAddr, n: usize) -> Vec<TcpStream> {
    let mut v = Vec::new();
    for _ in 0..n {
        v.push(TcpStream::connect(addr).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    v
}

async fn connect_eventually(addr: SocketAddr) -> bool {
    for _ in 0..40 {
        if connect(addr, Some(TOKEN)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_over_the_global_cap_are_dropped() {
    let s = start(cfg(|c| {
        c.max_connections = 2;
        c.max_connections_per_ip = 100;
    }))
    .await;
    let idle = hold(s.addr(), 2).await;
    let mut third = TcpStream::connect(s.addr()).await.unwrap();
    assert!(closed_within(&mut third, Duration::from_millis(500))
        .await
        .is_some());
    assert!(connect(s.addr(), Some(TOKEN)).await.is_err());
    drop(idle);
    assert!(connect_eventually(s.addr()).await, "slots come back");
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_over_the_per_ip_cap_are_dropped() {
    let s = start(cfg(|c| {
        c.max_connections = 100;
        c.max_connections_per_ip = 1;
    }))
    .await;
    let idle = hold(s.addr(), 1).await;
    let mut second = TcpStream::connect(s.addr()).await.unwrap();
    assert!(closed_within(&mut second, Duration::from_millis(500))
        .await
        .is_some());
    drop(idle);
    assert!(connect_eventually(s.addr()).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgraded_websocket_keeps_its_slot() {
    let s = start(cfg(|c| c.max_connections = 1)).await;
    let ws = session(s.addr()).await;
    assert!(
        connect(s.addr(), Some(TOKEN)).await.is_err(),
        "the open WebSocket still counts"
    );
    drop(ws);
    assert!(connect_eventually(s.addr()).await);
}

#[cfg(feature = "tls")]
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_tls_handshake_is_closed() {
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    std::fs::write(&cert, ck.cert.pem()).unwrap();
    std::fs::write(&key, ck.key_pair.serialize_pem()).unwrap();
    let s = start(cfg(|c| {
        c.handshake_timeout_ms = 300;
        c.tls = Some(murmur_server::config::TlsConfig { cert, key });
    }))
    .await;
    let mut t = TcpStream::connect(s.addr()).await.unwrap();
    assert!(closed_within(&mut t, Duration::from_secs(3))
        .await
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_never_reads_cannot_grow_the_outbound_queue() {
    // C17: every ping gets a pong; a client that floods pings and reads
    // nothing must be disconnected instead of buffering pongs without limit.
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let s = start(mock_cfg("x")).await;
    let mut ws = session(s.addr()).await;
    assert_eq!(s.open_connections(), 1);
    let ping = murmur_protocol::ClientMsg::Ping { n: 1 }.to_json();
    let t0 = Instant::now();
    'flood: for _ in 0..2000 {
        for _ in 0..500 {
            if ws.feed(Message::Text(ping.clone().into())).await.is_err() {
                break 'flood;
            }
        }
        if ws.flush().await.is_err() || s.open_connections() == 0 {
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(60));
    }
    let t1 = Instant::now();
    while s.open_connections() > 0 {
        assert!(
            t1.elapsed() < Duration::from_secs(5),
            "connection still open with an unread backlog"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
