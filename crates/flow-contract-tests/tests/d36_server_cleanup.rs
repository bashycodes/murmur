//! D36 end to end: the real flow-remote client asks the real in-process
//! flow-server to clean text (`clean` / `cleaned`), and the router uses it
//! as a step of the cleanup order whatever route transcribed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_contract_tests::*;
use flow_core::cleanup::{CleanError, CleanPrompt, CleanRequest, Cleaner};
use flow_core::config::{CleanupLevel, FieldContext, SessionConfig};
use flow_core::FlowCore;
use flow_remote::RemoteBackend;
use flow_router::{Backend, CleanupRoute, RouterConfig};

const TEXT: &str = "the demo is on thursday right";
const SERVER_OUT: &str = "The demo is on Thursday, right.";
const LOCAL_OUT: &str = "The demo is on Thursday right?";

/// A fixed-answer LLM that counts its calls.
struct Fixed(&'static str, Arc<AtomicUsize>);
impl Cleaner for Fixed {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        Ok(self.0.into())
    }
}

fn llm_server() -> (TestServer, Arc<AtomicUsize>) {
    let n = Arc::new(AtomicUsize::new(0));
    let core = FlowCore::from_engines(
        Arc::new(flow_core::asr::MockAsr::new(TEXT)),
        Some(Arc::new(Fixed(SERVER_OUT, n.clone())) as Arc<dyn Cleaner>),
    );
    (spawn_core_server(core), n)
}

fn local_core(llm: Option<Arc<AtomicUsize>>) -> Arc<FlowCore> {
    Arc::new(FlowCore::from_engines(
        Arc::new(flow_core::asr::MockAsr::new(TEXT)),
        llm.map(|n| Arc::new(Fixed(LOCAL_OUT, n)) as Arc<dyn Cleaner>),
    ))
}

fn session() -> SessionConfig {
    SessionConfig {
        llm_skip_clean: false,
        cleanup_level: CleanupLevel::Light,
        ..Default::default()
    }
}

fn dictate(r: &flow_router::Router) -> flow_core::CommitResult {
    r.start_session(session()).unwrap();
    r.set_context(FieldContext::default());
    for f in speech(500).chunks(FRAME) {
        assert!(r.push_audio(f));
    }
    r.commit()
}

#[test]
fn remote_backend_cleans_text_on_the_server() {
    let (srv, n) = llm_server();
    let b = RemoteBackend::new(server_config(srv.addr(), TOKEN), "test");
    assert!(b.server_hello().is_none(), "unknown before a connection");
    let req = CleanRequest {
        text: TEXT.into(),
        level: CleanupLevel::Light,
        timeout_ms: 2000,
        ..Default::default()
    };
    let c = b
        .clean_text(&req, Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(c.text.as_deref(), Some(SERVER_OUT));
    assert_eq!(n.load(Ordering::SeqCst), 1);
    let h = b.server_hello().unwrap();
    assert!(h.features.iter().any(|f| f == flow_protocol::FEATURE_CLEAN));
    // Sessions and cleans share the warm connection. (A clean waits for a
    // worker like a session: on this one-worker server, not while a
    // session holds it.)
    b.start_session(&session()).unwrap();
    b.cancel();
    let c = b
        .clean_text(&req, Instant::now() + Duration::from_secs(5))
        .unwrap();
    assert_eq!(c.text.as_deref(), Some(SERVER_OUT));
    assert_eq!(srv.connections(), 1);
}

#[test]
fn server_without_llm_answers_no_llm() {
    let srv = spawn_mock_server(TEXT);
    let b = RemoteBackend::new(server_config(srv.addr(), TOKEN), "test");
    let c = b
        .clean_text(
            &CleanRequest {
                text: TEXT.into(),
                ..Default::default()
            },
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(c.error.as_deref(), Some("no_llm"));
    assert!(b
        .server_hello()
        .is_some_and(|h| h.features.iter().all(|f| f != "clean")));
}

fn cfg(srv: &TestServer, server_stt: bool, order: &[CleanupRoute]) -> RouterConfig {
    let mut c = router_config(srv.addr(), server_stt, true);
    if server_stt {
        c.order = vec![flow_router::Route::Server, flow_router::Route::Client];
    }
    c.cleanup_order = order.to_vec();
    c
}

#[test]
fn local_stt_cleaned_by_the_server() {
    let (srv, n) = llm_server();
    let r = hybrid_router(
        cfg(&srv, false, &[CleanupRoute::Local, CleanupRoute::Server]),
        Some(local_core(None)),
    );
    let out = dictate(&r);
    assert_eq!(out.final_text.trim(), SERVER_OUT, "{out:?}");
    let rt = out.route.unwrap();
    assert_eq!(rt.engine, "local");
    assert_eq!(rt.llm_engine.as_deref(), Some("server"));
    assert_eq!(n.load(Ordering::SeqCst), 1);
}

#[test]
fn server_stt_cleaned_locally_when_local_comes_first() {
    let (srv, n) = llm_server();
    let local = Arc::new(AtomicUsize::new(0));
    let r = hybrid_router(
        cfg(&srv, true, &[CleanupRoute::Local, CleanupRoute::Server]),
        Some(local_core(Some(local.clone()))),
    );
    let out = dictate(&r);
    assert_eq!(out.final_text.trim(), LOCAL_OUT, "{out:?}");
    let rt = out.route.unwrap();
    assert_eq!(rt.engine, "remote");
    assert_eq!(rt.llm_engine.as_deref(), Some("local"));
    assert_eq!(n.load(Ordering::SeqCst), 0, "the server skipped its LLM");
    assert_eq!(local.load(Ordering::SeqCst), 1);
}

#[test]
fn server_stt_with_server_first_is_one_pass() {
    let (srv, n) = llm_server();
    let local = Arc::new(AtomicUsize::new(0));
    let r = hybrid_router(
        cfg(&srv, true, &[CleanupRoute::Server, CleanupRoute::Local]),
        Some(local_core(Some(local.clone()))),
    );
    let out = dictate(&r);
    assert_eq!(out.final_text.trim(), SERVER_OUT, "{out:?}");
    assert_eq!(out.route.unwrap().llm_engine.as_deref(), Some("server"));
    assert_eq!(
        (n.load(Ordering::SeqCst), local.load(Ordering::SeqCst)),
        (1, 0)
    );
}
