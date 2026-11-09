//! Cloud route policy (docs/cloud-stt.md §3, invariant C1) with fake
//! backends: no network, no models.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use murmur_router::*;

const KEY: &str = "sk-LEAKCHECK-policy-0123456789";

struct Fake {
    kind: EngineKind,
    ready: AtomicBool,
    start_fail: Mutex<Option<Failure>>,
    commit_fail: Mutex<Option<Failure>>,
    /// Session dies (session_failure) after this many samples.
    die_after: Mutex<Option<u64>>,
    received: AtomicU64,
    starts: AtomicUsize,
    commits: AtomicUsize,
    probes: AtomicUsize,
}

impl Fake {
    fn new(kind: EngineKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            ready: AtomicBool::new(true),
            start_fail: Mutex::new(None),
            commit_fail: Mutex::new(None),
            die_after: Mutex::new(None),
            received: AtomicU64::new(0),
            starts: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            probes: AtomicUsize::new(0),
        })
    }
    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
    fn received(&self) -> u64 {
        self.received.load(Ordering::SeqCst)
    }
    fn fail_commit(&self, f: Option<Failure>) {
        *self.commit_fail.lock().unwrap() = f;
    }
    fn fail_start(&self, f: Option<Failure>) {
        *self.start_fail.lock().unwrap() = f;
    }
    fn dead(&self) -> bool {
        self.die_after
            .lock()
            .unwrap()
            .is_some_and(|n| self.received() >= n)
    }
}

fn result(text: &str) -> CommitResult {
    serde_json::from_value(serde_json::json!({
        "status": "RAW_TRANSCRIPT", "asr_text": text, "final_text": text, "language": "en",
        "timings": {"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,"total_ms":0,"dropped_samples":0},
        "press_enter": false, "fallback_reason": null, "truncated": false, "error": null
    }))
    .unwrap()
}

impl Backend for Fake {
    fn kind(&self) -> EngineKind {
        self.kind
    }
    fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
    fn start_session(&self, _cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(f) = *self.start_fail.lock().unwrap() {
            return Err(BackendError::new(f, "fake start failure"));
        }
        self.received.store(0, Ordering::SeqCst);
        Ok(StartInfo::default())
    }
    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.received.fetch_add(pcm.len() as u64, Ordering::SeqCst);
        true
    }
    fn set_context(&self, _ctx: &FieldContext) {}
    fn partial(&self) -> Option<String> {
        None
    }
    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        assert!(deadline > Instant::now());
        if let Some(f) = *self.commit_fail.lock().unwrap() {
            return Err(BackendError::new(f, "fake commit failure"));
        }
        let mut r = result(&format!("{} {}", self.kind.as_str(), self.received()));
        if self.kind == EngineKind::Cloud {
            r.route = Some(RouteInfo {
                engine: "cloud".into(),
                provider: Some("openrouter".into()),
                model: Some("openai/whisper-large-v3-turbo".into()),
                billed_audio_ms: Some(1000),
                cost_usd: Some(Usd(0.5)),
                cost_estimated: Some(false),
                ..Default::default()
            });
        }
        Ok(r)
    }
    fn cancel(&self) {}
    fn probe(&self) -> HealthReport {
        self.probes.fetch_add(1, Ordering::SeqCst);
        HealthReport {
            ok: true,
            engine: self.kind,
            transcript: Some("and so my fellow americans".into()),
            ..Default::default()
        }
    }
    fn session_failure(&self) -> Option<BackendError> {
        self.dead()
            .then(|| BackendError::new(Failure::Disconnected, "fake died"))
    }
}

fn provider() -> OpenRouterConfig {
    let mut o = OpenRouterConfig::default();
    o.stt.model = "openai/whisper-large-v3-turbo".into();
    o.stt.consent_host = "openrouter.ai".into();
    o.stt.consent_at_ms = 1_760_000_000_000;
    o
}

/// Everything C1 asks for: allow_cloud + consent + order lists cloud (the
/// key is set by `rig`).
fn cloud_cfg(order: &[Route]) -> RouterConfig {
    RouterConfig {
        allow_cloud: true,
        order: order.to_vec(),
        server: ServerConfig {
            host: "100.64.0.10".into(),
            token: "0123456789abcdef".into(),
            ..Default::default()
        },
        cloud: CloudConfig {
            openrouter: provider(),
            ..Default::default()
        },
        ..Default::default()
    }
}

struct Rig {
    router: Router,
    local: Arc<Fake>,
    remote: Arc<Fake>,
    cloud: Arc<Fake>,
    cloud_calls: Arc<AtomicUsize>,
    keys_seen: Arc<Mutex<Vec<String>>>,
}

fn rig_opts(c: RouterConfig, with_cloud: bool, key: bool) -> Rig {
    let (local, remote, cloud) = (
        Fake::new(EngineKind::Local),
        Fake::new(EngineKind::Remote),
        Fake::new(EngineKind::Cloud),
    );
    let r2 = remote.clone();
    let rf: RemoteFactory = Box::new(move |_s: &ServerConfig| r2.clone() as Arc<dyn Backend>);
    let calls = Arc::new(AtomicUsize::new(0));
    let keys = Arc::new(Mutex::new(Vec::new()));
    let mut router = Router::new(c, Some(local.clone() as Arc<dyn Backend>), Some(rf));
    if with_cloud {
        let (c2, calls2, keys2) = (cloud.clone(), calls.clone(), keys.clone());
        router = router
            .with_cloud(Box::new(move |_p: &OpenRouterConfig, k: SecretString| {
                calls2.fetch_add(1, Ordering::SeqCst);
                keys2.lock().unwrap().push(k.expose().to_string());
                c2.clone() as Arc<dyn Backend>
            }))
            .with_cloud_adapters(vec!["openrouter".into()]);
    }
    if key {
        router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    }
    Rig {
        router,
        local,
        remote,
        cloud,
        cloud_calls: calls,
        keys_seen: keys,
    }
}

fn rig(c: RouterConfig) -> Rig {
    rig_opts(c, true, true)
}

const N: usize = 16_000;

fn dictate(r: &Router) -> CommitResult {
    for _ in 0..10 {
        assert!(r.push_audio(&[100i16; N / 10]));
    }
    r.commit()
}

fn route(r: &CommitResult) -> RouteInfo {
    r.route.clone().expect("route set")
}

fn assert_cloud_unused(k: &Rig) {
    assert_eq!(k.cloud.starts(), 0, "cloud backend must not start");
    assert_eq!(
        k.cloud_calls.load(Ordering::SeqCst),
        0,
        "factory not called"
    );
}

// ---- C1: cloud never used unless all four conditions hold ------------------

#[test]
fn cloud_with_all_conditions_is_the_primary() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "cloud 16000");
    let rt = route(&r);
    assert_eq!(rt.engine, "cloud");
    assert_eq!(rt.fallback, None);
    assert_eq!(rt.provider.as_deref(), Some("openrouter"));
    assert_eq!(rt.model.as_deref(), Some("openai/whisper-large-v3-turbo"));
    assert_eq!(rt.billed_audio_ms, Some(1000));
    assert_eq!(rt.cost_usd, Some(Usd(0.5)));
    assert_eq!(rt.server, None, "no server involved");
    assert_eq!(k.local.starts(), 0);
    assert_eq!(*k.keys_seen.lock().unwrap(), [KEY]);
}

#[test]
fn cloud_never_used_without_allow_cloud() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.allow_cloud = false;
    let k = rig(c);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(
        route(&r).fallback,
        None,
        "a disabled route is not a fallback"
    );
    assert_cloud_unused(&k);
}

#[test]
fn cloud_never_used_without_consent() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.openrouter.stt.consent_at_ms = 0;
    let k = rig(c);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback.as_deref(), Some("cloud_not_configured"));
    assert_cloud_unused(&k);
}

#[test]
fn cloud_never_used_when_consent_was_for_another_host() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.openrouter.stt.consent_host = "api.elsewhere.example".into();
    let k = rig(c);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
    assert_cloud_unused(&k);
}

#[test]
fn cloud_never_used_without_key() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud, Route::Client]), true, false);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback.as_deref(), Some("cloud_not_configured"));
    assert_cloud_unused(&k);
    // A cleared key is the same as none.
    k.router
        .set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    k.router.set_cloud_key("openrouter", None);
    assert!(!k.router.has_cloud_key("openrouter"));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
    assert_cloud_unused(&k);
}

/// D78: "one Cloud provider at a time" lives in the Windows settings, not
/// the router. Android still pairs LiteLLM cleanup with OpenRouter speech
/// (D52), so with LiteLLM as the cleanup provider cloud speech-to-text
/// still goes to OpenRouter.
#[test]
fn litellm_cleanup_keeps_openrouter_cloud_speech() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.provider = CloudProvider::LiteLlm;
    c.cloud.custom_cleanup.base_url = "http://10.0.0.5:4000/v1".into();
    c.cloud.custom_cleanup.model = "local-cleanup".into();
    c.validate().unwrap();
    let k = rig(c);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "cloud");
    assert_eq!(route(&r).fallback, None);
}

#[test]
fn cloud_never_used_when_order_is_empty_or_omits_cloud() {
    // Empty order = legacy D17 policy, which never includes the cloud.
    for order in [vec![], vec![Route::Server, Route::Client]] {
        let k = rig(cloud_cfg(&order));
        k.router.start_session(SessionConfig::default()).unwrap();
        assert_eq!(route(&dictate(&k.router)).engine, "remote");
        k.remote.fail_start(Some(Failure::ConnectFailed));
        k.router.reset_circuit();
        k.router.start_session(SessionConfig::default()).unwrap();
        assert_eq!(route(&dictate(&k.router)).engine, "local");
        assert_cloud_unused(&k);
    }
}

#[test]
fn cloud_never_used_in_a_build_without_the_cloud_factory() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud, Route::Client]), false, true);
    assert!(!k.router.cloud_capabilities().cloud);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(route(&r).fallback, None);
    assert!(k.router.route_status().cloud_circuit.is_none());
}

#[test]
fn cloud_only_order_without_consent_is_no_engine_enabled() {
    let mut c = cloud_cfg(&[Route::Cloud]);
    c.cloud.openrouter.stt.consent_at_ms = 0;
    let k = rig(c);
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e, RouterError::NoEngineEnabled);
    let r = k.router.commit();
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(route(&r).engine, "cloud");
    assert_eq!(route(&r).fallback.as_deref(), Some("cloud_not_configured"));
    assert!(r.error.unwrap().contains("no consent"));
}

// ---- order walking ---------------------------------------------------------

#[test]
fn duplicate_routes_in_order_are_bad_config() {
    let c = cloud_cfg(&[Route::Cloud, Route::Client, Route::Cloud]);
    assert!(matches!(c.validate(), Err(RouterError::BadConfig(_))));
    let k = rig(c);
    let e = k
        .router
        .start_session(SessionConfig::default())
        .unwrap_err();
    assert_eq!(e.kind(), "bad_config");
    assert_cloud_unused(&k);
}

#[test]
fn route_order_follows_order_and_toggles() {
    let c = cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]);
    assert_eq!(
        c.route_order(),
        [EngineKind::Remote, EngineKind::Cloud, EngineKind::Local]
    );
    let mut c2 = c.clone();
    c2.allow_cloud = false;
    assert_eq!(c2.route_order(), [EngineKind::Remote, EngineKind::Local]);
    c2.order.clear();
    assert_eq!(c2.route_order(), [EngineKind::Remote, EngineKind::Local]);
}

#[test]
fn cloud_commit_failure_replays_everything_to_local() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.cloud.fail_commit(Some(Failure::ProviderError));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "local 16000", "100 % of the samples replayed");
    let rt = route(&r);
    assert_eq!(rt.engine, "local");
    assert_eq!(rt.fallback.as_deref(), Some("provider_error"));
    assert_eq!(rt.provider, None, "provider is set only on cloud results");
    assert_eq!(rt.cost_usd, None);
    // Nothing reached the provider successfully: no usage recorded.
    assert!(k.router.cloud_usage().providers.is_empty());
}

#[test]
fn server_then_cloud_uses_cloud_only_when_server_is_down() {
    let k = rig(cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "remote");
    assert_eq!(k.cloud.starts(), 0);
    k.remote.fail_start(Some(Failure::ConnectFailed));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    let rt = route(&r);
    assert_eq!(rt.engine, "cloud");
    assert_eq!(rt.fallback.as_deref(), Some("connect_failed"));
    assert_eq!(rt.provider.as_deref(), Some("openrouter"));
    assert_eq!(r.final_text, "cloud 16000");
}

#[test]
fn server_dying_mid_session_replays_to_cloud() {
    let k = rig(cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]));
    *k.remote.die_after.lock().unwrap() = Some(8000);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "cloud 16000");
    assert_eq!(route(&r).fallback.as_deref(), Some("disconnected"));
    assert_eq!(k.local.starts(), 0);
}

#[test]
fn three_hop_chain_keeps_the_first_reason_and_all_samples() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Server, Route::Client]));
    k.cloud.fail_commit(Some(Failure::Timeout));
    k.remote.fail_commit(Some(Failure::Disconnected));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.final_text, "local 16000");
    let rt = route(&r);
    assert_eq!(rt.engine, "local");
    assert_eq!(rt.fallback.as_deref(), Some("timeout"));
    assert_eq!(k.remote.starts(), 1);
    assert_eq!(k.remote.received(), 16000, "server got the full replay too");
}

#[test]
fn client_then_cloud_uses_cloud_only_without_a_local_model() {
    let k = rig(cloud_cfg(&[Route::Client, Route::Cloud]));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
    k.local.ready.store(false, Ordering::SeqCst);
    k.router.start_session(SessionConfig::default()).unwrap();
    let rt = route(&dictate(&k.router));
    assert_eq!(rt.engine, "cloud");
    assert_eq!(rt.fallback.as_deref(), Some("local_not_ready"));
}

#[test]
fn cloud_failure_without_fallback_is_an_error_result() {
    let k = rig(cloud_cfg(&[Route::Cloud]));
    k.cloud.fail_commit(Some(Failure::ConnectFailed));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&k.router);
    assert_eq!(r.status, CommitStatus::Error);
    assert_eq!(route(&r).fallback.as_deref(), Some("connect_failed"));
}

// ---- circuits ----------------------------------------------------------------

#[test]
fn cloud_auth_failure_is_sticky_until_the_key_changes() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.cloud.fail_commit(Some(Failure::AuthFailed));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("auth_failed")
    );
    let cc = k.router.route_status().cloud_circuit.unwrap();
    assert!(cc.open && cc.until_config_change);
    assert_eq!(cc.reason.as_deref(), Some("auth_failed"));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("circuit_open")
    );
    assert_eq!(k.cloud.starts(), 1, "not retried");
    // Same config again: still open.
    k.router.update_config(k.router.config());
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert_eq!(k.cloud.starts(), 1);
    // New key: retried.
    k.cloud.fail_commit(None);
    k.router
        .set_cloud_key("openrouter", Some(SecretString::new("sk-new-key-0000")));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
    assert_eq!(
        k.keys_seen.lock().unwrap().last().unwrap(),
        "sk-new-key-0000"
    );
}

#[test]
fn cloud_quota_failure_is_sticky_and_provider_change_resets_it() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.cloud.fail_commit(Some(Failure::QuotaExceeded));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("quota_exceeded")
    );
    assert!(
        k.router
            .route_status()
            .cloud_circuit
            .unwrap()
            .until_config_change
    );
    let mut c = k.router.config();
    c.cloud.openrouter.stt.model = "elevenlabs/scribe-v2".into();
    k.router.update_config(c);
    k.cloud.fail_commit(None);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
}

#[test]
fn cloud_provider_error_backs_off_then_retries() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.cloud.fail_commit(Some(Failure::ProviderError));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    let cc = k.router.route_status().cloud_circuit.unwrap();
    assert!(cc.open && !cc.until_config_change);
    assert!(cc.retry_in_ms.unwrap() <= 5000);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("circuit_open")
    );
    // Remote circuit untouched.
    assert!(!k.router.route_status().circuit.open);
    k.router.reset_circuit();
    k.cloud.fail_commit(None);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
}

#[test]
fn open_cloud_circuit_without_fallback_still_tries_the_cloud() {
    let k = rig(cloud_cfg(&[Route::Cloud]));
    k.cloud.fail_commit(Some(Failure::Timeout));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    k.cloud.fail_commit(None);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
}

// ---- usage and budget --------------------------------------------------------

#[test]
fn usage_counts_cloud_dictations_and_budget_stops_the_route() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.monthly_budget_usd = Some(Usd(1.0));
    let k = rig(c);
    for _ in 0..2 {
        k.router.start_session(SessionConfig::default()).unwrap();
        assert_eq!(route(&dictate(&k.router)).engine, "cloud");
    }
    let u = k.router.cloud_usage();
    assert_eq!(u.month, month_of(now_ms()));
    assert_eq!(u.providers.len(), 1);
    assert_eq!(u.providers[0].provider_id, "openrouter");
    assert_eq!(u.providers[0].requests, 2);
    assert_eq!(u.providers[0].billed_audio_ms, 2000);
    assert_eq!(u.providers[0].cost_usd, Usd(1.0));
    // $1.00 spent of a $1.00 budget: the route is skipped.
    k.router.start_session(SessionConfig::default()).unwrap();
    let rt = route(&dictate(&k.router));
    assert_eq!(rt.engine, "local");
    assert_eq!(rt.fallback.as_deref(), Some("budget_exceeded"));
    assert_eq!(k.cloud.starts(), 2);
    // Reset counters: cloud again.
    k.router.reset_cloud_usage();
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
}

#[test]
fn persisted_usage_is_restored_and_old_months_dropped() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.monthly_budget_usd = Some(Usd(2.0));
    let k = rig(c);
    let this = CloudUsage {
        month: month_of(now_ms()),
        providers: vec![ProviderUsage {
            provider_id: "openrouter".into(),
            purpose: "stt".into(),
            requests: 9,
            billed_audio_ms: 90_000,
            cost_usd: Usd(2.5),
            cost_estimated: true,
        }],
    };
    k.router.set_cloud_usage(this.clone());
    assert_eq!(k.router.cloud_usage(), this);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(
        route(&dictate(&k.router)).fallback.as_deref(),
        Some("budget_exceeded")
    );
    k.router.set_cloud_usage(CloudUsage {
        month: "2001-01".into(),
        ..this
    });
    assert!(k.router.cloud_usage().providers.is_empty());
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "cloud");
}

#[test]
fn silence_records_no_usage() {
    // A cloud result without billed audio (VAD found no speech, nothing
    // was uploaded) is not a request.
    struct Silent;
    impl Backend for Silent {
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
            let mut r = result("");
            r.status = CommitStatus::Empty;
            r.route = Some(RouteInfo {
                engine: "cloud".into(),
                provider: Some("openrouter".into()),
                ..Default::default()
            });
            Ok(r)
        }
        fn cancel(&self) {}
        fn probe(&self) -> HealthReport {
            HealthReport::default()
        }
    }
    let router = Router::new(cloud_cfg(&[Route::Cloud]), None, None)
        .with_cloud(Box::new(|_p, _k| Arc::new(Silent) as Arc<dyn Backend>));
    router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    router.start_session(SessionConfig::default()).unwrap();
    let r = dictate(&router);
    assert_eq!(r.status, CommitStatus::Empty);
    assert!(router.cloud_usage().providers.is_empty());
}

// ---- probe, capabilities, keys ------------------------------------------------

#[test]
fn probe_cloud_uses_the_given_or_stored_key_and_needs_no_consent() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud]), true, false);
    let mut p = provider();
    p.stt.consent_at_ms = 0;
    let h = k.router.probe_cloud(&p, None);
    assert!(!h.ok);
    assert_eq!(h.error.as_deref(), Some("cloud_not_configured"));
    assert_eq!(h.engine, EngineKind::Cloud);
    let h = k
        .router
        .probe_cloud(&p, Some(SecretString::new("sk-form-key")));
    assert!(h.ok, "{h:?}");
    assert_eq!(h.provider.as_deref(), Some("openrouter"));
    assert_eq!(h.transcript.as_deref(), Some("and so my fellow americans"));
    assert_eq!(*k.keys_seen.lock().unwrap(), ["sk-form-key"]);
    k.router
        .set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    assert!(k.router.probe_cloud(&p, None).ok);
    assert_eq!(k.keys_seen.lock().unwrap().last().unwrap(), KEY);
    // Invalid form: no network.
    p.stt.model = "Not A Model".into();
    let h = k.router.probe_cloud(&p, None);
    assert!(!h.ok && h.message.unwrap().contains("model"));
    assert_eq!(k.cloud.probes.load(Ordering::SeqCst), 2);
}

#[test]
fn probe_cloud_without_cloud_support() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud]), false, true);
    let h = k.router.probe_cloud(&provider(), None);
    assert!(!h.ok);
    assert_eq!(h.error.as_deref(), Some("cloud_not_configured"));
}

#[test]
fn cloud_capabilities_and_unchanged_capabilities_json() {
    let k = rig(cloud_cfg(&[Route::Cloud]));
    let cc = k.router.cloud_capabilities();
    assert!(cc.cloud);
    assert_eq!(cc.adapters, ["openrouter"]);
    let j = serde_json::to_value(k.router.capabilities()).unwrap();
    assert!(j.get("cloud").is_none(), "Capabilities JSON unchanged: {j}");
}

#[test]
fn key_appears_in_no_router_output() {
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.cloud.monthly_budget_usd = Some(Usd(5.0));
    let k = rig(c.clone());
    k.cloud.fail_commit(Some(Failure::AuthFailed));
    k.router.start_session(SessionConfig::default()).unwrap();
    let r1 = dictate(&k.router);
    k.router.start_session(SessionConfig::default()).unwrap();
    let r2 = dictate(&k.router);
    let outputs = [
        serde_json::to_string(&k.router.config()).unwrap(),
        format!("{:?}", k.router.config()),
        serde_json::to_string(&r1).unwrap(),
        serde_json::to_string(&r2).unwrap(),
        serde_json::to_string(&k.router.route_status()).unwrap(),
        serde_json::to_string(&k.router.cloud_usage()).unwrap(),
        serde_json::to_string(&k.router.probe_cloud(&provider(), None)).unwrap(),
        format!("{:?}", SecretString::new(KEY)),
    ];
    for o in outputs {
        assert!(!o.contains(KEY), "key leaked: {o}");
        assert!(!o.contains("LEAKCHECK"), "key leaked: {o}");
    }
}

#[test]
fn default_config_json_has_no_cloud_fields() {
    let j = serde_json::to_value(RouterConfig::default()).unwrap();
    for f in ["allow_cloud", "order", "cloud"] {
        assert!(j.get(f).is_none(), "{f} must be omitted while default");
    }
    // Old JSON (no cloud fields) parses with cloud off.
    let c: RouterConfig = serde_json::from_value(j).unwrap();
    assert!(!c.allow_cloud && c.order.is_empty());
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn set_local_rebuilds_the_cloud_backend() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert_eq!(k.cloud_calls.load(Ordering::SeqCst), 1, "cached");
    k.router
        .set_local(Some(k.local.clone() as Arc<dyn Backend>));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert_eq!(
        k.cloud_calls.load(Ordering::SeqCst),
        2,
        "rebuilt with the new cleaner"
    );
}

// ---- live session route (start info + poll) ---------------------------------

#[test]
fn start_info_and_poll_report_the_cloud_provider() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    let start = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap()
        .route
        .expect("start route");
    assert_eq!(start.engine, EngineKind::Cloud);
    assert_eq!(start.provider.as_deref(), Some("openrouter"));
    assert_eq!(
        start.model.as_deref(),
        Some("openai/whisper-large-v3-turbo")
    );
    assert_eq!((start.server.clone(), start.fallback.clone()), (None, None));
    assert_eq!(k.router.poll().route.as_ref(), Some(&start));
    let json = serde_json::to_value(&start).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"engine":"cloud","provider":"openrouter","model":"openai/whisper-large-v3-turbo"})
    );
    let r = dictate(&k.router);
    assert_eq!(SessionRoute::from_route_info(&route(&r)), start);
}

#[test]
fn cloud_skipped_at_start_names_the_next_route_and_why() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud, Route::Client]), true, false);
    let start = k
        .router
        .start_session_info(SessionConfig::default())
        .unwrap()
        .route
        .unwrap();
    assert_eq!(start.engine, EngineKind::Local);
    assert_eq!(start.fallback.as_deref(), Some("cloud_not_configured"));
    assert_eq!((start.provider.clone(), start.model.clone()), (None, None));
    assert_eq!(
        SessionRoute::from_route_info(&route(&dictate(&k.router))),
        start
    );
}

#[test]
fn poll_follows_a_cloud_to_server_switch() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Server, Route::Client]));
    *k.cloud.die_after.lock().unwrap() = Some(8000);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(k.router.poll().route.unwrap().engine, EngineKind::Cloud);
    for _ in 0..2 {
        assert!(k.router.push_audio(&[100i16; 4000]));
    }
    let now = k.router.poll().route.unwrap();
    assert_eq!(now.engine, EngineKind::Remote);
    assert_eq!(now.server.as_deref(), Some("100.64.0.10:7313"));
    assert_eq!((now.provider.clone(), now.model.clone()), (None, None));
    assert_eq!(now.fallback.as_deref(), Some("disconnected"));
    assert_eq!(
        SessionRoute::from_route_info(&route(&dictate(&k.router))),
        now
    );
}

// ---- S1 / S4: a stored key only ever goes to the host + adapter it was saved for

fn keys_seen(k: &Rig) -> Vec<String> {
    k.keys_seen.lock().unwrap().clone()
}

// ---- D28: one key, bound to openrouter.ai ----------------------------------

#[test]
fn keys_for_other_provider_ids_are_ignored() {
    let k = rig_opts(cloud_cfg(&[Route::Cloud, Route::Client]), true, false);
    k.router
        .set_cloud_key("elevenlabs-1", Some(SecretString::new(KEY)));
    assert!(!k.router.has_cloud_key("elevenlabs-1"));
    assert!(!k.router.has_cloud_key("openrouter"));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "local");
    assert_cloud_unused(&k);
}

#[test]
fn the_legacy_openrouter_id_is_an_alias_after_migration() {
    // A shell that has not migrated its key store yet still sets the key
    // under the old entry id; the router maps it to `openrouter`.
    let legacy: RouterConfig = serde_json::from_value(serde_json::json!({
        "allow_cloud": true, "order": ["cloud", "client"],
        "cloud": {"active": "or-old", "providers": [{
            "id": "or-old", "preset": "openrouter", "adapter": "openrouter",
            "base_url": "https://openrouter.ai/api/v1", "model": "openai/whisper-large-v3",
            "consent_host": "openrouter.ai", "consent_at_ms": 1}]}
    }))
    .unwrap();
    let k = rig_opts(legacy, true, false);
    assert_eq!(k.router.config().cloud.schema, 2);
    k.router
        .set_cloud_key("or-old", Some(SecretString::new(KEY)));
    assert!(k.router.has_cloud_key("openrouter"));
    k.router.start_session(SessionConfig::default()).unwrap();
    let rt = route(&dictate(&k.router));
    assert_eq!(rt.engine, "cloud");
    assert_eq!(
        rt.model.as_deref(),
        Some("openai/whisper-large-v3-turbo"),
        "fake reports it"
    );
    assert_eq!(keys_seen(&k), [KEY]);
}

#[test]
fn unrelated_config_changes_keep_the_key_and_the_backend() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.allow_server = false;
    c.cloud.openrouter.cleanup.timeout_ms = 3000;
    k.router.update_config(c);
    assert!(k.router.has_cloud_key("openrouter"));
    k.router.start_session(SessionConfig::default()).unwrap();
    dictate(&k.router);
    assert_eq!(
        k.cloud_calls.load(Ordering::SeqCst),
        1,
        "cleanup edits keep the STT backend"
    );
}

// ---- S5: toggles, consent and keys are re-checked at every hop -----------------

/// Server primary that dies after 0.5 s, with cloud then local as fallbacks;
/// `change` runs mid-session, before the death.
fn server_dies_after(change: impl Fn(&Rig)) -> (Rig, CommitResult) {
    let k = rig(cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]));
    *k.remote.die_after.lock().unwrap() = Some(8000);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert!(k.router.push_audio(&[100i16; 4000]));
    change(&k);
    let r = dictate(&k.router);
    (k, r)
}

fn assert_went_local(k: &Rig, r: &CommitResult) {
    assert_eq!(route(r).engine, "local", "{r:?}");
    assert_eq!(r.final_text, "local 20000", "full replay into local");
    assert_eq!(k.cloud.starts(), 0, "buffered audio replayed to the cloud");
}

#[test]
fn allow_cloud_off_mid_session_skips_the_cloud_hop() {
    let (k, r) = server_dies_after(|k| {
        let mut c = cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]);
        c.allow_cloud = false;
        k.router.update_config(c);
    });
    assert_went_local(&k, &r);
}

#[test]
fn cloud_removed_from_order_mid_session_skips_the_cloud_hop() {
    let (k, r) = server_dies_after(|k| {
        k.router
            .update_config(cloud_cfg(&[Route::Server, Route::Client]));
    });
    assert_went_local(&k, &r);
}

#[test]
fn key_deleted_mid_session_skips_the_cloud_hop() {
    let (k, r) = server_dies_after(|k| k.router.set_cloud_key("openrouter", None));
    assert_went_local(&k, &r);
}

#[test]
fn consent_revoked_mid_session_skips_the_cloud_hop() {
    let (k, r) = server_dies_after(|k| {
        let mut c = cloud_cfg(&[Route::Server, Route::Cloud, Route::Client]);
        c.cloud.openrouter.stt.consent_at_ms = 0;
        k.router.update_config(c);
    });
    assert_went_local(&k, &r);
}

#[test]
fn allow_server_off_mid_session_skips_the_server_hop() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Server, Route::Client]));
    *k.cloud.die_after.lock().unwrap() = Some(8000);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert!(k.router.push_audio(&[100i16; 4000]));
    let mut c = cloud_cfg(&[Route::Cloud, Route::Server, Route::Client]);
    c.allow_server = false;
    k.router.update_config(c);
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(k.remote.starts(), 0);
}

#[test]
fn turning_cloud_off_mid_session_stops_the_cloud_engine_now() {
    let k = rig(cloud_cfg(&[Route::Cloud, Route::Client]));
    k.router.start_session(SessionConfig::default()).unwrap();
    assert!(k.router.push_audio(&[100i16; 4000]));
    let mut c = cloud_cfg(&[Route::Cloud, Route::Client]);
    c.allow_cloud = false;
    k.router.update_config(c);
    let live = k.router.poll().route.expect("still live");
    assert_eq!(live.engine, EngineKind::Local, "pill switches right away");
    let r = dictate(&k.router);
    assert_eq!(route(&r).engine, "local");
    assert_eq!(r.final_text, "local 20000", "nothing lost");
    assert_eq!(k.cloud.commits.load(Ordering::SeqCst), 0, "no upload");
}

#[test]
fn unusable_cloud_does_not_make_an_open_server_circuit_skip_the_server() {
    // C10: [server, cloud] with no cloud key: the cloud is no fallback, so
    // an open server circuit is retried (half-open) rather than skipped.
    let k = rig_opts(cloud_cfg(&[Route::Server, Route::Cloud]), true, false);
    k.remote.fail_start(Some(Failure::ConnectFailed));
    assert!(k.router.start_session(SessionConfig::default()).is_err());
    k.router.cancel();
    assert!(k.router.route_status().circuit.open);
    k.remote.fail_start(None);
    k.router.start_session(SessionConfig::default()).unwrap();
    assert_eq!(route(&dictate(&k.router)).engine, "remote");
    assert_cloud_unused(&k);
}
