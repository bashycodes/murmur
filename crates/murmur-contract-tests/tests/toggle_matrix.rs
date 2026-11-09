//! Toggle matrix (§4): allow_server × allow_client × server state, through
//! the real router, flow-remote client and an in-process flow-server.
//! MockAsr records the samples it was given, so loss is measurable.
//! Naming: matrix_<server>_<client>_<state>_<expectation>.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_contract_tests::*;
use flow_core::asr::MockAsr;
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::{CommitResult, CommitStatus, FlowCore};
use flow_router::{Backend, Prefer, Router, RouterConfig, RouterError};

const TEXT: &str = "hello world";
const AUDIO_MS: usize = 2000;

struct Rig {
    server: TestServer,
    asr: Arc<MockAsr>,
    router: Router,
}

fn local() -> (Arc<MockAsr>, Arc<FlowCore>) {
    let asr = Arc::new(MockAsr::new(TEXT));
    (asr.clone(), Arc::new(FlowCore::from_engines(asr, None)))
}

fn rig_cfg(server_on: bool, client_on: bool, tweak: impl FnOnce(&mut RouterConfig)) -> Rig {
    let server = spawn_mock_server(TEXT);
    let (asr, core) = local();
    let mut cfg = router_config(server.addr(), server_on, client_on);
    cfg.server.commit_timeout_base_ms = 500;
    tweak(&mut cfg);
    let router = hybrid_router(cfg, Some(core));
    Rig {
        server,
        asr,
        router,
    }
}

fn rig(server_on: bool, client_on: bool) -> Rig {
    rig_cfg(server_on, client_on, |_| {})
}

/// VAD off so MockAsr sees every pushed sample; no LLM budget in the
/// commit deadline.
fn session() -> SessionConfig {
    SessionConfig {
        vad: false,
        llm_timeout_ms: 0,
        ..Default::default()
    }
}

/// Start, push AUDIO_MS at a fast pace (calling `mid` halfway), commit.
fn dictate_with(r: &Router, mid: impl FnOnce()) -> (Result<(), RouterError>, CommitResult) {
    let started = r.start_session(session());
    let pcm = speech(AUDIO_MS);
    let half = pcm.len() / 2;
    let mut mid = Some(mid);
    for (i, c) in pcm.chunks(FRAME).enumerate() {
        if i * FRAME >= half {
            if let Some(f) = mid.take() {
                f();
            }
        }
        r.push_audio(c);
        std::thread::sleep(Duration::from_millis(2));
    }
    r.set_context(FieldContext {
        app_id: "notepad".into(),
        ..Default::default()
    });
    (started, r.commit())
}

fn dictate(r: &Router) -> (Result<(), RouterError>, CommitResult) {
    dictate_with(r, || {})
}

fn engine(r: &CommitResult) -> (String, Option<String>) {
    let rt = r.route.clone().expect("route");
    (rt.engine, rt.fallback)
}

fn remote_ok(r: &CommitResult) {
    assert_eq!(engine(r), ("remote".into(), None), "{r:?}");
    assert_eq!(r.status, CommitStatus::RawTranscript);
}

/// The result a failure-free local run gives.
fn reference() -> CommitResult {
    let (_, core) = local();
    let r = Router::new(
        RouterConfig {
            allow_server: false,
            ..Default::default()
        },
        Some(core as Arc<dyn Backend>),
        None,
    );
    dictate(&r).1
}

fn full_audio_reached_local(k: &Rig, r: &CommitResult) {
    assert_eq!(
        k.asr.last_len(),
        AUDIO_MS * 16,
        "local ASR saw every sample"
    );
    assert_eq!(r.timings.dropped_samples, 0);
    assert_eq!(normalize(r), normalize(&reference()));
}

// ---- both allowed ---------------------------------------------------------

#[test]
fn matrix_on_on_up_uses_remote() {
    let k = rig(true, true);
    let (s, r) = dictate(&k.router);
    s.unwrap();
    remote_ok(&r);
    assert_eq!(k.asr.calls(), 0, "local ASR never ran");
    assert_eq!(normalize(&r), normalize(&reference()));
    let rt = r.route.unwrap();
    assert_eq!(rt.server.unwrap(), k.server.addr().to_string());
    assert!(rt.rtt_ms.is_some() && rt.server_core.is_some());
}

#[test]
fn matrix_on_on_down_falls_back_local() {
    let k = rig(true, true);
    k.server.kill();
    let t = Instant::now();
    k.router.start_session(session()).unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(800),
        "start ≤ connect_timeout"
    );
    k.router.cancel();
    k.router.reset_circuit();
    let (s, r) = dictate(&k.router);
    s.unwrap();
    assert_eq!(engine(&r), ("local".into(), Some("connect_failed".into())));
    full_audio_reached_local(&k, &r);
}

#[test]
fn matrix_on_on_busy_goes_local() {
    let k = rig(true, true);
    k.server.set_busy(true);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("busy".into())));
    full_audio_reached_local(&k, &r);
    std::thread::sleep(Duration::from_millis(100));
    assert!(k.server.cancels() >= 1, "server received cancel");
    // Busy does not open the circuit: next session tries the server again.
    k.server.set_busy(false);
    let (_, r) = dictate(&k.router);
    remote_ok(&r);
}

#[test]
fn matrix_on_on_stalls_commit_times_out_to_local() {
    let k = rig(true, true);
    k.server.stall_commits(true);
    let t = Instant::now();
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("timeout".into())));
    full_audio_reached_local(&k, &r);
    // Deadline = 500 base + 0.3 × 2000 ms audio + 0 LLM = 1.1 s, plus pushes.
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}

#[test]
fn matrix_on_on_dies_mid_session_replays_buffer() {
    let k = rig(true, true);
    let (_, r) = dictate_with(&k.router, || k.server.kill());
    assert_eq!(engine(&r), ("local".into(), Some("disconnected".into())));
    full_audio_reached_local(&k, &r);
}

#[test]
fn matrix_on_on_bad_token_auth_failed_not_retried() {
    let k = rig_cfg(true, true, |c| {
        c.server.token = "wrong-token-0123456789".into()
    });
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("auth_failed".into())));
    let conns = k.server.connections();
    assert_eq!(conns, 1);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("circuit_open".into())));
    assert_eq!(
        k.server.connections(),
        conns,
        "second session opens no connection"
    );
    let st = k.router.route_status();
    assert!(st.circuit.open && st.circuit.until_config_change);
    // Fixing the token closes the circuit.
    let mut cfg = k.router.config();
    cfg.server.token = TOKEN.into();
    k.router.update_config(cfg);
    remote_ok(&dictate(&k.router).1);
}

#[test]
fn matrix_on_on_circuit_open_skips_remote() {
    let k = rig(true, true);
    k.server.stall_commits(true);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r).1.as_deref(), Some("timeout"));
    let conns = k.server.connections();
    let sessions = k.server.sessions();
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("circuit_open".into())));
    assert_eq!(k.server.connections(), conns, "0 new connections");
    assert_eq!(k.server.sessions(), sessions);
    let st = k.router.route_status();
    assert!(st.circuit.open);
    assert!(st.circuit.retry_in_ms.unwrap() <= 5000);
}

#[test]
fn matrix_on_on_prefer_client_uses_local() {
    let k = rig_cfg(true, true, |c| c.prefer = Prefer::Client);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), None));
    assert_eq!(k.server.connections(), 0);
}

#[test]
fn matrix_on_on_prefer_client_local_not_ready_uses_remote() {
    let server = spawn_mock_server(TEXT);
    let mut cfg = router_config(server.addr(), true, true);
    cfg.prefer = Prefer::Client;
    let router = hybrid_router(cfg, None).with_local_capability(true);
    let (_, r) = dictate(&router);
    assert_eq!(
        engine(&r),
        ("remote".into(), Some("local_not_ready".into()))
    );
    assert_eq!(r.status, CommitStatus::RawTranscript);
}

// ---- server only ------------------------------------------------------------

#[test]
fn matrix_on_off_up_uses_remote() {
    let k = rig(true, false);
    let (s, r) = dictate(&k.router);
    s.unwrap();
    remote_ok(&r);
    assert_eq!(k.asr.calls(), 0);
}

#[test]
fn matrix_on_off_down_is_error_server_unavailable() {
    let k = rig(true, false);
    k.server.kill();
    let (s, r) = dictate(&k.router);
    assert_eq!(s.unwrap_err().kind(), "server_unavailable");
    assert_eq!(r.status, CommitStatus::Error);
    assert!(r
        .error
        .as_deref()
        .unwrap()
        .starts_with("server_unavailable"));
    assert_eq!(engine(&r).1.as_deref(), Some("connect_failed"));
    assert_eq!(k.asr.calls(), 0);
}

#[test]
fn matrix_on_off_dies_mid_session_is_error() {
    let k = rig(true, false);
    let t = Instant::now();
    let (_, r) = dictate_with(&k.router, || k.server.kill());
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(engine(&r).1.as_deref(), Some("disconnected"));
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "no hang past the deadline"
    );
    assert_eq!(k.asr.calls(), 0);
}

// ---- client only ------------------------------------------------------------

#[test]
fn matrix_off_on_up_never_connects() {
    let k = rig(false, true);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), None));
    assert_eq!(k.server.connections(), 0);
    let _ = k.router.route_status();
    // Client-only build: no remote factory at all.
    let (_, core) = local();
    let r = Router::new(
        router_config(k.server.addr(), true, true),
        Some(core as Arc<dyn Backend>),
        None,
    );
    assert!(!r.capabilities().remote);
    assert_eq!(engine(&dictate(&r).1), ("local".into(), None));
    assert_eq!(k.server.connections(), 0);
}

#[test]
fn matrix_off_on_down_never_connects() {
    let k = rig(false, true);
    k.server.kill();
    let (s, r) = dictate(&k.router);
    s.unwrap();
    assert_eq!(engine(&r), ("local".into(), None));
    full_audio_reached_local(&k, &r);
    assert_eq!(k.server.connections(), 0);
}

// ---- nothing allowed --------------------------------------------------------

fn off_off(kill: bool) {
    let k = rig(false, false);
    if kill {
        k.server.kill();
    }
    let (s, r) = dictate(&k.router);
    assert_eq!(s.unwrap_err(), RouterError::NoEngineEnabled);
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(k.server.connections(), 0);
    assert_eq!(k.asr.calls(), 0);
}

#[test]
fn matrix_off_off_start_fails_no_engine_enabled_up() {
    off_off(false);
}

#[test]
fn matrix_off_off_start_fails_no_engine_enabled_down() {
    off_off(true);
}

// ---- config changes and the address gate ------------------------------------

#[test]
fn matrix_update_config_applies_next_session() {
    let k = rig(true, true);
    k.router.start_session(session()).unwrap();
    k.router.push_audio(&speech(500));
    let mut cfg = k.router.config();
    cfg.allow_server = false;
    k.router.update_config(cfg);
    k.router.push_audio(&speech(500));
    let r = k.router.commit();
    assert_eq!(
        engine(&r).0,
        "remote",
        "the running session keeps its route"
    );
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), None));
}

#[test]
fn matrix_plaintext_public_host_refused() {
    let (_, core) = local();
    let mut cfg = RouterConfig::default();
    cfg.server.host = "8.8.8.8".into();
    cfg.server.token = TOKEN.into();
    let r = hybrid_router(cfg, Some(core));
    let t = Instant::now();
    let (_, res) = dictate(&r);
    assert_eq!(
        engine(&res),
        ("local".into(), Some("connect_failed".into()))
    );
    let h = r.probe_server();
    assert!(!h.ok);
    assert!(h.message.unwrap().contains("plaintext to public address"));
    // Refused without a socket: no connect timeout was spent.
    assert!(
        t.elapsed() < Duration::from_millis(700),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn probe_server_reports_latency_and_server_info() {
    let k = rig(true, true);
    let h = k.router.probe_server();
    assert!(h.ok, "{h:?}");
    let rtt = h.rtt_ms.unwrap();
    assert!(rtt.min <= rtt.median && rtt.median <= rtt.max);
    assert_eq!(h.asr_backend.as_deref(), Some("mock"));
    assert_eq!(h.postprocess_match, Some(true));
    assert_eq!(h.proto, Some(1));
    assert!(k.router.route_status().health.unwrap().ok);
    let mut bad = k.router.config().server;
    bad.token = "nope-nope-nope-nope".into();
    let h = k.router.probe_config(&bad);
    assert_eq!(h.error.as_deref(), Some("auth_failed"));
}

#[test]
fn warm_connection_is_reused_across_sessions() {
    let k = rig(true, true);
    for _ in 0..3 {
        remote_ok(&dictate(&k.router).1);
    }
    assert_eq!(k.server.connections(), 1);
    assert_eq!(k.server.sessions(), 3);
}

// ---- cloud route (D18, docs/cloud-stt.md §3.2) ------------------------------

use flow_cloud::testing::Reply;
use flow_router::Route;
use std::sync::atomic::{AtomicU16, Ordering};

struct CloudRig {
    server: TestServer,
    asr: Arc<MockAsr>,
    router: Router,
    provider: MockCloud,
    status: Arc<AtomicU16>,
}

/// Real flow-server + a mock provider answering `status` (200 = "hello
/// world" from the cloud) + a local MockAsr core.
fn cloud_rig(order: &[Route]) -> CloudRig {
    let server = spawn_mock_server(TEXT);
    let (asr, core) = local();
    let status = Arc::new(AtomicU16::new(200));
    let s2 = status.clone();
    let provider = mock_cloud(move |_, _| match s2.load(Ordering::SeqCst) {
        200 => Reply::json(
            200,
            serde_json::json!({"text": "cloud words", "language": "en"}),
        ),
        s => Reply::json(s, serde_json::json!({})),
    });
    let mut cfg = router_config(server.addr(), true, true);
    cfg.server.commit_timeout_base_ms = 500;
    let cfg = with_cloud(cfg, cloud_provider(), order);
    let router = cloud_router(cfg, Some(core));
    CloudRig {
        server,
        asr,
        router,
        provider,
        status,
    }
}

fn cloud_full_audio_reached_local(k: &CloudRig, r: &CommitResult) {
    assert_eq!(
        k.asr.last_len(),
        AUDIO_MS * 16,
        "local ASR saw every sample"
    );
    assert_eq!(r.timings.dropped_samples, 0);
    assert_eq!(normalize(r), normalize(&reference()));
}

#[test]
fn matrix_cloud_first_uses_the_provider() {
    let k = cloud_rig(&[Route::Cloud, Route::Client]);
    let (s, r) = dictate(&k.router);
    s.unwrap();
    assert_eq!(engine(&r), ("cloud".into(), None));
    assert_eq!(r.asr_text, "cloud words");
    assert_eq!(k.asr.calls(), 0);
    assert_eq!(k.provider.count(), 1);
    assert_eq!(k.server.sessions(), 0, "server not involved");
}

#[test]
fn matrix_cloud_401_is_auth_failed_sticky_and_local_gets_everything() {
    let k = cloud_rig(&[Route::Cloud, Route::Client]);
    k.status.store(401, Ordering::SeqCst);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("auth_failed".into())));
    cloud_full_audio_reached_local(&k, &r);
    // Sticky: the provider is not asked again until the key changes.
    k.status.store(200, Ordering::SeqCst);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("circuit_open".into())));
    assert_eq!(k.provider.count(), 1);
    k.router.set_cloud_key(
        CLOUD_ID,
        Some(flow_router::SecretString::new("sk-another-key-000")),
    );
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("cloud".into(), None));
}

#[test]
fn matrix_cloud_500_goes_local_with_all_samples() {
    let k = cloud_rig(&[Route::Cloud, Route::Client]);
    k.status.store(500, Ordering::SeqCst);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("provider_error".into())));
    cloud_full_audio_reached_local(&k, &r);
}

#[test]
fn matrix_cloud_server_down_then_cloud() {
    let k = cloud_rig(&[Route::Server, Route::Cloud, Route::Client]);
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("remote".into(), None));
    assert_eq!(k.provider.count(), 0, "cloud only when the server is down");
    k.server.kill();
    k.router.reset_circuit();
    let (_, r) = dictate(&k.router);
    // The first dictation left a warm connection. If the client has already
    // read the server's close, the next start connects afresh and fails
    // (`connect_failed`); on a loaded box its reader may not have run yet,
    // so the start goes out on the dead warm connection and the session
    // fails as `disconnected`. Both are "server down" and both must replay
    // to the cloud, never to local.
    let (eng, why) = engine(&r);
    assert_eq!(eng, "cloud", "{r:?}");
    assert!(
        matches!(why.as_deref(), Some("connect_failed" | "disconnected")),
        "{why:?}"
    );
    assert_eq!(r.asr_text, "cloud words");
    assert_eq!(k.asr.calls(), 0);
}

#[test]
fn matrix_cloud_server_dies_mid_session_replays_to_cloud() {
    let k = cloud_rig(&[Route::Server, Route::Cloud, Route::Client]);
    let (_, r) = dictate_with(&k.router, || k.server.kill());
    assert_eq!(engine(&r).0, "cloud", "{r:?}");
    assert_eq!(r.asr_text, "cloud words");
    // FLAC of the whole replayed buffer: decode its sample count.
    let n = claxon::FlacReader::new(std::io::Cursor::new(k.provider.uploaded_audio(0)))
        .unwrap()
        .streaminfo()
        .samples
        .unwrap();
    assert_eq!(
        n as usize,
        AUDIO_MS * 16,
        "every sample reached the provider"
    );
}

#[test]
fn matrix_cloud_airplane_mode_goes_local() {
    let k = cloud_rig(&[Route::Cloud, Route::Client]);
    // Provider unreachable: point the config at a closed port.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    k.provider
        .point_at(&format!("http://127.0.0.1:{port}/api/v1"));
    let (_, r) = dictate(&k.router);
    assert_eq!(engine(&r), ("local".into(), Some("connect_failed".into())));
    cloud_full_audio_reached_local(&k, &r);
}
