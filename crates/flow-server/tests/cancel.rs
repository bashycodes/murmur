//! C4 (server side): `cancel` while a commit is running.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use flow_core::asr::MockAsr;
use flow_core::config::SessionConfig;
use flow_core::FlowCore;
use flow_protocol::{ClientMsg, ServerMsg};
use flow_server::{Pool, ServerHandle};

const COMMIT: Duration = Duration::from_millis(1500);

/// One worker whose final decode takes `COMMIT`.
async fn slow_server() -> ServerHandle {
    let cfg = mock_cfg("slow words");
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new("slow words").with_delay(COMMIT)),
        None,
    );
    let pool = Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    flow_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
        .await
        .unwrap()
}

async fn start_and_commit(ws: &mut Ws, sid: u64) {
    send(
        ws,
        &ClientMsg::Start {
            sid,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert!(matches!(recv(ws).await, Some(ServerMsg::Started { .. })));
    send_audio(ws, &speech(500)).await;
    send(ws, &ClientMsg::Commit { sid }).await;
    tokio::time::sleep(Duration::from_millis(100)).await; // commit is running
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_commit_answers_at_once_and_drops_the_result() {
    let s = slow_server().await;
    let mut ws = session(s.addr()).await;
    start_and_commit(&mut ws, 1).await;
    let t0 = Instant::now();
    send(&mut ws, &ClientMsg::Cancel { sid: 1 }).await;
    assert_eq!(recv(&mut ws).await, Some(ServerMsg::Cancelled { sid: 1 }));
    assert!(
        t0.elapsed() < COMMIT / 2,
        "cancelled without waiting for the job"
    );
    // The next session on this connection is served as soon as the worker
    // is back, and the cancelled session's result never arrives.
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 2,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Started { sid: 2, .. })
    ));
    send_audio(&mut ws, &speech(300)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 2 }).await;
    let got = match recv_final(&mut ws).await {
        Some(ServerMsg::Result { sid, .. }) => Some(sid),
        other => panic!("{other:?}"),
    };
    assert_eq!(got, Some(2), "only the live session gets a result");
    assert_eq!(s.pool().busy(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_after_the_job_never_touches_the_next_lessee() {
    // The worker returns to the pool after the cancelled commit and another
    // connection leases it; a late cancel/disconnect from the first
    // connection must not cancel the second one's session.
    let s = slow_server().await;
    let mut a = session(s.addr()).await;
    start_and_commit(&mut a, 1).await;
    tokio::time::sleep(COMMIT).await; // job done, result delivered
    assert!(matches!(
        recv_final(&mut a).await,
        Some(ServerMsg::Result { sid: 1, .. })
    ));
    let mut b = session(s.addr()).await;
    send(
        &mut b,
        &ClientMsg::Start {
            sid: 9,
            config: SessionConfig::default(),
        },
    )
    .await;
    assert!(matches!(
        recv(&mut b).await,
        Some(ServerMsg::Started {
            sid: 9,
            queued: false
        })
    ));
    send(&mut a, &ClientMsg::Cancel { sid: 1 }).await;
    assert_eq!(recv(&mut a).await, Some(ServerMsg::Cancelled { sid: 1 }));
    drop(a);
    send_audio(&mut b, &speech(300)).await;
    send(&mut b, &ClientMsg::Commit { sid: 9 }).await;
    let r = tokio::time::timeout(Duration::from_secs(5), recv_final(&mut b)).await;
    match r.unwrap() {
        Some(ServerMsg::Result { sid: 9, result, .. }) => {
            assert!(result.final_text.contains("slow words"), "{result:?}")
        }
        other => panic!("{other:?}"),
    }
}

/// Needs flow-core to honour `FlowCore::cancel()` during `commit()` (the
/// core half of C4: today the final decode has no abort flag, so the worker
/// stays busy until the job ends). The server already calls `cancel()` on
/// the in-flight core; un-ignore once flow-core aborts the commit.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs flow-core commit abort (C4 core side)"]
async fn cancel_during_commit_frees_the_worker_promptly() {
    let s = slow_server().await;
    let mut ws = session(s.addr()).await;
    start_and_commit(&mut ws, 1).await;
    send(&mut ws, &ClientMsg::Cancel { sid: 1 }).await;
    assert_eq!(recv(&mut ws).await, Some(ServerMsg::Cancelled { sid: 1 }));
    let t0 = Instant::now();
    while s.pool().busy() > 0 {
        assert!(
            t0.elapsed() < COMMIT / 3,
            "worker still busy with a cancelled job"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

struct PanickingAsr;
impl flow_core::asr::AsrEngine for PanickingAsr {
    fn transcribe(
        &self,
        _: &[f32],
        _: &flow_core::asr::AsrRequest,
    ) -> Result<flow_core::asr::AsrOutput, flow_core::asr::AsrError> {
        panic!("engine crashed (test)")
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_on_a_dead_worker_is_an_error_not_started() {
    let cfg = mock_cfg("x");
    let core = FlowCore::from_engines(Arc::new(PanickingAsr), None);
    let pool = Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = flow_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
        .await
        .unwrap();
    let mut ws = session(s.addr()).await;
    // The first commit kills the core's worker thread.
    start_and_commit(&mut ws, 1).await;
    assert!(matches!(
        recv_final(&mut ws).await,
        Some(ServerMsg::Result { sid: 1, .. })
    ));
    send(
        &mut ws,
        &ClientMsg::Start {
            sid: 2,
            config: SessionConfig::default(),
        },
    )
    .await;
    match recv(&mut ws).await {
        Some(ServerMsg::Error(e)) => {
            assert_eq!(e.code, flow_protocol::code::SERVER_ERROR, "{e:?}");
            assert_eq!(e.sid, Some(2));
        }
        other => panic!("expected server_error, got {other:?}"),
    }
    assert_eq!(s.pool().busy(), 0, "the lease went back");
}
