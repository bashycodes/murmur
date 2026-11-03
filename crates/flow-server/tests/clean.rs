//! D36 (server side): text-only `clean` requests.

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
    // A text-only cleanup is a use of the key, though no dictation.
    let u = s.keys().unwrap().list()[0].usage.clone();
    assert!(u.last_used.is_some(), "{u:?}");
    assert_eq!(u.dictations, 0);
}

#[tokio::test]
async fn failed_clean_is_not_a_use() {
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
    assert!(cleaned(&mut ws).await.error.is_some());
    assert_eq!(s.keys().unwrap().list()[0].usage.last_used, None);
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

/// Counts its calls.
struct Counting(std::sync::atomic::AtomicUsize);

impl Cleaner for Counting {
    fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(p.transcript.clone())
    }
}

#[test]
fn warm_llm_cleans_once_per_worker_with_an_llm() {
    let cfg = mock_cfg("words");
    let cl = Arc::new(Counting(Default::default()));
    let with = |c: Option<Arc<dyn Cleaner>>| {
        Arc::new(FlowCore::from_engines(Arc::new(MockAsr::new("w")), c))
    };
    let pool = Pool::from_cores(
        vec![with(Some(cl.clone())), with(None), with(Some(cl.clone()))],
        &cfg.model,
        cfg.max_queue,
    );
    assert_eq!(pool.warm_llm(), 2);
    assert_eq!(cl.0.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// One session through a server whose LLM takes 700 ms, with a 300 ms
/// session budget and a 100-character transcript: the fallback reason.
async fn session_fallback(per_char_ms: u32) -> Option<String> {
    let text = "so the build is green now and the release can go out today ".repeat(2);
    let text = &text[..100];
    let mut cfg = mock_cfg(text);
    cfg.llm_timeout_per_char_ms = per_char_ms;
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(text)),
        Some(Arc::new(Upper {
            delay: Duration::from_millis(700),
        }) as Arc<dyn Cleaner>),
    );
    let pool = Pool::from_cores(vec![Arc::new(core)], &cfg.model, cfg.max_queue);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = flow_server::serve_with_pool(cfg, Arc::new(pool), vec![l])
        .await
        .unwrap();
    let mut ws = session(s.addr()).await;
    let config = flow_core::config::SessionConfig {
        llm_timeout_ms: 300,
        llm_skip_clean: false,
        ..Default::default()
    };
    send(&mut ws, &ClientMsg::Start { sid: 1, config }).await;
    assert!(matches!(
        recv(&mut ws).await,
        Some(ServerMsg::Started { .. })
    ));
    send_audio(&mut ws, &speech(500)).await;
    send(&mut ws, &ClientMsg::Commit { sid: 1 }).await;
    match recv_final(&mut ws).await {
        Some(ServerMsg::Result { result, .. }) => result.fallback_reason,
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_llm_budget_scales_with_the_transcript_length() {
    // 300 ms + 15 ms x 100 chars = 1.8 s: the 700 ms LLM fits.
    let scaled = session_fallback(15).await;
    assert_ne!(scaled.as_deref(), Some("llm_timeout"), "{scaled:?}");
    // Scaling off on the server: the fixed 300 ms runs out.
    assert_eq!(session_fallback(0).await.as_deref(), Some("llm_timeout"));
}
