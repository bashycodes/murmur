//! Worker leasing and the queue (§3.3).

mod common;

use std::time::Duration;

use common::*;
use flow_core::config::SessionConfig;
use flow_protocol::{code, ClientMsg, ServerMsg};

fn cfg(max_queue: usize, queue_timeout_ms: u64) -> flow_server::Config {
    flow_server::Config {
        max_queue,
        queue_timeout_ms,
        ..mock_cfg("queued words")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn second_session_queues_third_is_busy_and_queued_audio_is_replayed() {
    let s = start(cfg(1, 10_000)).await;
    let mut a = session(s.addr()).await;
    let mut b = session(s.addr()).await;
    let mut c = session(s.addr()).await;
    send(
        &mut a,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut a).await,
        Some(ServerMsg::Started {
            sid: 1,
            queued: false
        })
    );
    send(
        &mut b,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut b).await,
        Some(ServerMsg::Started {
            sid: 1,
            queued: true
        })
    );
    send(
        &mut c,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    let Some(ServerMsg::Error(e)) = recv(&mut c).await else {
        panic!()
    };
    assert_eq!(e.code, code::BUSY);
    assert!(!e.fatal);

    // B speaks and commits while still queued.
    send_audio(&mut b, &speech(700)).await;
    send(&mut b, &ClientMsg::Commit { sid: 1 }).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A finishes; its worker goes to B.
    send_audio(&mut a, &speech(300)).await;
    send(&mut a, &ClientMsg::Commit { sid: 1 }).await;
    assert!(matches!(
        recv_final(&mut a).await,
        Some(ServerMsg::Result { .. })
    ));
    let Some(ServerMsg::Result {
        result,
        server_timings,
        ..
    }) = recv_final(&mut b).await
    else {
        panic!()
    };
    assert_eq!(result.timings.audio_ms, 700, "all queued audio replayed");
    assert_eq!(result.asr_text, "queued words");
    assert!(server_timings.queue_ms >= 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn queue_timeout_is_busy() {
    let s = start(cfg(2, 300)).await;
    let mut a = session(s.addr()).await;
    let mut b = session(s.addr()).await;
    send(
        &mut a,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    recv(&mut a).await;
    send(
        &mut b,
        &ClientMsg::Start {
            sid: 9,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut b).await,
        Some(ServerMsg::Started {
            sid: 9,
            queued: true
        })
    );
    let Some(ServerMsg::Error(e)) = recv(&mut b).await else {
        panic!()
    };
    assert_eq!((e.code.as_str(), e.sid), (code::BUSY, Some(9)));
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnect_returns_the_worker() {
    let s = start(cfg(0, 1000)).await;
    {
        let mut a = session(s.addr()).await;
        send(
            &mut a,
            &ClientMsg::Start {
                sid: 1,
                config: SessionConfig::default(),
            },
        )
        .await;
        recv(&mut a).await;
        send_audio(&mut a, &speech(300)).await;
    }
    // A dropped without commit/cancel.
    let mut b = session(s.addr()).await;
    let mut ok = false;
    for _ in 0..50 {
        send(
            &mut b,
            &ClientMsg::Start {
                sid: 2,
                config: SessionConfig::default(),
            },
        )
        .await;
        match recv(&mut b).await {
            Some(ServerMsg::Started { queued: false, .. }) => {
                ok = true;
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    assert!(ok, "worker freed after disconnect");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_busy_hook_queues_new_sessions() {
    let s = start(cfg(2, 10_000)).await;
    s.set_busy(true);
    let mut a = session(s.addr()).await;
    send(
        &mut a,
        &ClientMsg::Start {
            sid: 1,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert_eq!(
        recv(&mut a).await,
        Some(ServerMsg::Started {
            sid: 1,
            queued: true
        })
    );
    send(&mut a, &ClientMsg::Ping { n: 1 }).await;
    assert_eq!(recv(&mut a).await, Some(ServerMsg::Pong { n: 1, busy: 1 }));
}
