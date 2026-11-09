//! RemoteBackend against a minimal in-test WebSocket server (no murmur-server):
//! cancel during commit (review C4, client side) and the resolved-address
//! cleartext rule (review S6).
#![allow(clippy::result_large_err)] // tungstenite's handshake callback signature

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use murmur_core::config::SessionConfig;
use murmur_protocol::{ClientMsg, HelloOk, ServerMsg, OLD_SUBPROTOCOL, SUBPROTOCOL};
use murmur_remote::RemoteBackend;
use murmur_router::{Backend, Failure, ServerConfig};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;

/// Says hello_ok and started, never answers a commit, records what it got.
struct SilentServer {
    port: u16,
    got: Arc<Mutex<Vec<String>>>,
    _rt: tokio::runtime::Runtime,
}

impl SilentServer {
    fn start() -> Self {
        Self::start_speaking(SUBPROTOCOL)
    }

    /// A server that answers the upgrade with `subprotocol`.
    fn start_speaking(subprotocol: &'static str) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let l = rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let port = l.local_addr().unwrap().port();
        let g2 = got.clone();
        rt.spawn(async move {
            while let Ok((tcp, _)) = l.accept().await {
                let g3 = g2.clone();
                tokio::spawn(async move {
                    let cb = |_: &Request, mut resp: Response| {
                        resp.headers_mut().insert(
                            "sec-websocket-protocol",
                            HeaderValue::from_static(subprotocol),
                        );
                        Ok(resp)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(tcp, cb).await else {
                        return;
                    };
                    while let Some(Ok(m)) = ws.next().await {
                        let Message::Text(t) = m else { continue };
                        let Ok(msg) = serde_json::from_str::<ClientMsg>(&t) else {
                            continue;
                        };
                        let reply = match &msg {
                            ClientMsg::Hello(_) => Some(ServerMsg::HelloOk(HelloOk {
                                proto: 1,
                                workers: 1,
                                ..Default::default()
                            })),
                            ClientMsg::Start { sid, .. } => Some(ServerMsg::Started {
                                sid: *sid,
                                queued: false,
                            }),
                            _ => None,
                        };
                        g3.lock().unwrap().push(t.to_string());
                        if let Some(r) = reply {
                            let _ = ws.send(Message::Text(r.to_json().into())).await;
                        }
                    }
                });
            }
        });
        Self { port, got, _rt: rt }
    }

    fn count(&self, kind: &str) -> usize {
        let tag = format!("\"t\":\"{kind}\"");
        self.got
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.contains(&tag))
            .count()
    }

    fn got(&self, kind: &str) -> bool {
        let tag = format!("\"t\":\"{kind}\"");
        self.got.lock().unwrap().iter().any(|m| m.contains(&tag))
    }
}

fn server_cfg(host: &str, port: u16) -> ServerConfig {
    ServerConfig {
        host: host.into(),
        port,
        token: "client-test-token-0123456789".into(),
        // The harness's handshake deadline, not the product default (2500 ms):
        // a loaded box running the workspace tests in parallel misses short ones
        // on a loopback handshake. These tests are about cancel and the
        // address rule, not connect latency.
        connect_timeout_ms: 10_000,
        ..Default::default()
    }
}

#[test]
fn cancel_during_commit_sends_cancel_and_stops_waiting() {
    let srv = SilentServer::start();
    let b = Arc::new(RemoteBackend::new(
        server_cfg("127.0.0.1", srv.port),
        "test",
    ));
    b.start_session(&SessionConfig::default()).unwrap();
    assert!(b.push_audio(&[100i16; 1600]));
    let done = Arc::new(AtomicBool::new(false));
    let (b2, d2) = (b.clone(), done.clone());
    let h = std::thread::spawn(move || {
        let t = Instant::now();
        let r = b2.commit(Instant::now() + Duration::from_secs(20));
        d2.store(true, Ordering::SeqCst);
        (r, t.elapsed())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(!done.load(Ordering::SeqCst), "server never answers");
    b.cancel();
    let (r, took) = h.join().unwrap();
    assert!(
        took < Duration::from_secs(3),
        "commit kept waiting {took:?}"
    );
    assert!(r.is_err());
    let t0 = Instant::now();
    while !srv.got("cancel") && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(srv.got("cancel"), "server worker never told to stop");
}

#[test]
fn cleartext_to_a_name_is_decided_by_its_resolved_addresses() {
    let srv = SilentServer::start();
    // `localhost` resolves to loopback only: allowed.
    let ok = RemoteBackend::new(server_cfg("localhost", srv.port), "test");
    ok.start_session(&SessionConfig::default())
        .expect("loopback name allowed");
    ok.cancel();
    // A single-label name no longer counts as private by its name: if it
    // does not resolve, nothing is sent and the reason says so.
    let nx = RemoteBackend::new(server_cfg("murmur-no-such-host-zz9", srv.port), "test");
    let e = nx.start_session(&SessionConfig::default()).unwrap_err();
    assert_eq!(e.kind, Failure::ConnectFailed);
    assert!(
        e.message.contains("resolve") || e.message.contains("enable TLS"),
        "{}",
        e.message
    );
}

/// D83: a server from before the Murmur rename answers with the old
/// subprotocol name; the client still offers it, so the session starts.
#[test]
fn a_server_speaking_the_old_subprotocol_name_still_works() {
    let srv = SilentServer::start_speaking(OLD_SUBPROTOCOL);
    let b = RemoteBackend::new(server_cfg("127.0.0.1", srv.port), "test");
    b.start_session(&SessionConfig::default())
        .expect("old server accepted");
    assert!(srv.got("hello"));
    b.cancel();
}

#[test]
fn warm_opens_the_connection_the_next_session_reuses() {
    let srv = SilentServer::start();
    let b = RemoteBackend::new(server_cfg("127.0.0.1", srv.port), "test");
    b.warm().expect("warm connects");
    b.warm().expect("second warm reuses");
    b.start_session(&SessionConfig::default()).unwrap();
    let t0 = Instant::now();
    while !srv.got("start") && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        srv.count("hello"),
        1,
        "one handshake for warm + warm + start"
    );
    b.cancel();
    // Unreachable server: warm fails with connect_failed within the timeout.
    let mut c = server_cfg("127.0.0.1", 9);
    c.connect_timeout_ms = 300;
    let e = RemoteBackend::new(c, "test").warm().unwrap_err();
    assert_eq!(e.kind, Failure::ConnectFailed);
}
