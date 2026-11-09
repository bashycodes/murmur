//! S8: client-supplied sizes are bounded before they reach murmur-core.

mod common;

use common::*;
use murmur_core::config::{FieldContext, SessionConfig, SAMPLE_RATE};
use murmur_protocol::{code, ClientMsg, ServerMsg};

async fn start_with(ws: &mut Ws, sid: u64, config: SessionConfig) -> Option<ServerMsg> {
    send(ws, &ClientMsg::Start { sid, config }).await;
    recv(ws).await
}

fn too_large(m: Option<ServerMsg>, sid: u64) {
    match m {
        Some(ServerMsg::Error(e)) => {
            assert_eq!(e.code, code::TOO_LARGE, "{e:?}");
            assert_eq!(e.sid, Some(sid));
            assert!(!e.fatal);
        }
        other => panic!("expected too_large, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_ring_is_rejected_and_the_server_survives() {
    let s = start(mock_cfg("x")).await;
    let mut ws = session(s.addr()).await;
    let huge = SessionConfig {
        ring_capacity_samples: 1 << 34,
        ..Default::default()
    };
    too_large(start_with(&mut ws, 1, huge).await, 1);
    assert_eq!(s.pool().busy(), 0, "no worker leased for a rejected start");
    // Same connection, sane config: still served.
    assert_eq!(
        start_with(&mut ws, 2, SessionConfig::default()).await,
        Some(ServerMsg::Started {
            sid: 2,
            queued: false
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ring_up_to_the_session_cap_plus_replay_headroom_is_accepted() {
    // The router enlarges the ring to replay buffered audio after a
    // fallback; that must keep working.
    let cfg = murmur_server::Config {
        max_session_s: 60,
        ..mock_cfg("x")
    };
    let s = start(cfg).await;
    let mut ws = session(s.addr()).await;
    let ok = SessionConfig {
        ring_capacity_samples: 90 * SAMPLE_RATE,
        ..Default::default()
    };
    assert!(matches!(
        start_with(&mut ws, 1, ok).await,
        Some(ServerMsg::Started { sid: 1, .. })
    ));
    let over = SessionConfig {
        ring_capacity_samples: 90 * SAMPLE_RATE + 1,
        ..Default::default()
    };
    too_large(start_with(&mut ws, 2, over).await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn max_samples_is_clamped_to_the_server_limit_and_advertised() {
    let cfg = murmur_server::Config {
        max_session_s: 2,
        ..mock_cfg("x")
    };
    let s = start(cfg).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let h = hello(&mut ws).await;
    assert_eq!(
        h.max_samples,
        2 * SAMPLE_RATE as u64,
        "hello_ok reports the server limit"
    );
    let (status, body) = http_get(s.addr(), "/v1/health", Some(TOKEN)).await;
    assert_eq!(status, 200);
    let h: murmur_protocol::HelloOk = serde_json::from_str(&body).unwrap();
    assert_eq!(h.max_samples, 2 * SAMPLE_RATE as u64);
    // An absurd client value is clamped, not trusted.
    let big = SessionConfig {
        max_samples: usize::MAX,
        ..Default::default()
    };
    assert!(matches!(
        start_with(&mut ws, 1, big).await,
        Some(ServerMsg::Started { sid: 1, .. })
    ));
    send_audio(&mut ws, &speech(1500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 1, .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_context_and_llm_budget_are_rejected() {
    let s = start(mock_cfg("x")).await;
    let mut ws = session(s.addr()).await;
    let slow = SessionConfig {
        llm_timeout_ms: 10 * 60 * 1000,
        ..Default::default()
    };
    too_large(start_with(&mut ws, 1, slow).await, 1);
    assert!(matches!(
        start_with(&mut ws, 2, SessionConfig::default()).await,
        Some(ServerMsg::Started { sid: 2, .. })
    ));
    let ctx = FieldContext {
        before: "a".repeat(20_000),
        ..Default::default()
    };
    send(&mut ws, &ClientMsg::Context { sid: 2, ctx }).await;
    too_large(recv(&mut ws).await, 2);
    let ctx = FieldContext {
        screen_words: vec!["w".into(); 5000],
        ..Default::default()
    };
    send(&mut ws, &ClientMsg::Context { sid: 2, ctx }).await;
    too_large(recv(&mut ws).await, 2);
    // A normal context is fine and the session is still usable.
    send(
        &mut ws,
        &ClientMsg::Context {
            sid: 2,
            ctx: FieldContext {
                before: "Hi ".into(),
                ..Default::default()
            },
        },
    )
    .await;
    send_audio(&mut ws, &speech(300)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 2 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 2, .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_websocket_frame_over_the_limit_closes_the_connection() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let s = start(mock_cfg("x")).await;
    let mut ws = session(s.addr()).await;
    let _ = ws.send(Message::Text("x".repeat(1 << 20).into())).await;
    assert_eq!(recv(&mut ws).await, None, "server closed on a 1 MiB frame");
}
