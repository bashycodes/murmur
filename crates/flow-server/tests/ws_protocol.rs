//! Protocol behaviour of flow-server against a raw WebSocket client.

mod common;

use std::sync::Arc;

use common::*;
use flow_core::asr::MockAsr;
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::FlowCore;
use flow_protocol::{code, ClientMsg, Hello, ServerMsg};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

const TEXT: &str = "um so zephyr is great press enter";

fn session_cfg() -> SessionConfig {
    SessionConfig {
        dictionary: serde_json::from_str(r#"[{"word":"zephyr","replacement":"Zephyr"}]"#).unwrap(),
        ..Default::default()
    }
}

fn ctx() -> FieldContext {
    FieldContext {
        app_id: "com.slack".into(),
        before: "Hi ".into(),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_ok_describes_the_server() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let h = hello(&mut ws).await;
    assert_eq!(h.proto, 1);
    // D26: product version + commit (`flow-server 0.2.0+abc1234[.dirty]`); `core`
    // stays the plain version (informational; compatibility is `proto`).
    assert_eq!(h.server, format!("flow-server {}", flow_build_info::FULL));
    assert!(
        h.server
            .starts_with(&format!("flow-server {}+", flow_core::VERSION)),
        "{}",
        h.server
    );
    assert_eq!(h.core, flow_core::VERSION);
    assert_eq!(flow_build_info::VERSION, flow_core::VERSION);
    assert_eq!(h.postprocess_rev, flow_core::POSTPROCESS_REV);
    assert_eq!(h.asr_backend, "mock");
    assert_eq!(h.workers, 1);
    assert_eq!(h.busy, 0);
    assert!(!h.llm);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_result_equals_flowcore_directly() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 1,
            config: session_cfg(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut ws).await,
        Some(ServerMsg::Started {
            sid: 1,
            queued: false
        })
    );
    send_audio(&mut ws, &speech(1000)).await;
    send(&mut ws, &ClientMsg::Context { sid: 1, ctx: ctx() }).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    let Some(ServerMsg::Result {
        sid,
        result,
        server_timings,
    }) = recv_final(&mut ws).await
    else {
        panic!("no result")
    };
    assert_eq!(sid, 1);
    assert_eq!(server_timings.recv_audio_ms, 1000);
    assert!(result.route.is_none(), "the server never sets route");

    let core = FlowCore::from_engines(Arc::new(MockAsr::new(TEXT)), None);
    core.start_session(session_cfg()).unwrap();
    core.push_audio(&speech(1000));
    core.set_context(ctx());
    let mut want = core.commit();
    let mut got = *result;
    want.timings = Default::default();
    got.timings = Default::default();
    assert_eq!(got, want);
    assert!(got.press_enter);
}

#[tokio::test(flavor = "multi_thread")]
async fn partials_are_pushed_when_enabled() {
    let s = start(mock_cfg("ask not")).await;
    let mut ws = session(s.addr()).await;
    let cfg = SessionConfig {
        partial_interval_ms: Some(200),
        ..Default::default()
    };
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 3,
            config: cfg,
        },
    )
    .await;
    recv(&mut ws).await;
    send_audio(&mut ws, &speech(1000)).await;
    let mut got = None;
    for _ in 0..20 {
        if let Some(ServerMsg::Partial { sid, text }) = recv(&mut ws).await {
            got = Some((sid, text));
            break;
        }
    }
    assert_eq!(got, Some((3, "ask not".to_string())));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_then_commit_is_no_session() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut ws).await;
    send_audio(&mut ws, &speech(300)).await;
    send(&mut ws, &ClientMsg::Cancel { sid: 1 }).await;
    assert_eq!(recv(&mut ws).await, Some(ServerMsg::Cancelled { sid: 1 }));
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    match recv(&mut ws).await {
        Some(ServerMsg::Error(e)) => {
            assert_eq!(e.code, code::NO_SESSION);
            assert!(!e.fatal);
        }
        other => panic!("{other:?}"),
    }
    // The worker was returned: a new session starts unqueued.
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 2,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut ws).await,
        Some(ServerMsg::Started {
            sid: 2,
            queued: false
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_sid_and_bad_frames_are_non_fatal() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 5,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut ws).await;
    send(&mut ws, &ClientMsg::Commit { sid: 4 }).await;
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(
        (e.code.as_str(), e.sid, e.fatal),
        (code::STALE_SID, Some(4), false)
    );

    ws.send(Message::Text(r#"{"t":"teleport"}"#.into()))
        .await
        .unwrap();
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(e.code, code::BAD_REQUEST);

    ws.send(Message::Binary(vec![1u8, 2, 3].into()))
        .await
        .unwrap();
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(e.code, code::BAD_REQUEST);

    ws.send(Message::Binary(vec![0u8; 70_000].into()))
        .await
        .unwrap();
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(e.code, code::TOO_LARGE);

    send(&mut ws, &ClientMsg::Ping { n: 9 }).await;
    assert_eq!(recv(&mut ws).await, Some(ServerMsg::Pong { n: 9, busy: 1 }));

    // The session still works.
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 5 }).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 5, .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_start_replaces_the_active_session() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut ws).await;
    send_audio(&mut ws, &speech(2000)).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 2,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut ws).await,
        Some(ServerMsg::Started {
            sid: 2,
            queued: false
        })
    );
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 2 }).await;
    let Some(ServerMsg::Result {
        sid,
        server_timings,
        ..
    }) = recv_final(&mut ws).await
    else {
        panic!()
    };
    assert_eq!(sid, 2);
    assert_eq!(
        server_timings.recv_audio_ms, 500,
        "old session's audio not counted"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audio_without_a_session_is_dropped_silently() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send_audio(&mut ws, &speech(200)).await;
    send(&mut ws, &ClientMsg::Ping { n: 1 }).await;
    assert_eq!(recv(&mut ws).await, Some(ServerMsg::Pong { n: 1, busy: 0 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn proto_mismatch_is_fatal_with_close_4002() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    send(
        &mut ws,
        &ClientMsg::Hello(Hello {
            proto_min: 2,
            proto_max: 3,
            ..Default::default()
        }),
    )
    .await;
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(e.code, code::PROTO_MISMATCH);
    assert!(e.fatal);
    let close = loop {
        match ws.next().await {
            Some(Ok(Message::Close(f))) => break f,
            Some(Ok(_)) => continue,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(u16::from(close.unwrap().code), 4002);
}

#[tokio::test(flavor = "multi_thread")]
async fn something_before_hello_is_fatal() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    send(&mut ws, &ClientMsg::Ping { n: 1 }).await;
    let Some(ServerMsg::Error(e)) = recv(&mut ws).await else {
        panic!()
    };
    assert_eq!(e.code, code::BAD_REQUEST);
    assert!(e.fatal);
    assert_eq!(recv(&mut ws).await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn health_endpoint_returns_hello_ok() {
    let s = start(mock_cfg(TEXT)).await;
    let (status, body) = http_get(s.addr(), "/v1/health", Some(TOKEN)).await;
    assert_eq!(status, 200);
    let h: flow_protocol::HelloOk = serde_json::from_str(&body).unwrap();
    assert_eq!(h.asr_backend, "mock");
    assert_eq!(h.proto, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_drops_connections_and_stops_accepting() {
    let s = start(mock_cfg(TEXT)).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut ws).await;
    s.kill();
    assert_eq!(recv(&mut ws).await, None);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(connect(s.addr(), Some(TOKEN)).await.is_err());
    assert_eq!(s.connections(), 1);
}
