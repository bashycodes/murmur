//! D35 (server side): text-only `clean` requests.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use flow_core::asr::MockAsr;
use flow_core::cleanup::{CleanError, CleanPrompt, CleanRequest, Cleaner};
use flow_core::config::CleanupLevel;
use flow_core::FlowCore;
use flow_protocol::{ClientMsg, ServerMsg, FEATURE_CLEAN, MAX_CLEAN_TEXT_BYTES};
use flow_server::{Pool, ServerHandle};

/// Upper-cases the raw transcript after `delay`.
struct Upper {
    delay: Duration,
}

impl Cleaner for Upper {
    fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
        std::thread::sleep(self.delay);
        if p.transcript == "fail" {
            return Err(CleanError::Backend("boom".into()));
        }
        Ok(p.transcript.to_uppercase())
    }

    fn model(&self) -> Option<String> {
        Some("fake-llm".into())
    }
}

async fn llm_server(delay: Duration, max_queue: usize) -> ServerHandle {
    let mut cfg = mock_cfg("words");
    cfg.max_queue = max_queue;
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new("words")),
        Some(Arc::new(Upper { delay }) as Arc<dyn Cleaner>),
    );
    let pool = Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    flow_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
        .await
        .unwrap()
}

fn req(text: &str) -> Box<CleanRequest> {
    Box::new(CleanRequest {
        text: text.into(),
        level: CleanupLevel::Light,
        timeout_ms: 2000,
        ..Default::default()
    })
}

async fn cleaned(ws: &mut Ws) -> flow_protocol::Cleaned {
    match recv(ws).await {
        Some(ServerMsg::Cleaned(c)) => c,
        other => panic!("expected cleaned, got {other:?}"),
    }
}

#[tokio::test]
async fn llm_server_advertises_and_cleans() {
    let s = llm_server(Duration::ZERO, 4).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let h = hello(&mut ws).await;
    assert!(h.llm);
    assert!(h.features.iter().any(|f| f == FEATURE_CLEAN), "{h:?}");
    send(
        &mut ws,
        &ClientMsg::Clean {
            id: 9,
            req: req("the demo"),
        },
    )
    .await;
    let c = cleaned(&mut ws).await;
    assert_eq!(c.id, 9);
    assert_eq!(c.text.as_deref(), Some("THE DEMO"));
    assert_eq!(c.error, None);
    assert_eq!(c.model.as_deref(), Some("fake-llm"));
}

#[tokio::test]
async fn backend_error_is_reported() {
    let s = llm_server(Duration::ZERO, 4).await;
    let mut ws = session(s.addr()).await;
    send(
        &mut ws,
        &ClientMsg::Clean {
            id: 1,
            req: req("fail"),
        },
    )
    .await;
    let c = cleaned(&mut ws).await;
    assert_eq!(c.text, None);
    assert!(c.error.unwrap().starts_with("llm_error"));
}

#[tokio::test]
async fn slow_llm_times_out() {
    let s = llm_server(Duration::from_millis(1500), 4).await;
    let mut ws = session(s.addr()).await;
    let mut r = req("slow");
    r.timeout_ms = 200;
    send(&mut ws, &ClientMsg::Clean { id: 2, req: r }).await;
    let c = cleaned(&mut ws).await;
    assert_eq!(c.error.as_deref(), Some("llm_timeout"));
    assert!(c.llm_ms >= 200 && c.llm_ms < 1500, "{c:?}");
}

#[tokio::test]
async fn no_llm_server_refuses() {
    let s = start(mock_cfg("words")).await;
    let mut ws = connect(s.addr(), Some(TOKEN)).await.unwrap();
    let h = hello(&mut ws).await;
    assert!(!h.features.iter().any(|f| f == FEATURE_CLEAN));
    send(
        &mut ws,
        &ClientMsg::Clean {
            id: 3,
            req: req("x"),
        },
    )
    .await;
    let c = cleaned(&mut ws).await;
    assert_eq!((c.id, c.error.as_deref()), (3, Some("no_llm")));
}

#[tokio::test]
async fn oversized_request_is_refused() {
    let s = llm_server(Duration::ZERO, 4).await;
    let mut ws = session(s.addr()).await;
    // Under the WebSocket frame cap, over the text cap.
    let big = "a".repeat(MAX_CLEAN_TEXT_BYTES + 1);
    send(
        &mut ws,
        &ClientMsg::Clean {
            id: 4,
            req: req(&big),
        },
    )
    .await;
    let c = cleaned(&mut ws).await;
    assert_eq!(c.error.as_deref(), Some("too_large"));
    let mut r = req("x");
    r.timeout_ms = 1_000_000;
    send(&mut ws, &ClientMsg::Clean { id: 5, req: r }).await;
    assert_eq!(cleaned(&mut ws).await.error.as_deref(), Some("too_large"));
}

#[tokio::test]
async fn clean_queues_behind_a_busy_worker_and_busy_when_full() {
    let s = llm_server(Duration::from_millis(400), 1).await;
    let mut a = session(s.addr()).await;
    let mut b = session(s.addr()).await;
    let mut c = session(s.addr()).await;
    send(
        &mut a,
        &ClientMsg::Clean {
            id: 1,
            req: req("a"),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    send(
        &mut b,
        &ClientMsg::Clean {
            id: 2,
            req: req("b"),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    send(
        &mut c,
        &ClientMsg::Clean {
            id: 3,
            req: req("c"),
        },
    )
    .await;
    // The queue (1) is full: the third is refused at once.
    assert_eq!(cleaned(&mut c).await.error.as_deref(), Some("busy"));
    assert_eq!(cleaned(&mut a).await.text.as_deref(), Some("A"));
    let qb = cleaned(&mut b).await;
    assert_eq!(qb.text.as_deref(), Some("B"));
    assert!(qb.queue_ms >= 200, "{qb:?}");
}
