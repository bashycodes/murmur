//! D36 on Windows: a dictation transcribed on this PC is cleaned by the
//! cleanup order (this PC → server → cloud), not only by this PC's LLM.
//! The router's local backend is the shell's model slot
//! (`router_adapter::LocalBackend` over `MurmurCoreAdapter`, MockAsr + a fake
//! on-device LLM); a fake remote backend advertising `clean` is the server
//! and a fake OpenRouter cleaner is the cloud. Before the fix the slot
//! ignored the router's cleanup plan, so the server and cloud were never
//! asked.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use murmur_core::asr::AbortFlag;
use murmur_core::cleanup::{CleanError, CleanPrompt, CleanRequest, CleanUsage, Cleaner};
use murmur_protocol::{Cleaned, HelloOk, FEATURE_CLEAN};
use murmur_router::{
    Backend, BackendError, CleanupRoute, CloudConfig, CommitResult, EngineKind, Failure,
    FieldContext, HealthReport, OpenRouterConfig, Router, RouterConfig, SecretString, ServerConfig,
    SessionConfig, StartInfo,
};
use murmur_windows::core_adapter::{LlmLoaded, LlmSpec, ModelSpec, MurmurCoreAdapter};
use murmur_windows::router_adapter::LocalBackend;
use CleanupRoute::{Cloud as C, Local as L, Server as S};

const KEY: &str = "sk-or-LEAKCHECK-wincc-0123456789";
const HEARD: &str = "the demo is on thursday right";
/// Already tidy: the deterministic rules leave an LLM nothing to do (D27).
const TIDY: &str = "The demo is on Thursday, right?";
// Each cleaner answers differently.
const PC_OUT: &str = "The demo is on Thursday right?";
const SERVER_OUT: &str = "The demo is on Thursday, right.";
const CLOUD_OUT: &str = "The demo is on Thursday, right!";

/// Counts calls and answers `out` (the fake on-device LLM and OpenRouter).
struct Fake {
    engine: &'static str,
    out: &'static str,
    calls: AtomicUsize,
}

impl Fake {
    fn new(engine: &'static str, out: &'static str) -> Arc<Self> {
        Arc::new(Fake {
            engine,
            out,
            calls: AtomicUsize::new(0),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Cleaner for Fake {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.clean_metered(p, d, &AbortFlag::default()).0
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
    fn clean_metered(
        &self,
        _p: &CleanPrompt,
        _d: Instant,
        _c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (Ok(self.out.into()), CleanUsage::default())
    }
}

/// Fake murmur-server with an LLM: advertises `clean` and answers every
/// text-only request. Never transcribes here (the speech route is this PC).
#[derive(Default)]
struct Server {
    cleans: Mutex<Vec<CleanRequest>>,
    starts: AtomicUsize,
    /// Answer `clean` with a lost connection.
    fail: AtomicBool,
}

impl Server {
    fn cleans(&self) -> usize {
        self.cleans.lock().unwrap().len()
    }
}

impl Backend for Server {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }
    fn ready(&self) -> bool {
        true
    }
    fn start_session(&self, _c: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(BackendError::new(
            Failure::ConnectFailed,
            "not a speech route",
        ))
    }
    fn push_audio(&self, _p: &[i16]) -> bool {
        false
    }
    fn set_context(&self, _c: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, _d: Instant) -> Result<CommitResult, BackendError> {
        Err(BackendError::new(Failure::Disconnected, "no session"))
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        HealthReport::default()
    }
    fn server_hello(&self) -> Option<HelloOk> {
        Some(HelloOk {
            llm: true,
            server: "murmur-server 0.12.0".into(),
            features: vec![FEATURE_CLEAN.into()],
            ..Default::default()
        })
    }
    fn clean_text(&self, req: &CleanRequest, _d: Instant) -> Result<Cleaned, BackendError> {
        self.cleans.lock().unwrap().push(req.clone());
        if self.fail.load(Ordering::SeqCst) {
            return Err(BackendError::new(Failure::Disconnected, "gone"));
        }
        Ok(Cleaned {
            text: Some(SERVER_OUT.into()),
            model: Some("server-llm".into()),
            ..Default::default()
        })
    }
}

/// Speech on this PC only; server configured (cleanup only); cloud
/// cleanup allowed and consented. `order` = the cleanup order, `off` = the
/// entries switched off.
fn cfg(order: &[CleanupRoute], off: &[CleanupRoute]) -> RouterConfig {
    let mut openrouter = OpenRouterConfig::default();
    openrouter.cleanup.consent_host = "openrouter.ai".into();
    openrouter.cleanup.consent_at_ms = 1_792_540_900_000;
    RouterConfig {
        allow_server: false,
        allow_client: true,
        server: ServerConfig {
            host: "100.64.0.10".into(),
            token: "0123456789abcdef".into(),
            ..Default::default()
        },
        allow_cloud_cleanup: !off.contains(&C),
        cleanup_order: order.to_vec(),
        cleanup_off: off.to_vec(),
        cloud: CloudConfig {
            openrouter,
            ..Default::default()
        },
        ..Default::default()
    }
}

struct Rig {
    router: Router,
    slot: Arc<MurmurCoreAdapter>,
    pc: Arc<Fake>,
    server: Arc<Server>,
    cloud: Arc<Fake>,
    /// Remote backends built (a server connection needs one).
    remotes: Arc<AtomicUsize>,
}

impl Rig {
    fn new(c: RouterConfig, pc_llm: bool, heard: &str) -> Rig {
        let pc = Fake::new("local", PC_OUT);
        let pc2 = pc.clone();
        let slot = MurmurCoreAdapter::with_llm_loader(Arc::new(move |_s: &LlmSpec| {
            Ok(LlmLoaded {
                cleaner: pc2.clone() as Arc<dyn Cleaner>,
                plugin: "fake-plugin".into(),
                backend: None,
            })
        }));
        assert!(slot
            .load_now(&ModelSpec::Mock { text: heard.into() })
            .is_ready());
        let server = Arc::new(Server::default());
        let cloud = Fake::new("cloud", CLOUD_OUT);
        let remotes = Arc::new(AtomicUsize::new(0));
        let (s2, n2, c2) = (server.clone(), remotes.clone(), cloud.clone());
        let router = Router::new(
            c,
            Some(LocalBackend::new(slot.clone()) as Arc<dyn Backend>),
            Some(Box::new(move |_s: &ServerConfig| {
                n2.fetch_add(1, Ordering::SeqCst);
                s2.clone() as Arc<dyn Backend>
            })),
        )
        .with_cleanup_factory(Box::new(move |_o, _k| c2.clone() as Arc<dyn Cleaner>));
        router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
        let k = Rig {
            router,
            slot,
            pc,
            server,
            cloud,
            remotes,
        };
        k.llm(pc_llm);
        k
    }

    /// Turn this PC's LLM on or off (the slot rebuilds its core).
    fn llm(&self, on: bool) {
        self.slot.load_llm_now(on.then(|| LlmSpec {
            model_path: "models/fake.gguf".into(),
            lib_path: None,
            threads: Some(1),
        }));
    }

    fn dictate(&self, llm_skip_clean: bool) -> CommitResult {
        self.router
            .start_session(SessionConfig {
                llm_skip_clean,
                ..SessionConfig::default()
            })
            .unwrap();
        let pcm: Vec<i16> = (0..300 * 16)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        assert!(self.router.push_audio(&pcm));
        self.router.commit()
    }

    /// (this PC's LLM, server `clean`, cloud) calls so far.
    fn calls(&self) -> (usize, usize, usize) {
        (self.pc.calls(), self.server.cleans(), self.cloud.calls())
    }
}

fn engines(r: &CommitResult) -> (String, Option<String>) {
    let rt = r.route.clone().expect("route info");
    (rt.engine, rt.llm_engine)
}

#[test]
fn pc_llm_off_the_server_cleans_a_dictation_made_on_this_pc() {
    let k = Rig::new(cfg(&[L, S, C], &[]), false, HEARD);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), SERVER_OUT, "{r:?}");
    assert_eq!(k.calls(), (0, 1, 0));
    assert_eq!(
        engines(&r),
        ("local".into(), Some("server".into())),
        "speech stays on this PC"
    );
    assert_eq!(k.server.starts.load(Ordering::SeqCst), 0, "no server STT");
}

#[test]
fn server_off_openrouter_cleans() {
    let k = Rig::new(cfg(&[L, S, C], &[S]), false, HEARD);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), CLOUD_OUT, "{r:?}");
    assert_eq!(k.calls(), (0, 0, 1));
    assert_eq!(engines(&r).1.as_deref(), Some("cloud"));
}

#[test]
fn pc_llm_on_and_first_cleans_and_the_server_is_not_asked() {
    let k = Rig::new(cfg(&[L, S, C], &[]), true, HEARD);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), PC_OUT, "{r:?}");
    assert_eq!(k.calls(), (1, 0, 0));
    assert_eq!(engines(&r).0, "local");
}

#[test]
fn the_already_clean_skip_is_for_this_pcs_llm_only() {
    // D27/D43: this PC's LLM first and loaded → a tidy dictation is skipped.
    let k = Rig::new(cfg(&[L, S, C], &[]), true, TIDY);
    let r = k.dictate(true);
    assert_eq!(
        r.fallback_reason.as_deref(),
        Some("llm_skipped_clean"),
        "{r:?}"
    );
    assert_eq!(k.calls(), (0, 0, 0));
    // This PC's LLM off → the server is first and still cleans it.
    let k = Rig::new(cfg(&[L, S, C], &[]), false, TIDY);
    let r = k.dictate(true);
    assert_eq!(r.final_text.trim(), SERVER_OUT, "{r:?}");
    assert_eq!(k.calls(), (0, 1, 0));
}

#[test]
fn a_model_reload_keeps_the_server_hop() {
    let k = Rig::new(cfg(&[L, S, C], &[]), false, HEARD);
    // Switching the speech model rebuilds the slot's core; the router is
    // not told (the shell only refreshes it after some loads).
    assert!(k
        .slot
        .load_now(&ModelSpec::Mock { text: HEARD.into() })
        .is_ready());
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), SERVER_OUT, "{r:?}");
    assert_eq!(k.calls(), (0, 1, 0));
    // This PC's LLM on (another rebuild): it is first, so it cleans.
    k.llm(true);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), PC_OUT, "{r:?}");
    assert_eq!(k.calls(), (1, 1, 0));
    // And off again: back to the server.
    k.llm(false);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), SERVER_OUT, "{r:?}");
    assert_eq!(k.calls(), (1, 2, 0));
}

#[test]
fn a_failing_server_hands_over_to_openrouter_then_the_rules() {
    let k = Rig::new(cfg(&[L, S, C], &[]), false, HEARD);
    k.server.fail.store(true, Ordering::SeqCst);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), CLOUD_OUT, "{r:?}");
    assert_eq!(k.calls(), (0, 1, 1));
    assert_eq!(engines(&r).1.as_deref(), Some("cloud"));
    // OpenRouter off too: the rules alone, and the dictation still lands.
    let k = Rig::new(cfg(&[L, S, C], &[C]), false, HEARD);
    k.server.fail.store(true, Ordering::SeqCst);
    let r = k.dictate(false);
    assert_eq!(k.calls(), (0, 1, 0));
    assert_eq!(r.error, None, "{r:?}");
    assert!(
        r.final_text.to_lowercase().contains(HEARD),
        "rules output: {r:?}"
    );
}

#[test]
fn cleanup_off_beyond_this_pc_builds_no_remote() {
    let k = Rig::new(cfg(&[L, S, C], &[S, C]), true, HEARD);
    let r = k.dictate(false);
    assert_eq!(r.final_text.trim(), PC_OUT, "{r:?}");
    assert_eq!(k.calls(), (1, 0, 0));
    assert_eq!(k.remotes.load(Ordering::SeqCst), 0, "no remote built");
}

#[test]
fn llm_cleaner_is_this_pcs_llm_only() {
    let k = Rig::new(cfg(&[L, S, C], &[]), false, HEARD);
    let b = LocalBackend::new(k.slot.clone());
    assert!(b.llm_cleaner().is_none(), "LLM off: no cleaner");
    k.llm(true);
    let c = b.llm_cleaner().expect("this PC's LLM");
    assert_eq!(c.engine(), "local");
}
