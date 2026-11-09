//! Every WebSocket connection leaves exactly one `disconnected` log line,
//! whichever way it ends, with its key, client, audio bytes and reason, and
//! never the transcript. Connections that produced no result (seen from a
//! PC on 2026-11-01/02 with nothing else in the log) are explained by it.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::*;
use flow_core::config::SessionConfig;
use flow_protocol::{ClientMsg, Hello, ServerMsg};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::MaybeTlsStream;

const TEXT: &str = "the quick brown fox";

/// Everything the server logs in this test binary.
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        #[derive(Clone)]
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
        let buf = Arc::new(Mutex::new(Vec::new()));
        let w = Buf(buf.clone());
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || w.clone())
            .init();
        buf
    })
}

async fn server(f: impl FnOnce(&mut flow_server::Config)) -> flow_server::ServerHandle {
    logs();
    let mut c = mock_cfg(TEXT);
    f(&mut c);
    start(c).await
}

fn local_addr(ws: &Ws) -> SocketAddr {
    match ws.get_ref() {
        MaybeTlsStream::Plain(t) => t.local_addr().unwrap(),
        _ => unreachable!("tests connect over plain TCP"),
    }
}

/// Logged lines with `msg` from `peer`.
fn lines(msg: &str, peer: SocketAddr) -> Vec<String> {
    let needle = format!("peer={peer} ");
    String::from_utf8_lossy(&logs().lock().unwrap())
        .lines()
        .filter(|l| l.contains(msg) && l.contains(&needle))
        .map(str::to_string)
        .collect()
}

/// The `session ended` lines for `peer`, in order (call after `end_line`).
fn session_lines(peer: SocketAddr) -> Vec<String> {
    let v = lines(" session ended ", peer);
    for l in &v {
        assert!(!l.contains(TEXT), "no transcript: {l}");
    }
    v
}

/// The `disconnected` line for the connection from `peer`, after checking
/// there is exactly one (waits up to 3 s for it, then 300 ms for a second).
async fn end_line(peer: SocketAddr) -> String {
    let find = || lines(" disconnected ", peer);
    let t0 = Instant::now();
    while find().is_empty() && t0.elapsed() < Duration::from_secs(3) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let lines = find();
    assert_eq!(
        lines.len(),
        1,
        "one disconnected line for {peer}: {lines:?}"
    );
    let line = lines.into_iter().next().unwrap();
    assert!(!line.contains(TEXT), "no transcript: {line}");
    line
}

async fn hello_as(ws: &mut Ws, client: &str) {
    send(
        ws,
        &ClientMsg::Hello(Hello {
            client: client.into(),
            ..Default::default()
        }),
    )
    .await;
    assert!(matches!(recv(ws).await, Some(ServerMsg::HelloOk(_))));
}

async fn start_session(ws: &mut Ws, sid: u64) {
    send(
        ws,
        &ClientMsg::Start {
            sid,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert!(matches!(recv(ws).await, Some(ServerMsg::Started { .. })));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_completed_dictation_logs_result_and_audio_bytes() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test 9.9.9").await;
    start_session(&mut ws, 1).await;
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 1, .. })
    ));
    ws.close(None).await.unwrap();
    let line = end_line(peer).await;
    assert!(line.contains("client=\"flow-test 9.9.9\""), "{line}");
    assert!(line.contains("key=\"k-"), "the owner key id: {line}");
    // 500 ms at 16 kHz, 2 bytes a sample.
    assert!(line.contains("audio_bytes=16000"), "{line}");
    assert!(
        line.contains("sessions=1 results=1 errors=0 cancels=0"),
        "{line}"
    );
    assert!(
        session_lines(peer).is_empty(),
        "a result needs no session line"
    );
    assert!(!line.contains("open_sid"), "{line}");
    assert!(line.contains("reason=\"client_closed\""), "{line}");
    assert!(line.contains("outcome=\"result\""), "{line}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_closes_mid_session_logs_the_open_session() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test").await;
    start_session(&mut ws, 4).await;
    ws.close(None).await.unwrap();
    let line = end_line(peer).await;
    assert!(line.contains("audio_bytes=0"), "{line}");
    assert!(line.contains("open_sid=4"), "{line}");
    assert!(line.contains("reason=\"client_closed\""), "{line}");
    assert!(line.contains("errors=1"), "{line}");
    assert!(line.contains("outcome=\"errors\""), "{line}");
    let s = session_lines(peer);
    assert_eq!(s.len(), 1, "{s:?}");
    assert!(s[0].contains("sid=4 "), "{s:?}");
    assert!(s[0].contains("reason=\"disconnect\""), "{s:?}");
    assert!(s[0].contains("audio_bytes=0"), "{s:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_authenticated_but_idle_client_is_logged_as_idle() {
    let s = server(|c| c.idle_timeout_s = 1).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test").await;
    // Keep reading (answering pings) until the server closes.
    while tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("closed within 5 s")
        .is_some_and(|m| m.is_ok())
    {}
    let line = end_line(peer).await;
    assert!(line.contains("reason=\"idle\""), "{line}");
    assert!(line.contains("msgs=0 "), "{line}");
    assert!(line.contains("sessions=0"), "{line}");
    assert!(line.contains("outcome=\"no_audio\""), "{line}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silent_peer_is_logged_as_dead() {
    let s = server(|c| {
        c.ws_ping_ms = 100;
        c.ws_dead_ms = 400;
    })
    .await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test").await;
    start_session(&mut ws, 2).await;
    // `ws` is not polled again, so pings go unanswered.
    let line = end_line(peer).await;
    assert!(line.contains("reason=\"dead_peer\""), "{line}");
    assert!(line.contains("open_sid=2"), "{line}");
    drop(ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connection_without_hello_is_logged() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    ws.send(Message::Text("not json".into())).await.unwrap();
    let line = end_line(peer).await;
    assert!(line.contains("client=\"-\""), "{line}");
    assert!(line.contains("reason=\"no_hello\""), "{line}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_protocol_mismatch_is_logged_with_the_client() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    send(
        &mut ws,
        &ClientMsg::Hello(Hello {
            proto_min: 900,
            proto_max: 901,
            client: "flow-future 99.0.0".into(),
            ..Default::default()
        }),
    )
    .await;
    let line = end_line(peer).await;
    assert!(line.contains("client=\"flow-future 99.0.0\""), "{line}");
    assert!(line.contains("reason=\"proto_mismatch\""), "{line}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_shutdown_is_logged() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test").await;
    send_audio(&mut ws, &speech(100)).await;
    // Let the server read the audio before it stops.
    send(&mut ws, &ClientMsg::Ping { n: 1 }).await;
    assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Pong { .. })));
    s.kill();
    let line = end_line(peer).await;
    assert!(line.contains("reason=\"shutdown\""), "{line}");
    assert!(line.contains("audio_bytes=3200"), "{line}");
    assert!(line.contains("outcome=\"no_result\""), "{line}");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_sessions_beside_a_result_each_get_a_line() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "flow-test").await;
    start_session(&mut ws, 1).await;
    send_audio(&mut ws, &speech(100)).await;
    start_session(&mut ws, 2).await; // replaces 1
    send(&mut ws, &ClientMsg::Cancel { sid: 2 }).await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Cancelled { sid: 2 })
    ));
    // Refused at start: an LLM budget over the server limit.
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 3,
            config: SessionConfig {
                llm_timeout_ms: u64::MAX,
                ..Default::default()
            },
        },
    )
    .await;
    assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Error(_))));
    start_session(&mut ws, 4).await;
    send_audio(&mut ws, &speech(200)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 4 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 4, .. })
    ));
    ws.close(None).await.unwrap();

    let line = end_line(peer).await;
    assert!(
        line.contains("sessions=4 results=1 errors=1 cancels=2"),
        "{line}"
    );
    assert!(
        line.contains("outcome=\"errors\""),
        "not hidden by the result: {line}"
    );
    let s = session_lines(peer);
    assert_eq!(s.len(), 3, "one line per session without a result: {s:?}");
    assert!(
        s[0].contains("sid=1 ") && s[0].contains("reason=\"replaced\""),
        "{s:?}"
    );
    assert!(s[0].contains("audio_bytes=3200"), "{s:?}");
    assert!(
        s[1].contains("sid=2 ") && s[1].contains("reason=\"cancelled\""),
        "{s:?}"
    );
    assert!(
        s[2].contains("sid=3 ") && s[2].contains("reason=\"too_large\""),
        "{s:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newline_in_the_client_string_cannot_forge_a_line() {
    let s = server(|_| {}).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let peer = local_addr(&ws);
    hello_as(&mut ws, "evil\nINFO flow_server::ws: forged peer=1.2.3.4:5").await;
    send(&mut ws, &ClientMsg::Ping { n: 1 }).await;
    assert!(matches!(recv(&mut ws).await, Some(ServerMsg::Pong { .. })));
    ws.close(None).await.unwrap();
    let line = end_line(peer).await;
    assert!(line.contains("client=\"evil\\nINFO"), "{line}");
    assert!(
        line.contains("msgs=1 "),
        "a probe's pings are counted: {line}"
    );
    let log = String::from_utf8_lossy(&logs().lock().unwrap()).to_string();
    assert!(
        !log.lines()
            .any(|l| l.starts_with("INFO flow_server::ws: forged")),
        "no forged line"
    );
}
