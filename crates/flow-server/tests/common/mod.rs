//! Raw-protocol test client for flow-server (mock ASR, in-process server).
#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use flow_core::config::{AsrBackend, ModelConfig};
use flow_protocol::{ClientMsg, Hello, HelloOk, ServerMsg};
use flow_server::{Config, ServerHandle};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub const TOKEN: &str = "test-token-0123456789abcdef";
pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub fn mock_cfg(text: &str) -> Config {
    Config {
        bind: vec!["127.0.0.1:0".into()],
        token: Some(TOKEN.into()),
        model: ModelConfig {
            asr_backend: AsrBackend::Mock { text: text.into() },
            ..Default::default()
        },
        ..Default::default()
    }
}

pub async fn start(cfg: Config) -> ServerHandle {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    flow_server::serve(cfg, vec![l]).await.unwrap()
}

pub fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

pub async fn connect(
    addr: SocketAddr,
    token: Option<&str>,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/v1/stream")
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("flow.v1"),
    );
    if let Some(t) = token {
        req.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {t}")).unwrap(),
        );
    }
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

pub async fn send(ws: &mut Ws, m: &ClientMsg) {
    ws.send(Message::Text(m.to_json().into())).await.unwrap();
}

pub async fn send_audio(ws: &mut Ws, pcm: &[i16]) {
    for c in pcm.chunks(1600) {
        ws.send(Message::Binary(flow_protocol::encode_pcm(c).into()))
            .await
            .unwrap();
    }
}

/// Next control message (5 s timeout); `None` when the server closed.
pub async fn recv(ws: &mut Ws) -> Option<ServerMsg> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("server reply within 5 s");
        match m {
            Some(Ok(Message::Text(t))) => return Some(serde_json::from_str(&t).unwrap()),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            _ => {}
        }
    }
}

/// Next message that is not a partial.
pub async fn recv_final(ws: &mut Ws) -> Option<ServerMsg> {
    loop {
        match recv(ws).await {
            Some(ServerMsg::Partial { .. }) => continue,
            other => return other,
        }
    }
}

pub async fn hello(ws: &mut Ws) -> HelloOk {
    send(
        ws,
        &ClientMsg::Hello(Hello {
            client: "test".into(),
            ..Default::default()
        }),
    )
    .await;
    match recv(ws).await {
        Some(ServerMsg::HelloOk(h)) => h,
        other => panic!("expected hello_ok, got {other:?}"),
    }
}

pub async fn session(addr: SocketAddr) -> Ws {
    let mut ws = connect(addr, Some(TOKEN)).await.unwrap();
    hello(&mut ws).await;
    ws
}

/// Plain HTTP GET; returns (status, body).
pub async fn http_get(addr: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = TcpStream::connect(addr).await.unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}
