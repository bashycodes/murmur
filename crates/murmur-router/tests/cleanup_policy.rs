//! D28 cloud cleanup policy (spec §2.5/§2.6) over a real `MurmurCore`
//! (MockAsr + a fake on-device LLM) and a fake OpenRouter cleaner injected
//! as the cleanup factory: C1 for cleanup, consent re-check per call,
//! circuit, budget, deadline, server route never cleaned in the cloud.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use murmur_core::asr::{AbortFlag, MockAsr};
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use murmur_core::MurmurCore;
use murmur_router::*;

const KEY: &str = "sk-or-LEAKCHECK-cleanup-0123456789";
const IN: &str = "the demo is on thursday right";
const CLOUD_OUT: &str = "The demo is on Thursday, right?";
const LOCAL_OUT: &str = "The demo is on Thursday right?";

/// Fake on-device LLM.
struct Local(AtomicUsize);
impl Cleaner for Local {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(LOCAL_OUT.into())
    }
}

/// Fake OpenRouter cleaner (what murmur-cloud's factory returns).
struct Cloud {
    calls: AtomicUsize,
    out: Mutex<Result<String, CleanError>>,
    delay: Mutex<Duration>,
    estimate: Mutex<Option<f64>>,
    cost: f64,
    seen: Mutex<Vec<CleanPrompt>>,
}

impl Cloud {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            out: Mutex::new(Ok(CLOUD_OUT.into())),
            delay: Mutex::new(Duration::ZERO),
            estimate: Mutex::new(Some(0.001)),
            cost: 0.0007,
            seen: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn fail(&self, e: Option<CleanError>) {
        *self.out.lock().unwrap() = match e {
            Some(e) => Err(e),
            None => Ok(CLOUD_OUT.into()),
        };
    }
}

fn clone_err(e: &CleanError) -> CleanError {
    match e {
        CleanError::Unavailable(c) => CleanError::Unavailable(c.clone()),
        CleanError::Backend(c) => CleanError::Backend(c.clone()),
    }
}

impl Cleaner for Cloud {
    fn clean(&self, p: &CleanPrompt, d: Instant) -> Result<String, CleanError> {
        self.clean_metered(p, d, &AbortFlag::default()).0
    }
    fn engine(&self) -> &'static str {
        "cloud"
    }
    fn estimate_cost_usd(&self, _p: &CleanPrompt) -> Option<f64> {
        *self.estimate.lock().unwrap()
    }
    fn clean_metered(
        &self,
        p: &CleanPrompt,
        _d: Instant,
        _c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(p.clone());
        std::thread::sleep(*self.delay.lock().unwrap());
        let r = match &*self.out.lock().unwrap() {
            Ok(s) => Ok(s.clone()),
            Err(e) => Err(clone_err(e)),
        };
        let billed = r.is_ok();
        (
            r,
            CleanUsage {
                cost_usd: billed.then_some(Usd(self.cost)),
                generation_id: billed.then(|| "gen-fake".into()),
                ..Default::default()
            },
        )
    }
}

/// A server backend that records whether anyone tried to install a
/// cleanup plan on it.
struct Server {
    installs: AtomicUsize,
}
impl Backend for Server {
    fn kind(&self) -> EngineKind {
        EngineKind::Remote
    }
    fn ready(&self) -> bool {
        true
    }
    fn start_session(&self, _c: &SessionConfig) -> Result<StartInfo, BackendError> {
        Ok(StartInfo::default())
    }
    fn push_audio(&self, _p: &[i16]) -> bool {
        true
    }
    fn set_context(&self, _c: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, _d: Instant) -> Result<CommitResult, BackendError> {
        Ok(serde_json::from_value(serde_json::json!({
            "status": "FORMATTED", "asr_text": IN, "final_text": "server text", "language": "en",
            "timings": {"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},
            "press_enter": false, "fallback_reason": null, "truncated": false, "error": null,
            "llm": {"engine": "local", "model": "qwen2.5-1.5b"}
        }))
        .unwrap())
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        HealthReport::default()
    }
    fn install_cleanup(&self, _p: Option<CleanupPlan>) {
        self.installs.fetch_add(1, Ordering::SeqCst);
    }
}

fn consented() -> OpenRouterConfig {
    let mut o = OpenRouterConfig::default();
    o.cleanup.consent_host = "openrouter.ai".into();
    o.cleanup.consent_at_ms = 1_792_540_900_000;
    o
}

/// Everything C1-for-cleanup asks for, on the client route.
fn cfg() -> RouterConfig {
    RouterConfig {
        allow_server: false,
        allow_cloud_cleanup: true,
        cleanup_order: vec![CleanupRoute::Cloud, CleanupRoute::Local],
        cloud: CloudConfig {
            openrouter: consented(),
            ..Default::default()
        },
        ..Default::default()
    }
}

struct Rig {
    router: Router,
    local_llm: Arc<Local>,
    cloud: Arc<Cloud>,
    factory_calls: Arc<AtomicUsize>,
    keys_seen: Arc<Mutex<Vec<String>>>,
}

fn rig_opts(c: RouterConfig, factory: bool, local_llm: bool, key: bool) -> Rig {
    let llm = Arc::new(Local(AtomicUsize::new(0)));
    let core = MurmurCore::from_engines(
        Arc::new(MockAsr::new(IN)),
        local_llm.then(|| llm.clone() as Arc<dyn Cleaner>),
    );
    let cloud = Cloud::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let keys = Arc::new(Mutex::new(Vec::new()));
    let mut router = Router::new(c, Some(Arc::new(core) as Arc<dyn Backend>), None);
    if factory {
        let (c2, calls2, keys2) = (cloud.clone(), calls.clone(), keys.clone());
        router = router.with_cleanup_factory(Box::new(move |_o: &OpenRouterConfig, k| {
            calls2.fetch_add(1, Ordering::SeqCst);
            keys2.lock().unwrap().push(k.expose().to_string());
            c2.clone() as Arc<dyn Cleaner>
        }));
    }
    if key {
        router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    }
    Rig {
        router,
        local_llm: llm,
        cloud,
        factory_calls: calls,
        keys_seen: keys,
    }
}

fn rig(c: RouterConfig) -> Rig {
    rig_opts(c, true, true, true)
}

fn session() -> SessionConfig {
    SessionConfig {
        llm_skip_clean: false,
        ..Default::default()
    }
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn dictate_with(r: &Router, ctx: FieldContext) -> CommitResult {
    r.start_session(session()).unwrap();
    r.set_context(ctx);
    assert!(r.push_audio(&speech(300)));
    r.commit()
}

fn dictate(r: &Router) -> CommitResult {
    dictate_with(r, FieldContext::default())
}

fn local_calls(k: &Rig) -> usize {
    k.local_llm.0.load(Ordering::SeqCst)
}

// ---- C1 for cleanup ---------------------------------------------------------

#[test]
fn cloud_cleanup_with_all_conditions_runs_first() {
    let k = rig(cfg());
    let r = dictate(&k.router);
    assert!(r.final_text.contains(CLOUD_OUT), "{r:?}");
    assert_eq!((k.cloud.calls(), local_calls(&k)), (1, 0));
    let rt = r.route.clone().unwrap();
    assert_eq!(rt.engine, "local", "STT stayed on-device");
    assert_eq!(rt.llm_engine.as_deref(), Some("cloud"));
    assert_eq!(
        rt.llm_model.as_deref(),
        Some("meta-llama/llama-3.3-70b-instruct")
    );
    assert_eq!(rt.llm_cost_usd, Some(Usd(0.0007)));
    assert_eq!(r.llm.unwrap().generation_id.as_deref(), Some("gen-fake"));
    let u = k.router.cloud_usage();
    assert!((u.cost_for("cleanup") - 0.0007).abs() < 1e-12, "{u:?}");
    assert_eq!(u.cost_for("stt"), 0.0);
    let st = k.router.cleanup_status();
    assert!(
        st.supported && st.enabled && st.consent && st.blocked.is_none(),
        "{st:?}"
    );
    assert_eq!(*k.keys_seen.lock().unwrap(), [KEY]);
}

fn assert_no_cloud_cleanup(k: &Rig, why: &str) {
    let r = dictate(&k.router);
    assert_eq!(k.cloud.calls(), 0, "{why}: cloud cleaner called");
    assert!(
        r.final_text.contains(LOCAL_OUT),
        "{why}: local LLM expected: {r:?}"
    );
    assert_eq!(
        r.route.unwrap().llm_engine.as_deref(),
        Some("local"),
        "{why}"
    );
}

#[test]
fn each_missing_condition_means_no_cloud_request() {
    assert_no_cloud_cleanup(&rig_opts(cfg(), false, true, true), "no factory");
    assert_no_cloud_cleanup(&rig_opts(cfg(), true, true, false), "no key");
    let mut c = cfg();
    c.allow_cloud_cleanup = false;
    assert_no_cloud_cleanup(&rig(c), "toggle off");
    let mut c = cfg();
    c.cleanup_order = vec![CleanupRoute::Local];
    assert_no_cloud_cleanup(&rig(c), "order omits cloud");
    let mut c = cfg();
    c.cleanup_order.clear();
    assert_no_cloud_cleanup(&rig(c), "empty order = pre-D28");
    let mut c = cfg();
    c.cloud.openrouter.cleanup.consent_at_ms = 0;
    assert_no_cloud_cleanup(&rig(c), "no consent");
    let mut c = cfg();
    c.cloud.openrouter.cleanup.consent_host = "api.example.com".into();
    assert_no_cloud_cleanup(&rig(c), "consent for another host");
}

#[test]
fn stt_consent_does_not_grant_cleanup_consent() {
    let mut c = cfg();
    let o = &mut c.cloud.openrouter;
    o.stt.consent_host = "openrouter.ai".into();
    o.stt.consent_at_ms = 1;
    o.cleanup.consent_at_ms = 0;
    let k = rig(c);
    assert_no_cloud_cleanup(&k, "only STT consent");
    assert_eq!(
        k.router.cleanup_status().blocked.as_deref(),
        Some("cloud_cleanup_not_configured")
    );
    // And cleanup consent alone never enables cloud STT.
    let k = rig(cfg());
    let cs = k.router.config();
    assert!(!cs.cloud.openrouter.stt.consent_ok() && !cs.allow_cloud);
}

#[test]
fn consent_revoked_mid_session_stops_the_cloud_call() {
    let k = rig(cfg());
    k.router.start_session(session()).unwrap();
    assert!(k.router.push_audio(&speech(300)));
    let mut c = cfg();
    c.cloud.openrouter.cleanup.consent_at_ms = 0;
    k.router.update_config(c);
    let r = k.router.commit();
    assert_eq!(k.cloud.calls(), 0);
    assert!(r.final_text.contains(LOCAL_OUT), "{r:?}");
}

#[test]
fn toggle_off_mid_session_stops_the_cloud_call() {
    let k = rig(cfg());
    k.router.start_session(session()).unwrap();
    assert!(k.router.push_audio(&speech(300)));
    k.router
        .set_cleanup(
            false,
            vec![CleanupRoute::Cloud, CleanupRoute::Local],
            consented().cleanup,
        )
        .unwrap();
    k.router.commit();
    assert_eq!(k.cloud.calls(), 0);
}

// ---- order --------------------------------------------------------------------

#[test]
fn local_first_uses_cloud_only_without_a_local_llm() {
    let mut c = cfg();
    c.cleanup_order = vec![CleanupRoute::Local, CleanupRoute::Cloud];
    let k = rig(c.clone());
    let r = dictate(&k.router);
    assert!(r.final_text.contains(LOCAL_OUT));
    assert_eq!(k.cloud.calls(), 0);
    let k = rig_opts(c, true, false, true);
    let r = dictate(&k.router);
    assert!(r.final_text.contains(CLOUD_OUT), "{r:?}");
    assert_eq!(k.cloud.calls(), 1);
}

#[test]
fn cloud_only_order_without_local_llm_works_and_falls_back_to_deterministic() {
    let mut c = cfg();
    c.cleanup_order = vec![CleanupRoute::Cloud];
    let k = rig_opts(c, true, false, true);
    assert!(dictate(&k.router).final_text.contains(CLOUD_OUT));
    k.cloud
        .fail(Some(CleanError::Unavailable("connect_failed".into())));
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(
        r.fallback_reason.as_deref(),
        Some("llm_error: connect_failed")
    );
}

// ---- circuit ------------------------------------------------------------------

#[test]
fn auth_failure_falls_back_now_and_is_sticky_until_the_key_changes() {
    let k = rig(cfg());
    k.cloud
        .fail(Some(CleanError::Unavailable("auth_failed".into())));
    let r = dictate(&k.router);
    assert!(
        r.final_text.contains(LOCAL_OUT),
        "fast failure → local LLM: {r:?}"
    );
    assert_eq!((k.cloud.calls(), local_calls(&k)), (1, 1));
    let cc = k.router.route_status().cleanup_circuit.unwrap();
    assert!(cc.open && cc.until_config_change);
    assert_eq!(cc.reason.as_deref(), Some("auth_failed"));
    // Circuit open: no request, straight to the local LLM.
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
    // STT circuit untouched.
    assert!(k.router.route_status().cloud_circuit.is_none());
    // New key: retried.
    k.cloud.fail(None);
    k.router
        .set_cloud_key("openrouter", Some(SecretString::new("sk-or-new-0000")));
    assert!(dictate(&k.router).final_text.contains(CLOUD_OUT));
    assert_eq!(k.cloud.calls(), 2);
}

#[test]
fn provider_error_backs_off_and_falls_back_to_the_local_pipeline() {
    let k = rig(cfg());
    k.cloud
        .fail(Some(CleanError::Backend("provider_error".into())));
    let r = dictate(&k.router);
    // D30: any AI cleanup failure runs today's pipeline (rules + local LLM).
    assert_eq!(local_calls(&k), 1);
    assert!(r.final_text.contains(LOCAL_OUT), "{r:?}");
    let rt = r.route.unwrap();
    assert_eq!(rt.cleanup_route.as_deref(), Some("local"));
    assert_eq!(
        rt.cloud_fallback.as_deref(),
        Some("llm_error: provider_error")
    );
    // The detail reaches the route for the debug log.
    assert_eq!(rt.fallback_detail.as_deref(), Some("error: provider_error"));
    assert_eq!(rt.llm_engine.as_deref(), Some("local"));
    let cc = k.router.cleanup_status().circuit.unwrap();
    assert!(cc.open && !cc.until_config_change);
    let r = dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1, "open circuit skips the cloud");
    assert_eq!(local_calls(&k), 2, "…to the local LLM");
    assert_eq!(
        r.route.unwrap().cloud_fallback.as_deref(),
        Some("llm_error: circuit_open")
    );
    k.router.reset_circuit();
    k.cloud.fail(None);
    let r = dictate(&k.router);
    assert_eq!(k.cloud.calls(), 2);
    assert_eq!(r.route.unwrap().cleanup_route.as_deref(), Some("cloud"));
}

#[test]
fn no_private_route_is_sticky_until_the_model_changes() {
    let k = rig(cfg());
    k.cloud
        .fail(Some(CleanError::Unavailable("no_private_route".into())));
    dictate(&k.router);
    let cc = k.router.cleanup_status().circuit.unwrap();
    assert!(cc.until_config_change);
    assert_eq!(cc.reason.as_deref(), Some("no_private_route"));
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
    // Same config again: still open; another model: retried.
    k.router.update_config(k.router.config());
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
    let mut c = k.router.config();
    c.cloud.openrouter.cleanup.model = "meta-llama/llama-3.1-8b-instruct".into();
    k.router.update_config(c);
    k.cloud.fail(None);
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 2);
}

// ---- budget -------------------------------------------------------------------

#[test]
fn budget_uses_the_estimate_before_the_call() {
    let mut c = cfg();
    c.cloud.monthly_budget_usd = Some(Usd(0.0005));
    let k = rig(c);
    // Estimate 0.001 > budget 0.0005: skipped before any request.
    let r = dictate(&k.router);
    assert_eq!(k.cloud.calls(), 0);
    assert!(r.final_text.contains(LOCAL_OUT));
    assert_eq!(
        k.router.cleanup_status().blocked,
        None,
        "budget is per call, not static"
    );
    // Budget large enough for exactly one call (0.0007 recorded, then
    // 0.0007 + 0.001 > 0.0015).
    let mut c = k.router.config();
    c.cloud.monthly_budget_usd = Some(Usd(0.0015));
    k.router.update_config(c);
    dictate(&k.router);
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
    // STT spending counts too (one budget for both uses).
    k.router.reset_cloud_usage();
    k.router.set_cloud_usage(
        serde_json::from_value(serde_json::json!({
            "month": month_of(now_ms()),
            "providers": [{"provider_id": "openrouter", "purpose": "stt", "cost_usd": 0.0012}]
        }))
        .unwrap(),
    );
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
}

#[test]
fn unpriced_model_with_a_budget_waits_for_a_real_cost() {
    let mut c = cfg();
    c.cloud.monthly_budget_usd = Some(Usd(1.0));
    let k = rig(c);
    *k.cloud.estimate.lock().unwrap() = None;
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 0, "no estimate, no recorded cost: refused");
    // Without a budget it runs; its real cost then prices later calls.
    let mut c = k.router.config();
    c.cloud.monthly_budget_usd = None;
    k.router.update_config(c.clone());
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 1);
    c.cloud.monthly_budget_usd = Some(Usd(1.0));
    k.router.update_config(c);
    dictate(&k.router);
    assert_eq!(k.cloud.calls(), 2, "last cost × 2 is the estimate now");
}

// ---- never blocks dictation ---------------------------------------------------

#[test]
fn slow_cloud_hits_the_deadline_and_falls_back_to_the_local_pipeline() {
    let mut c = cfg();
    c.cloud.openrouter.cleanup.timeout_ms = 800;
    let k = rig(c);
    *k.cloud.delay.lock().unwrap() = Duration::from_secs(4);
    k.router.start_session(session()).unwrap();
    assert!(k.router.push_audio(&speech(300)));
    let t = Instant::now();
    let r = k.router.commit();
    let took = t.elapsed();
    // D30: the timeout runs today's pipeline; the local LLM answers.
    assert!(r.final_text.contains(LOCAL_OUT), "{r:?}");
    assert_eq!(r.cloud_fallback.as_deref(), Some("llm_timeout"));
    assert_eq!(r.cleanup_route.as_deref(), Some("local"));
    assert!(took < Duration::from_millis(800 + 600), "{took:?}");
    assert_eq!(local_calls(&k), 1);
}

#[test]
fn default_cleanup_timeout_is_2500_ms() {
    let k = rig(cfg());
    assert_eq!(k.router.cleanup_status().timeout_ms, 2500);
}

// ---- routes ---------------------------------------------------------------------

/// D36: with the server first in the cleanup order, a server-route
/// dictation is cleaned in its own session and never reaches the cloud
/// (cloud first: see `d36_cleanup_order.rs`).
#[test]
fn server_first_cleans_in_session_and_never_uses_cloud_cleanup() {
    let server = Arc::new(Server {
        installs: AtomicUsize::new(0),
    });
    let s2 = server.clone();
    let mut c = cfg();
    c.allow_server = true;
    c.order = vec![Route::Server, Route::Client];
    c.cleanup_order = vec![
        CleanupRoute::Server,
        CleanupRoute::Cloud,
        CleanupRoute::Local,
    ];
    c.server = ServerConfig {
        host: "100.64.0.10".into(),
        token: "0123456789abcdef".into(),
        ..Default::default()
    };
    let cloud = Cloud::new();
    let c2 = cloud.clone();
    let router = Router::new(
        c,
        None,
        Some(Box::new(move |_s: &ServerConfig| {
            s2.clone() as Arc<dyn Backend>
        })),
    )
    .with_cleanup_factory(Box::new(move |_o, _k| c2.clone() as Arc<dyn Cleaner>));
    router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    for _ in 0..3 {
        let r = dictate(&router);
        assert_eq!(r.final_text, "server text");
        let rt = r.route.unwrap();
        assert_eq!(rt.engine, "remote");
        assert_eq!(rt.llm_engine.as_deref(), Some("server"));
    }
    assert_eq!(cloud.calls(), 0);
    assert_eq!(
        server.installs.load(Ordering::SeqCst),
        0,
        "plan never offered to the server"
    );
}

#[test]
fn cloud_stt_backend_gets_the_cleanup_plan() {
    struct CloudStt(Mutex<Vec<bool>>);
    impl Backend for CloudStt {
        fn kind(&self) -> EngineKind {
            EngineKind::Cloud
        }
        fn ready(&self) -> bool {
            true
        }
        fn start_session(&self, _c: &SessionConfig) -> Result<StartInfo, BackendError> {
            Ok(StartInfo::default())
        }
        fn push_audio(&self, _p: &[i16]) -> bool {
            true
        }
        fn set_context(&self, _c: &FieldContext) {}
        fn partial(&self) -> Option<String> {
            None
        }
        fn commit(&self, _d: Instant) -> Result<CommitResult, BackendError> {
            Err(BackendError::new(Failure::Error, "unused"))
        }
        fn cancel(&self) {}
        fn probe(&self) -> HealthReport {
            HealthReport::default()
        }
        // D36: a plan is always installed; record whether it cleans
        // without an on-device LLM (i.e. reaches the cloud).
        fn install_cleanup(&self, p: Option<CleanupPlan>) {
            self.0
                .lock()
                .unwrap()
                .push(p.and_then(|p| p(None)).is_some());
        }
    }
    let stt = Arc::new(CloudStt(Mutex::new(Vec::new())));
    let s2 = stt.clone();
    let mut c = cfg();
    c.allow_cloud = true;
    c.order = vec![Route::Cloud, Route::Client];
    c.cloud.openrouter.stt.consent_host = "openrouter.ai".into();
    c.cloud.openrouter.stt.consent_at_ms = 1;
    let router = Router::new(c, None, None)
        .with_cloud(Box::new(move |_o, _k| s2.clone() as Arc<dyn Backend>))
        .with_cleanup_factory(Box::new(|_o, _k| Cloud::new() as Arc<dyn Cleaner>));
    router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    router.start_session(session()).unwrap();
    router.cancel();
    assert_eq!(stt.0.lock().unwrap().first(), Some(&true));
    // Cleanup off: the plan is withdrawn.
    let mut c = router.config();
    c.allow_cloud_cleanup = false;
    router.update_config(c);
    assert_eq!(stt.0.lock().unwrap().last(), Some(&false));
}

// ---- prompt scope -------------------------------------------------------------

fn leaky() -> FieldContext {
    FieldContext {
        app_id: "com.secret.appid".into(),
        before: "hello BEFORESECRET".into(),
        selected: "SELECTEDSECRET".into(),
        after: "AFTERSECRET".into(),
        screen_words: vec!["SCREENSECRET".into()],
        ..Default::default()
    }
}

fn sent(k: &Rig) -> String {
    let p = k.cloud.seen.lock().unwrap().last().cloned().unwrap();
    let mut s = format!("{}{}", p.system, p.user);
    for (u, a) in p.examples {
        s += &u;
        s += &a;
    }
    s
}

#[test]
fn by_default_nothing_beyond_the_dictation_reaches_openrouter() {
    let k = rig(cfg());
    dictate_with(&k.router, leaky());
    let s = sent(&k);
    for x in [
        "BEFORESECRET",
        "SELECTEDSECRET",
        "AFTERSECRET",
        "SCREENSECRET",
        "com.secret.appid",
    ] {
        assert!(!s.contains(x), "{x} sent to the cloud");
    }
    assert!(s.contains(IN));
}

#[test]
fn each_toggle_admits_its_context() {
    let mut c = cfg();
    c.cloud.openrouter.cleanup.send_surrounding_text = true;
    c.cloud.openrouter.cleanup.send_app_name = true;
    let k = rig(c);
    dictate_with(&k.router, leaky());
    let s = sent(&k);
    assert!(s.contains("BEFORESECRET") && s.contains("com.secret.appid"));
    assert!(!s.contains("SCREENSECRET"));
}

#[test]
fn the_key_never_appears_in_router_output() {
    let k = rig(cfg());
    let r = dictate(&k.router);
    for o in [
        serde_json::to_string(&r).unwrap(),
        serde_json::to_string(&k.router.cleanup_status()).unwrap(),
        serde_json::to_string(&k.router.route_status()).unwrap(),
        serde_json::to_string(&k.router.config()).unwrap(),
        serde_json::to_string(&k.router.cloud_usage()).unwrap(),
    ] {
        assert!(!o.contains("LEAKCHECK"), "key leaked: {o}");
    }
    assert_eq!(k.factory_calls.load(Ordering::SeqCst), 1);
}

// ---- Command Mode: one-off LLM requests take the cleanup route ------------

fn command(
    r: &Router,
    base: Option<Arc<dyn Cleaner>>,
    selected: Option<&str>,
) -> Result<String, murmur_core::command::CommandError> {
    murmur_core::command::run(
        r.command_cleaner(base),
        "make it formal",
        selected,
        Duration::from_secs(5),
    )
}

#[test]
fn command_cleaner_follows_the_cleanup_route() {
    // Cloud first with consent + key: the gate, metered like cleanup.
    let k = rig(cfg());
    let local = k.local_llm.clone() as Arc<dyn Cleaner>;
    assert_eq!(
        command(&k.router, Some(local), None).as_deref(),
        Ok(CLOUD_OUT)
    );
    assert_eq!((k.cloud.calls(), local_calls(&k)), (1, 0));
    assert!(k.router.cloud_usage().cost_for("cleanup") > 0.0);

    // Cloud cleanup not allowed: the on-device LLM only, never the cloud.
    let k = rig(RouterConfig {
        allow_cloud_cleanup: false,
        ..cfg()
    });
    let local = k.local_llm.clone() as Arc<dyn Cleaner>;
    assert_eq!(
        command(&k.router, Some(local), None).as_deref(),
        Ok(LOCAL_OUT)
    );
    assert_eq!(k.cloud.calls(), 0);
    assert!(k.router.command_cleaner(None).is_none(), "no LLM at all");
    assert_eq!(
        command(&k.router, None, None),
        Err(murmur_core::command::CommandError::NoLlm)
    );

    // Local first: the loaded LLM wins; without one, the cloud.
    let k = rig(RouterConfig {
        cleanup_order: vec![CleanupRoute::Local, CleanupRoute::Cloud],
        ..cfg()
    });
    let local = k.local_llm.clone() as Arc<dyn Cleaner>;
    assert_eq!(
        command(&k.router, Some(local), None).as_deref(),
        Ok(LOCAL_OUT)
    );
    assert_eq!(command(&k.router, None, None).as_deref(), Ok(CLOUD_OUT));
}

#[test]
fn command_selection_stays_on_device_unless_surrounding_text_is_allowed() {
    use murmur_core::command::CommandError;
    let k = rig(cfg());
    let local = k.local_llm.clone() as Arc<dyn Cleaner>;
    assert_eq!(
        command(&k.router, Some(local), Some("SELECTEDSECRET")).as_deref(),
        Ok(LOCAL_OUT),
        "cloud first, but the selection goes to the on-device part"
    );
    assert_eq!(
        command(&k.router, None, Some("SELECTEDSECRET")),
        Err(CommandError::SelectionNotAllowed)
    );
    assert_eq!(k.cloud.calls(), 0, "the selection never reached OpenRouter");

    let mut c = cfg();
    c.cloud.openrouter.cleanup.send_surrounding_text = true;
    let k = rig(c);
    assert_eq!(
        command(&k.router, None, Some("SELECTEDSECRET")).as_deref(),
        Ok(CLOUD_OUT)
    );
    assert!(sent(&k).contains("SELECTEDSECRET"));
}

#[test]
fn command_cleaner_rechecks_consent_per_call() {
    let k = rig(cfg());
    let cl = k.router.command_cleaner(None);
    assert!(cl.is_some());
    let mut c = cfg();
    c.cloud.openrouter.cleanup.consent_at_ms = 0;
    k.router.update_config(c);
    let r = murmur_core::command::run(cl, "write hello", None, Duration::from_secs(5));
    assert!(r.is_err(), "{r:?}");
    assert_eq!(k.cloud.calls(), 0, "consent revoked: nothing sent");
}

// ---- Router::command: the router finds the on-device LLM itself (Android) --

#[test]
fn router_command_uses_the_local_engines_llm() {
    use murmur_core::command::CommandError;
    const T: Duration = Duration::from_secs(5);
    // Cloud cleanup off: the warm local engine's LLM, no base from the shell.
    let k = rig(RouterConfig {
        allow_cloud_cleanup: false,
        ..cfg()
    });
    assert_eq!(
        k.router
            .command("make it shorter", Some("a long text"), T)
            .as_deref(),
        Ok(LOCAL_OUT)
    );
    assert_eq!((k.cloud.calls(), local_calls(&k)), (0, 1));

    // Cloud first, a selection, surrounding text not allowed: the on-device part.
    let k = rig(cfg());
    assert_eq!(
        k.router
            .command("make it shorter", Some("SELECTEDSECRET"), T)
            .as_deref(),
        Ok(LOCAL_OUT)
    );
    assert_eq!(k.cloud.calls(), 0, "the selection never reached OpenRouter");

    // No local LLM and no cloud cleanup: nothing to ask, nothing called.
    let no_cloud = RouterConfig {
        allow_cloud_cleanup: false,
        ..cfg()
    };
    let k = rig_opts(no_cloud, true, false, true);
    assert_eq!(
        k.router.command("make it shorter", Some("x"), T),
        Err(CommandError::NoLlm)
    );
    // No local LLM, cloud only, a selection without surrounding-text consent.
    let k = rig_opts(cfg(), true, false, true);
    assert_eq!(
        k.router.command("make it shorter", Some("x"), T),
        Err(CommandError::SelectionNotAllowed)
    );
    assert_eq!(k.cloud.calls(), 0);
    // ... while generating at the cursor may use the cloud.
    assert_eq!(
        k.router.command("write hello", None, T).as_deref(),
        Ok(CLOUD_OUT)
    );
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
