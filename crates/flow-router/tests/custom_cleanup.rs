//! D52: the `cloud` cleanup route with `cloud.cleanup_provider = custom`
//! calls the user's own OpenAI-compatible server (fake cleaner injected as
//! the custom factory); OpenRouter stays the default and is unchanged.
//! Consent binds to the typed host, the key is optional, STT is untouched
//! and the budget keeps working when the server reports no cost.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::{AbortFlag, MockAsr};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::FlowCore;
use flow_router::*;

const OR_KEY: &str = "sk-or-LEAKCHECK-d52-0123456789";
const CUSTOM_KEY: &str = "sk-litellm-LEAKCHECK-d52";
const IN: &str = "the review is on friday right";
const OR_OUT: &str = "The review is on Friday, right?";
const CUSTOM_OUT: &str = "The review is on Friday, right.";
const LOCAL_OUT: &str = "The review is on Friday right?";

struct Local;
impl Cleaner for Local {
    fn clean(&self, _p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        Ok(LOCAL_OUT.into())
    }
}

/// A fake cleaner that answers `out`, reporting `cost` (None = unknown).
struct Fake {
    out: &'static str,
    cost: Option<f64>,
    calls: AtomicUsize,
}

impl Fake {
    fn new(out: &'static str, cost: Option<f64>) -> Arc<Self> {
        Arc::new(Self {
            out,
            cost,
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
        "cloud"
    }
    fn clean_metered(
        &self,
        _p: &CleanPrompt,
        _d: Instant,
        _c: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (
            Ok(self.out.into()),
            CleanUsage {
                cost_usd: self.cost.map(Usd),
                ..Default::default()
            },
        )
    }
}

fn custom_cfg() -> RouterConfig {
    let mut c = RouterConfig {
        allow_server: false,
        allow_cloud_cleanup: true,
        cleanup_order: vec![CleanupRoute::Cloud, CleanupRoute::Local],
        ..Default::default()
    };
    c.cloud.cleanup_provider = CleanupProvider::Custom;
    c.cloud.custom_cleanup = CustomCleanupConfig {
        base_url: "http://10.0.0.5:4000".into(),
        model: "Qwen/Qwen2.5-7B-Instruct".into(),
        consent_host: "10.0.0.5".into(),
        consent_at_ms: 1_792_540_900_000,
        ..Default::default()
    };
    c.validate().unwrap();
    c
}

/// (base_url, key) each custom build saw.
type Builds = Arc<Mutex<Vec<(String, Option<String>)>>>;

struct Rig {
    router: Router,
    or: Arc<Fake>,
    custom: Arc<Fake>,
    custom_built: Builds,
}

fn rig(c: RouterConfig, custom_cost: Option<f64>) -> Rig {
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(IN)),
        Some(Arc::new(Local) as Arc<dyn Cleaner>),
    );
    let or = Fake::new(OR_OUT, Some(0.0007));
    let custom = Fake::new(CUSTOM_OUT, custom_cost);
    let built = Arc::new(Mutex::new(Vec::new()));
    let (o2, c2, b2) = (or.clone(), custom.clone(), built.clone());
    let router = Router::new(c, Some(Arc::new(core) as Arc<dyn Backend>), None)
        .with_cleanup_factory(Box::new(move |_o, _k| o2.clone() as Arc<dyn Cleaner>))
        .with_custom_cleanup_factory(Box::new(move |cc, _scope, k| {
            b2.lock()
                .unwrap()
                .push((cc.base_url.clone(), k.map(|k| k.expose().to_string())));
            c2.clone() as Arc<dyn Cleaner>
        }));
    router.set_cloud_key("openrouter", Some(SecretString::new(OR_KEY)));
    Rig {
        router,
        or,
        custom,
        custom_built: built,
    }
}

fn dictate(r: &Router) -> CommitResult {
    r.start_session(SessionConfig {
        llm_skip_clean: false,
        ..Default::default()
    })
    .unwrap();
    let pcm: Vec<i16> = (0..300 * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    assert!(r.push_audio(&pcm));
    r.commit()
}

#[test]
fn custom_provider_cleans_the_cloud_route_without_a_key() {
    let k = rig(custom_cfg(), None);
    let r = dictate(&k.router);
    assert!(r.final_text.contains(CUSTOM_OUT), "{r:?}");
    assert_eq!((k.custom.calls(), k.or.calls()), (1, 0));
    let rt = r.route.unwrap();
    assert_eq!(rt.llm_engine.as_deref(), Some("cloud"));
    assert_eq!(rt.llm_model.as_deref(), Some("Qwen/Qwen2.5-7B-Instruct"));
    // Built once, keyless: the OpenRouter key never reaches it.
    assert_eq!(
        *k.custom_built.lock().unwrap(),
        [("http://10.0.0.5:4000".to_string(), None)]
    );
    let st = k.router.cleanup_status();
    assert_eq!(st.provider, CleanupProvider::Custom);
    assert_eq!(st.host, "10.0.0.5");
    assert!(st.supported && st.enabled && st.consent && st.blocked.is_none());
    assert_eq!(st.model, "Qwen/Qwen2.5-7B-Instruct");
    assert_eq!(st.timeout_ms, 4000);
    // Counted under its own id, cost unknown.
    let u = k.router.cloud_usage();
    let p = u
        .providers
        .iter()
        .find(|p| p.provider_id == "custom")
        .unwrap();
    assert_eq!((p.purpose.as_str(), p.requests), ("cleanup", 1));
    assert!(!k.router.has_cloud_key("custom"));
}

#[test]
fn custom_key_is_its_own_and_rebuilds_the_cleaner() {
    let k = rig(custom_cfg(), None);
    k.router
        .set_cloud_key("custom", Some(SecretString::new(CUSTOM_KEY)));
    assert!(k.router.has_cloud_key("custom") && k.router.has_cloud_key("openrouter"));
    dictate(&k.router);
    let built = k.custom_built.lock().unwrap().clone();
    assert_eq!(built.last().unwrap().1.as_deref(), Some(CUSTOM_KEY));
    // The custom key does not replace the OpenRouter key (STT keeps it).
    k.router
        .with_cloud_key(|key| assert_eq!(key.unwrap().expose(), OR_KEY));
    k.router.set_cloud_key("custom", None);
    assert!(!k.router.has_cloud_key("custom"));
}

#[test]
fn openrouter_stays_the_default_and_unchanged() {
    let mut c = custom_cfg();
    c.cloud.cleanup_provider = CleanupProvider::OpenRouter;
    c.cloud.openrouter.cleanup.consent_host = "openrouter.ai".into();
    c.cloud.openrouter.cleanup.consent_at_ms = 1;
    let k = rig(c, None);
    let r = dictate(&k.router);
    assert!(r.final_text.contains(OR_OUT), "{r:?}");
    assert_eq!((k.or.calls(), k.custom.calls()), (1, 0));
    assert!(k.custom_built.lock().unwrap().is_empty());
    let st = k.router.cleanup_status();
    assert_eq!(st.provider, CleanupProvider::OpenRouter);
    assert!(st.host.is_empty());
    // Switching the provider back and forth takes effect at once.
    let mut c = k.router.config();
    c.cloud.cleanup_provider = CleanupProvider::Custom;
    k.router.update_config(c);
    assert!(dictate(&k.router).final_text.contains(CUSTOM_OUT));
}

#[test]
fn custom_consent_is_bound_to_the_typed_host() {
    // OpenRouter consent does not grant the custom server.
    let mut c = custom_cfg();
    c.cloud.custom_cleanup.consent_at_ms = 0;
    c.cloud.openrouter.cleanup.consent_host = "openrouter.ai".into();
    c.cloud.openrouter.cleanup.consent_at_ms = 1;
    let k = rig(c, None);
    let r = dictate(&k.router);
    assert!(r.final_text.contains(LOCAL_OUT), "{r:?}");
    assert_eq!((k.custom.calls(), k.or.calls()), (0, 0));
    assert_eq!(
        k.router.cleanup_status().blocked.as_deref(),
        Some("cloud_cleanup_not_configured")
    );
    // Pointing the URL at another host voids the consent.
    let mut c = custom_cfg();
    c.cloud.custom_cleanup.base_url = "http://10.0.0.99:4000/v1".into();
    let k = rig(c, None);
    assert!(dictate(&k.router).final_text.contains(LOCAL_OUT));
    assert_eq!(k.custom.calls(), 0);
}

#[test]
fn custom_needs_its_factory() {
    let core = FlowCore::from_engines(
        Arc::new(MockAsr::new(IN)),
        Some(Arc::new(Local) as Arc<dyn Cleaner>),
    );
    let or = Fake::new(OR_OUT, Some(0.0007));
    let o2 = or.clone();
    let router = Router::new(custom_cfg(), Some(Arc::new(core) as Arc<dyn Backend>), None)
        .with_cleanup_factory(Box::new(move |_o, _k| o2.clone() as Arc<dyn Cleaner>));
    router.set_cloud_key("openrouter", Some(SecretString::new(OR_KEY)));
    assert!(dictate(&router).final_text.contains(LOCAL_OUT));
    assert_eq!(
        or.calls(),
        0,
        "custom selected: OpenRouter is not a stand-in"
    );
    assert!(!router.cleanup_status().supported);
}

#[test]
fn budget_with_unknown_custom_cost_does_not_block() {
    let mut c = custom_cfg();
    c.cloud.monthly_budget_usd = Some(Usd(1.0));
    let k = rig(c, None);
    for _ in 0..3 {
        assert!(dictate(&k.router).final_text.contains(CUSTOM_OUT));
    }
    assert_eq!(k.custom.calls(), 3);
    // A reported cost counts; once the budget is spent, cleanup stops.
    let mut c = custom_cfg();
    c.cloud.monthly_budget_usd = Some(Usd(0.001));
    let k = rig(c, Some(0.002));
    assert!(dictate(&k.router).final_text.contains(CUSTOM_OUT));
    assert!(dictate(&k.router).final_text.contains(LOCAL_OUT));
    assert_eq!(k.custom.calls(), 1);
}

#[test]
fn stt_config_is_untouched_by_the_custom_provider() {
    let k = rig(custom_cfg(), None);
    let c = k.router.config();
    assert!(!c.allow_cloud);
    assert_eq!(c.cloud.openrouter.stt, SttConfig::default());
    // The custom key is not the cloud STT key.
    k.router.set_cloud_key("openrouter", None);
    k.router
        .set_cloud_key("custom", Some(SecretString::new(CUSTOM_KEY)));
    assert!(!k.router.has_cloud_key("openrouter"));
    k.router.with_cloud_key(|key| assert!(key.is_none()));
}
