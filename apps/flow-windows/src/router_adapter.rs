//! Hybrid local/server dictation (D17): the shell's [`CoreApi`] served by
//! `flow_router::Router` in every build (invariant I1).
//!
//! * [`LocalBackend`] wraps the existing [`FlowCoreAdapter`] (its two-phase
//!   model/LLM load and "core of the open session" rule stay as they were)
//!   as the router's `Local` backend.
//! * The remote backend is `flow_remote::factory()` in builds with the
//!   `remote` feature; client-only builds pass `None`, so they contain no
//!   network code (I3). Cargo features only decide which side is `Some`.
//! * [`RouterAdapter`] maps shell types onto flow-core's (the same mapping
//!   as the local path, so a remote session sees the identical
//!   `SessionConfig`/`FieldContext`, I2), turns router errors into pill
//!   text, and remembers the last dictation's route for the Developer page
//!   and the optional S/L badge.
//! * Cloud STT (D18, feature `cloud`): `flow_cloud::attach_with` adds the
//!   opt-in third-party route; it shares the local LLM cleaner (rebuilt via
//!   [`RouterAdapter::refresh_local`] after the two-phase load). Keys come
//!   from Credential Manager through `Router::set_cloud_key` only. Cloud
//!   results carry provider/model/cost into [`LastDictation`] and the `C`
//!   badge, and cloud failures get user-facing text (`crate::cloud`).

use crate::core_adapter::{
    from_core_result, to_core_context, to_core_session, FlowCoreAdapter, LOAD_WAIT,
};
use crate::core_api::{
    CommitResult, CommitStatus, CoreApi, FieldContext, LearnCandidate, SessionConfig,
};
use crate::dev_settings::{BuildCaps, DevSettings, NO_ENGINE_MSG};
use flow_router::{
    Backend, BackendError, CloudCapabilities, CloudUsage, EngineKind, Failure, HealthReport,
    RemoteFactory, RouteStatus, Router, RouterConfig, RouterError, ServerConfig, StartInfo,
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ local

/// The router's `Local` backend: the shell's model slot.
pub struct LocalBackend {
    adapter: Arc<FlowCoreAdapter>,
}

impl LocalBackend {
    pub fn new(adapter: Arc<FlowCoreAdapter>) -> Arc<Self> {
        Arc::new(LocalBackend { adapter })
    }
}

impl Backend for LocalBackend {
    fn kind(&self) -> EngineKind {
        EngineKind::Local
    }

    fn ready(&self) -> bool {
        self.adapter.is_ready()
    }

    fn start_session(
        &self,
        cfg: &flow_core::config::SessionConfig,
    ) -> Result<StartInfo, BackendError> {
        self.adapter
            .core_start(cfg.clone())
            .map(|_| StartInfo::default())
            .map_err(|m| BackendError::new(Failure::Error, m))
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.adapter.core_push(pcm)
    }

    fn set_context(&self, ctx: &flow_core::config::FieldContext) {
        self.adapter.core_set_context(ctx.clone())
    }

    fn partial(&self) -> Option<String> {
        self.adapter.core_partial()
    }

    fn commit(&self, _deadline: Instant) -> Result<flow_core::CommitResult, BackendError> {
        self.adapter
            .core_commit()
            .ok_or_else(|| BackendError::new(Failure::Error, "no active session"))
    }

    fn cancel(&self) {
        CoreApi::cancel(&*self.adapter)
    }

    fn probe(&self) -> HealthReport {
        let status = self.adapter.status();
        HealthReport {
            ok: status.is_ready(),
            engine: EngineKind::Local,
            message: status.problem(),
            postprocess_rev: Some(flow_core::POSTPROCESS_REV.into()),
            postprocess_match: Some(true),
            checked_at_ms: now_ms(),
            ..HealthReport::default()
        }
    }
}

// ------------------------------------------------------------------ views

/// "Active engine (last dictation)" row: route + timings, in memory only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LastDictation {
    /// "local" | "remote" | "cloud"; `None` when no engine ran.
    pub engine: Option<String>,
    pub fallback: Option<String>,
    pub server: Option<String>,
    pub rtt_ms: Option<u32>,
    pub queue_ms: Option<u32>,
    pub server_core: Option<String>,
    pub asr_ms: u64,
    pub total_ms: u64,
    pub audio_ms: u64,
    pub dropped_samples: u64,
    pub status: CommitStatus,
    pub empty_reason: Option<String>,
    pub error: Option<String>,
    pub at_ms: u64,
    /// Cloud (D18): provider id and its label, model, billed audio, cost.
    pub provider: Option<String>,
    pub provider_label: Option<String>,
    pub model: Option<String>,
    pub billed_audio_ms: Option<u32>,
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
    /// `cloud · <provider> · <model> · $cost` for a cloud result.
    pub row: Option<String>,
    /// User-facing cloud failure ("Cloud STT failed (key rejected); used
    /// this PC") when the cloud was meant to run but did not.
    pub notice: Option<String>,
}

impl LastDictation {
    fn of(r: &CommitResult, cfg: &RouterConfig) -> Self {
        let route = r.route.as_ref();
        let provider = route.and_then(|x| x.provider.clone());
        let provider_label = provider.as_ref().map(|id| {
            cfg.cloud
                .providers
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.display_name().to_string())
                .unwrap_or_else(|| id.clone())
        });
        let model = route.and_then(|x| x.model.clone());
        let cost_usd = route.and_then(|x| x.cost_usd).map(|c| c.0);
        let cost_estimated = route.and_then(|x| x.cost_estimated).unwrap_or(false);
        let is_cloud = route.is_some_and(|x| x.engine == "cloud");
        let row = is_cloud.then(|| {
            crate::cloud::engine_row(
                provider_label.as_deref().unwrap_or("?"),
                model.as_deref().unwrap_or(""),
                cost_usd,
                cost_estimated,
            )
        });
        LastDictation {
            provider,
            provider_label,
            model,
            billed_audio_ms: route.and_then(|x| x.billed_audio_ms),
            cost_usd,
            cost_estimated,
            row,
            notice: cloud_notice(r, cfg),
            engine: route.map(|x| x.engine.clone()),
            fallback: route.and_then(|x| x.fallback.clone()),
            server: route.and_then(|x| x.server.clone()),
            rtt_ms: route.and_then(|x| x.rtt_ms),
            queue_ms: route.and_then(|x| x.queue_ms),
            server_core: route.and_then(|x| x.server_core.clone()),
            asr_ms: r.timings.asr_ms,
            total_ms: r.timings.total_ms,
            audio_ms: r.timings.audio_ms,
            dropped_samples: r.timings.dropped_samples,
            status: r.status,
            empty_reason: r.empty_reason.clone(),
            error: r.error.clone(),
            at_ms: now_ms(),
        }
    }

    /// Overlay badge: `S` / `L` / `C`, fallback = red dot. A cloud failure
    /// notice rides along (shown above the pill).
    pub fn badge(&self) -> Option<Badge> {
        let letter = match self.engine.as_deref()? {
            "remote" => "S",
            "cloud" => "C",
            _ => "L",
        };
        Some(Badge {
            letter,
            fallback: self.fallback.is_some(),
            title: self
                .provider_label
                .as_ref()
                .filter(|_| letter == "C")
                .map(|p| format!("Sent to {p}")),
            message: self.notice.clone(),
        })
    }

    /// The badge must show even without "show engine badge": every cloud
    /// dictation, and every cloud failure, is visible (C7).
    pub fn always_badge(&self) -> bool {
        self.engine.as_deref() == Some("cloud") || self.notice.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Badge {
    pub letter: &'static str,
    pub fallback: bool,
    /// Tooltip ("Sent to OpenRouter").
    pub title: Option<String>,
    /// Cloud failure text for the bubble above the pill.
    pub message: Option<String>,
}

/// The failure code of a cloud attempt that did not produce the result.
fn cloud_failure_code(r: &CommitResult, cfg: &RouterConfig) -> Option<String> {
    let route = r.route.as_ref()?;
    let fb = route.fallback.clone()?;
    let cloud_only_code = matches!(
        fb.as_str(),
        "cloud_not_configured" | "budget_exceeded" | "quota_exceeded" | "provider_error"
    );
    // The cloud was the first route the toggles allow (a skipped cloud
    // route reports e.g. `auth_failed` like a server would).
    let cloud_was_first =
        cfg.order.iter().find(|r| cfg.route_allowed(**r)) == Some(&flow_router::Route::Cloud);
    (route.engine != "cloud" && (cloud_only_code || cloud_was_first)).then_some(fb)
}

fn cloud_notice(r: &CommitResult, cfg: &RouterConfig) -> Option<String> {
    let code = cloud_failure_code(r, cfg)?;
    let used = (r.status != CommitStatus::Error)
        .then(|| r.route.as_ref().map(|x| x.engine.as_str()))
        .flatten();
    Some(crate::cloud::failure_message(&code, used))
}

/// `dev_route_status`: the router's view plus the shell's last dictation.
#[derive(Debug, Clone, Serialize)]
pub struct RouteView {
    pub last: Option<LastDictation>,
    pub circuit: flow_router::CircuitStatus,
    pub health: Option<HealthReport>,
    /// Cloud route circuit (builds with the cloud client only).
    pub cloud_circuit: Option<flow_router::CircuitStatus>,
}

/// Result of the Developer page's "Test session" (1 s of silence).
#[derive(Debug, Clone, Serialize)]
pub struct SessionTest {
    pub status: CommitStatus,
    pub empty_reason: Option<String>,
    pub error: Option<String>,
    pub engine: Option<String>,
    pub fallback: Option<String>,
    pub total_ms: u64,
}

// ------------------------------------------------------------------ config

/// Developer settings → `RouterConfig` (via the §1.2 JSON, which the
/// router fixtures pin), validated by the router itself.
pub fn router_config(dev: &DevSettings, caps: BuildCaps) -> Result<RouterConfig, String> {
    let json = dev.router_config_json(caps, &dev.token());
    let cfg: RouterConfig = serde_json::from_value(json).map_err(|e| e.to_string())?;
    cfg.validate().map_err(|e| e.to_string())?;
    Ok(cfg)
}

/// Same, for the unsaved form (Test connection) with the token in clear.
pub fn server_config(dev: &DevSettings, token: &str) -> Result<ServerConfig, String> {
    let json = dev.router_config_json(BuildCaps::current(), token);
    let cfg: RouterConfig = serde_json::from_value(json).map_err(|e| e.to_string())?;
    cfg.validate().map_err(|e| e.to_string())?;
    Ok(cfg.server)
}

/// The remote side of this build: `None` without the `remote` feature.
pub fn remote_factory() -> Option<RemoteFactory> {
    #[cfg(feature = "remote")]
    {
        Some(flow_remote::factory(concat!(
            "flow-windows ",
            env!("CARGO_PKG_VERSION")
        )))
    }
    #[cfg(not(feature = "remote"))]
    {
        None
    }
}

// ------------------------------------------------------------------ adapter

type RouteListener = Box<dyn Fn(&LastDictation) + Send + Sync>;

pub struct RouterAdapter {
    router: Router,
    local: Option<Arc<FlowCoreAdapter>>,
    last: Mutex<Option<LastDictation>>,
    listener: Mutex<Option<RouteListener>>,
}

impl RouterAdapter {
    /// `local`: the model slot when this build has an on-device engine
    /// (its models may still be loading). `remote`: [`remote_factory`].
    pub fn new(
        cfg: RouterConfig,
        local: Option<Arc<FlowCoreAdapter>>,
        remote: Option<RemoteFactory>,
    ) -> Arc<Self> {
        let backend = local
            .clone()
            .map(|a| LocalBackend::new(a) as Arc<dyn Backend>);
        let engines = local_engines();
        let router = Router::new(cfg, backend, remote)
            .with_local_capability(local.is_some())
            .with_local_info(
                if local.is_some() { engines } else { Vec::new() },
                local.is_some(),
            );
        // D18: the cloud route, sharing the local LLM cleaner (looked up per
        // backend build; `refresh_local` drops the cached backend).
        #[cfg(feature = "cloud")]
        let router = {
            let slot = local.clone();
            flow_cloud::attach_with(router, move || slot.as_ref().and_then(|a| a.cleaner()))
        };
        Arc::new(RouterAdapter {
            router,
            local,
            last: Mutex::new(None),
            listener: Mutex::new(None),
        })
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub fn local(&self) -> Option<&Arc<FlowCoreAdapter>> {
        self.local.as_ref()
    }

    /// The local models (or LLM cleaner) changed: hand the router its local
    /// backend again, which also rebuilds the cloud backend with the
    /// current cleaner (changelog "What W needs").
    pub fn refresh_local(&self) {
        self.router.set_local(
            self.local
                .clone()
                .map(|a| LocalBackend::new(a) as Arc<dyn Backend>),
        );
    }

    pub fn cloud_capabilities(&self) -> CloudCapabilities {
        self.router.cloud_capabilities()
    }

    pub fn cloud_usage(&self) -> CloudUsage {
        self.router.cloud_usage()
    }

    /// Load every provider's key from the credential store into the router
    /// (startup, and after a Save that added providers).
    pub fn load_cloud_keys(
        &self,
        dev: &DevSettings,
        store: &dyn crate::credentials::CredentialStore,
    ) {
        for p in &dev.cloud.providers {
            let key = store.get(&p.id).ok().flatten();
            self.router
                .set_cloud_key(&p.id, key.map(flow_router::SecretString::new));
        }
    }

    /// Called after every commit (overlay badge, Developer page refresh).
    pub fn on_route(&self, f: impl Fn(&LastDictation) + Send + Sync + 'static) {
        *self.listener.lock().unwrap() = Some(Box::new(f));
    }

    /// What this router can serve (the engines it was built with).
    pub fn caps(&self) -> BuildCaps {
        let c = self.router.capabilities();
        BuildCaps {
            local: c.local,
            remote: c.remote,
            cloud: self.router.cloud_capabilities().cloud,
        }
    }

    /// Settings → router; applies from the next `start_session` (§5).
    pub fn apply(&self, dev: &DevSettings) -> Result<(), String> {
        self.router.update_config(router_config(dev, self.caps())?);
        Ok(())
    }

    pub fn last(&self) -> Option<LastDictation> {
        self.last.lock().unwrap().clone()
    }

    pub fn route_view(&self) -> RouteView {
        let RouteStatus {
            circuit,
            health,
            cloud_circuit,
            ..
        } = self.router.route_status();
        RouteView {
            last: self.last(),
            circuit,
            health,
            cloud_circuit,
        }
    }

    /// 1 s of silence through the full router path with the saved toggles;
    /// a healthy setup answers EMPTY `no_speech` (§5 "Test session").
    pub fn test_session(&self, cfg: SessionConfig) -> SessionTest {
        let r = match CoreApi::start_session(self, cfg) {
            Ok(()) => {
                for _ in 0..10 {
                    CoreApi::push_audio(self, &[0i16; 1600]);
                }
                CoreApi::commit(self)
            }
            Err(e) => CommitResult::failed(e),
        };
        let route = r.route.as_ref();
        SessionTest {
            status: r.status,
            empty_reason: r.empty_reason.clone(),
            error: r.error.clone(),
            engine: route.map(|x| x.engine.clone()),
            fallback: route.and_then(|x| x.fallback.clone()),
            total_ms: r.timings.total_ms,
        }
    }

    /// Will this session start on the local engine (so a model load in
    /// progress is worth waiting for)?
    fn local_first(&self) -> bool {
        let cfg = self.router.config();
        let caps = self.router.capabilities();
        let cloud = self.cloud_ready(&cfg);
        cfg.route_order().into_iter().find(|k| match k {
            EngineKind::Local => caps.local,
            EngineKind::Remote => caps.remote,
            EngineKind::Cloud => cloud,
            _ => false,
        }) == Some(EngineKind::Local)
    }

    /// The cloud route would start now (no network: config, consent, key).
    fn cloud_ready(&self, cfg: &RouterConfig) -> bool {
        self.router.cloud_capabilities().cloud
            && cfg.allow_cloud
            && cfg
                .cloud
                .active_provider()
                .is_some_and(|p| p.consent_ok() && self.router.has_cloud_key(&p.id))
    }

    /// Only the cloud route is allowed (start errors then name the cloud).
    fn cloud_only(&self, cfg: &RouterConfig) -> bool {
        let caps = self.router.capabilities();
        self.router.cloud_capabilities().cloud
            && cfg.allow_cloud
            && cfg.order.contains(&flow_router::Route::Cloud)
            && !(caps.remote && cfg.allow_server && cfg.order.contains(&flow_router::Route::Server))
            && !(caps.local && cfg.allow_client && cfg.order.contains(&flow_router::Route::Client))
    }

    /// Why the cloud route cannot start (budget, else setup).
    fn cloud_skip_reason(&self, cfg: &RouterConfig) -> &'static str {
        let spent = self.router.cloud_usage().total_cost();
        if cfg.cloud.monthly_budget_usd.is_some_and(|b| spent >= b.0) {
            "budget_exceeded"
        } else {
            "cloud_not_configured"
        }
    }

    fn start_error(&self, e: RouterError) -> String {
        let cfg = self.router.config();
        match e {
            RouterError::NoEngineEnabled if self.cloud_only(&cfg) => {
                crate::cloud::failure_message(self.cloud_skip_reason(&cfg), None)
            }
            RouterError::ServerUnavailable(why) if self.cloud_only(&cfg) => {
                let code = why.split(':').next().unwrap_or("").trim().to_string();
                if crate::cloud::failure_text(&code).is_some() {
                    crate::cloud::failure_message(&code, None)
                } else {
                    format!("Cloud STT failed ({why})")
                }
            }
            RouterError::NoEngineEnabled => NO_ENGINE_MSG.into(),
            RouterError::LocalNotReady => self
                .local
                .as_ref()
                .and_then(|l| l.status().problem())
                .unwrap_or_else(|| "Speech model is not loaded".into()),
            RouterError::ServerNotConfigured => {
                "Server unreachable (no server host set in Developer settings)".into()
            }
            RouterError::ServerUnavailable(why) => format!("Server unreachable ({why})"),
            RouterError::BadConfig(m) => format!("Developer settings are invalid: {m}"),
        }
    }
}

/// ASR engines compiled into this build (Capabilities row).
fn local_engines() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(feature = "whisper") {
        v.push("whisper".to_string());
    }
    if cfg!(feature = "parakeet") {
        v.push("parakeet".to_string());
    }
    v
}

impl CoreApi for RouterAdapter {
    fn start_session(&self, cfg: SessionConfig) -> Result<(), String> {
        // Keep the old "dictation during model load waits a moment" rule
        // when the local engine is the one that will run.
        if let Some(l) = &self.local {
            if self.local_first() {
                let _ = l.wait_ready(LOAD_WAIT);
            }
        }
        self.router
            .start_session(to_core_session(&cfg))
            .map_err(|e| self.start_error(e))
    }

    fn push_audio(&self, pcm: &[i16]) {
        self.router.push_audio(pcm);
    }

    fn set_context(&self, ctx: FieldContext) {
        self.router.set_context(to_core_context(&ctx));
    }

    fn partial(&self) -> Option<String> {
        self.router.partial().filter(|p| !p.trim().is_empty())
    }

    fn commit(&self) -> CommitResult {
        let mut r = from_core_result(self.router.commit());
        let cfg = self.router.config();
        // Server-only failure: say so in the pill (§5 "Errors in normal UI").
        if r.status == CommitStatus::Error {
            // A session that never started reports the start error again;
            // give it the same pill text as `start_session` would.
            let detail = r.error.clone().unwrap_or_default();
            let cloud_failed = cloud_failure_code(&r, &cfg).or_else(|| {
                r.route
                    .as_ref()
                    .filter(|x| x.engine == "cloud")
                    .and_then(|x| x.fallback.clone())
            });
            if let Some(code) = cloud_failed.filter(|_| self.cloud_only(&cfg)) {
                r.error = Some(crate::cloud::failure_message(&code, None));
            } else if detail.starts_with("no_engine_enabled") {
                r.error = Some(if self.cloud_only(&cfg) {
                    crate::cloud::failure_message(self.cloud_skip_reason(&cfg), None)
                } else {
                    NO_ENGINE_MSG.into()
                });
            } else if detail.starts_with("local_not_ready") {
                r.error = Some(self.start_error(RouterError::LocalNotReady));
            } else if let Some(fb) = r.route.as_ref().and_then(|x| x.fallback.clone()) {
                let detail = r.error.clone().unwrap_or_default();
                r.error = Some(if detail.is_empty() {
                    format!("Server unreachable ({fb})")
                } else {
                    format!("Server unreachable ({fb}): {detail}")
                });
            }
        }
        if r.route.is_some() || r.error.as_deref() != Some("no active session") {
            let last = LastDictation::of(&r, &cfg);
            *self.last.lock().unwrap() = Some(last.clone());
            if let Some(f) = self.listener.lock().unwrap().as_ref() {
                f(&last);
            }
        }
        r
    }

    fn cancel(&self) {
        self.router.cancel();
    }

    fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<LearnCandidate> {
        self.router
            .learn_edit(pasted, edited)
            .into_iter()
            .map(|c| LearnCandidate {
                word: c.corrected,
                heard: c.original,
                replacement: None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_adapter::ModelSpec;
    use crate::dev_settings::{Route, ServerSettings};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    const BOTH: BuildCaps = BuildCaps {
        local: true,
        remote: true,
        cloud: true,
    };

    /// A "server": a real FlowCore with MockAsr behind `kind() == Remote`,
    /// that can be switched off. Counts connections (factory calls that
    /// reached `start_session`) and received samples.
    struct FakeServer {
        core: flow_core::FlowCore,
        up: AtomicBool,
        starts: AtomicUsize,
        samples: AtomicU64,
    }

    impl FakeServer {
        fn new(text: &str) -> Arc<Self> {
            let core = flow_core::FlowCore::load_models(&flow_core::config::ModelConfig {
                asr_backend: flow_core::config::AsrBackend::Mock { text: text.into() },
                ..Default::default()
            })
            .unwrap();
            Arc::new(FakeServer {
                core,
                up: AtomicBool::new(true),
                starts: AtomicUsize::new(0),
                samples: AtomicU64::new(0),
            })
        }
    }

    struct FakeRemote(Arc<FakeServer>);

    impl Backend for FakeRemote {
        fn kind(&self) -> EngineKind {
            EngineKind::Remote
        }
        fn ready(&self) -> bool {
            self.0.up.load(Ordering::SeqCst)
        }
        fn start_session(
            &self,
            cfg: &flow_core::config::SessionConfig,
        ) -> Result<StartInfo, BackendError> {
            if !self.0.up.load(Ordering::SeqCst) {
                return Err(BackendError::new(Failure::ConnectFailed, "refused"));
            }
            self.0.starts.fetch_add(1, Ordering::SeqCst);
            Backend::start_session(&self.0.core, cfg)
        }
        fn push_audio(&self, pcm: &[i16]) -> bool {
            self.0.samples.fetch_add(pcm.len() as u64, Ordering::SeqCst);
            Backend::push_audio(&self.0.core, pcm)
        }
        fn set_context(&self, ctx: &flow_core::config::FieldContext) {
            Backend::set_context(&self.0.core, ctx)
        }
        fn partial(&self) -> Option<String> {
            Backend::partial(&self.0.core)
        }
        fn commit(&self, d: Instant) -> Result<flow_core::CommitResult, BackendError> {
            if !self.0.up.load(Ordering::SeqCst) {
                return Err(BackendError::new(Failure::Disconnected, "gone"));
            }
            Backend::commit(&self.0.core, d)
        }
        fn cancel(&self) {
            Backend::cancel(&self.0.core)
        }
        fn probe(&self) -> HealthReport {
            HealthReport {
                ok: self.0.up.load(Ordering::SeqCst),
                engine: EngineKind::Remote,
                ..HealthReport::default()
            }
        }
    }

    struct Rig {
        router: Arc<RouterAdapter>,
        server: Arc<FakeServer>,
        factory_calls: Arc<AtomicUsize>,
    }

    fn dev(server: bool, client: bool) -> DevSettings {
        DevSettings {
            allow_server: server,
            allow_client: client,
            server: ServerSettings {
                host: "100.64.0.10".into(),
                ..ServerSettings::default()
            },
            ..DevSettings::for_build(BOTH)
        }
    }

    fn rig(d: &DevSettings) -> Rig {
        let local = FlowCoreAdapter::new();
        assert!(local
            .load_now(&ModelSpec::Mock {
                text: "from this pc".into()
            })
            .is_ready());
        let server = FakeServer::new("from the server");
        let calls = Arc::new(AtomicUsize::new(0));
        let (s2, c2) = (server.clone(), calls.clone());
        let factory: RemoteFactory = Box::new(move |_cfg: &ServerConfig| {
            c2.fetch_add(1, Ordering::SeqCst);
            Arc::new(FakeRemote(s2.clone())) as Arc<dyn Backend>
        });
        let router =
            RouterAdapter::new(router_config(d, BOTH).unwrap(), Some(local), Some(factory));
        Rig {
            router,
            server,
            factory_calls: calls,
        }
    }

    fn session() -> SessionConfig {
        SessionConfig {
            max_samples: 16_000 * 60,
            ..SessionConfig::default()
        }
    }

    fn dictate(r: &RouterAdapter) -> Result<CommitResult, String> {
        r.start_session(session())?;
        for _ in 0..10 {
            r.push_audio(&[3000; 1600]);
        }
        Ok(r.commit())
    }

    #[test]
    fn both_on_server_up_uses_the_server() {
        let r = rig(&dev(true, true));
        let out = dictate(&r.router).unwrap();
        assert_eq!(out.final_text.trim(), "from the server", "{out:?}");
        let route = out.route.unwrap();
        assert_eq!(route.engine, "remote");
        assert_eq!(route.fallback, None);
        let last = r.router.last().unwrap();
        assert_eq!(last.badge().unwrap().letter, "S");
        assert!(!last.badge().unwrap().fallback);
    }

    #[test]
    fn both_on_server_down_falls_back_locally() {
        let r = rig(&dev(true, true));
        r.server.up.store(false, Ordering::SeqCst);
        let out = dictate(&r.router).unwrap();
        assert_eq!(out.final_text.trim(), "from this pc", "{out:?}");
        let route = out.route.clone().unwrap();
        assert_eq!(route.engine, "local");
        assert_eq!(route.fallback.as_deref(), Some("connect_failed"));
        let badge = r.router.last().unwrap().badge().unwrap();
        assert_eq!((badge.letter, badge.fallback), ("L", true));
        assert_eq!(
            badge.message, None,
            "a server fallback is not a cloud notice"
        );
    }

    #[test]
    fn server_dies_mid_session_local_gets_all_audio() {
        let r = rig(&dev(true, true));
        r.router.start_session(session()).unwrap();
        for _ in 0..5 {
            r.router.push_audio(&[3000; 1600]);
        }
        r.server.up.store(false, Ordering::SeqCst);
        for _ in 0..5 {
            r.router.push_audio(&[3000; 1600]);
        }
        let out = r.router.commit();
        assert_eq!(out.final_text.trim(), "from this pc", "{out:?}");
        assert_eq!(out.timings.audio_ms, 1000, "I5: replayed + live = all");
        assert_eq!(out.timings.dropped_samples, 0);
        let route = out.route.unwrap();
        assert_eq!(route.engine, "local");
        assert!(route.fallback.is_some());
    }

    #[test]
    fn server_only_down_is_a_clear_error() {
        let r = rig(&dev(true, false));
        r.server.up.store(false, Ordering::SeqCst);
        let err = match dictate(&r.router) {
            Err(e) => e,
            Ok(res) => res.error.unwrap_or_default(),
        };
        assert!(err.starts_with("Server unreachable"), "{err}");
    }

    #[test]
    fn client_only_never_builds_a_remote() {
        let r = rig(&dev(false, true));
        let out = dictate(&r.router).unwrap();
        assert_eq!(out.final_text.trim(), "from this pc");
        assert_eq!(r.factory_calls.load(Ordering::SeqCst), 0);
        assert_eq!(r.server.starts.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn both_off_is_dictation_disabled() {
        let r = rig(&dev(false, false));
        assert_eq!(dictate(&r.router).unwrap_err(), NO_ENGINE_MSG);
        assert_eq!(r.factory_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn settings_apply_to_the_next_session_without_reload() {
        let r = rig(&dev(true, true));
        assert_eq!(dictate(&r.router).unwrap().route.unwrap().engine, "remote");
        let client_first = DevSettings {
            route_order: vec![Route::Client, Route::Server],
            ..dev(true, true)
        };
        r.router.apply(&client_first).unwrap();
        assert_eq!(dictate(&r.router).unwrap().route.unwrap().engine, "local");
        r.router.apply(&dev(false, false)).unwrap();
        assert!(dictate(&r.router).is_err());
        assert!(
            r.router.local().unwrap().status().is_ready(),
            "never reloaded"
        );
    }

    #[test]
    fn test_session_reports_no_speech_and_route() {
        let r = rig(&dev(true, true));
        let t = r.router.test_session(session());
        assert_eq!(t.status, CommitStatus::Empty, "{t:?}");
        assert_eq!(t.empty_reason.as_deref(), Some("no_speech"));
        assert_eq!(t.engine.as_deref(), Some("remote"));
    }

    #[test]
    fn learn_edit_runs_locally_with_the_last_dictionary() {
        let r = rig(&dev(true, true));
        let cands = r
            .router
            .learn_edit("I use whisper flow", "I use Wispr Flow");
        assert!(cands.iter().any(|c| c.word.contains("Wispr")), "{cands:?}");
    }

    #[test]
    fn local_not_ready_message_names_the_model_problem() {
        let local = FlowCoreAdapter::new();
        local.load_now(&ModelSpec::Whisper {
            path: "/nonexistent/ggml-base.bin".into(),
            threads: None,
        });
        let router = RouterAdapter::new(
            router_config(&dev(false, true), BOTH).unwrap(),
            Some(local),
            None,
        );
        let err = router.start_session(session()).unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }

    /// The router fixtures are the spec for the settings → config mapping.
    #[test]
    fn dev_settings_map_to_the_router_fixtures() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/flow-router/fixtures");
        let fixture = |name: &str| -> serde_json::Value {
            let t = std::fs::read_to_string(dir.join(name)).unwrap();
            serde_json::from_str(&t).unwrap()
        };
        let def = DevSettings::for_build(BOTH);
        assert_eq!(
            def.router_config_json(BOTH, ""),
            fixture("router_config_default.json")
        );
        let hybrid = DevSettings {
            server: ServerSettings {
                host: "100.64.0.10".into(),
                ..ServerSettings::default()
            },
            ..def.clone()
        };
        assert_eq!(
            hybrid.router_config_json(BOTH, "dev-token-0123456789abcdef"),
            fixture("router_config_hybrid.json")
        );
        let local_caps = BuildCaps {
            local: true,
            remote: false,
            cloud: false,
        };
        let local_only = DevSettings {
            route_order: vec![Route::Client, Route::Server],
            ..DevSettings::for_build(local_caps)
        };
        assert_eq!(
            local_only.router_config_json(local_caps, ""),
            fixture("router_config_local_only.json")
        );
        let remote_caps = BuildCaps {
            local: false,
            remote: true,
            cloud: false,
        };
        let remote_only = DevSettings {
            keep_local_warm: false,
            server: ServerSettings {
                host: "flow.example.com".into(),
                port: 443,
                tls: true,
                tls_pin_sha256: "3f2a9c0b7e4d1f6a8b5c2e9d0a7f4b1c6e3d8a5f2b9c0e7d4a1f6b3c8e5d2a9f"
                    .into(),
                connect_timeout_ms: 1500,
                ..ServerSettings::default()
            },
            ..DevSettings::for_build(remote_caps)
        };
        assert_eq!(
            remote_only.router_config_json(remote_caps, "dev-token-0123456789abcdef"),
            fixture("router_config_remote_only.json")
        );
        // ...and every one of them is a valid RouterConfig.
        for f in [
            "router_config_default.json",
            "router_config_hybrid.json",
            "router_config_local_only.json",
            "router_config_remote_only.json",
            "router_config_cloud.json",
        ] {
            let c: RouterConfig = serde_json::from_value(fixture(f)).unwrap();
            c.validate().unwrap();
        }
    }

    /// Shell host validation and the router's agree on the §5 table.
    #[test]
    fn host_rules_agree_with_the_router() {
        for h in [
            "100.64.0.10",
            "fd7a:115c:a1e0::1",
            "box.tailnet.ts.net",
            "mybox",
            "example.com",
        ] {
            let mut d = dev(true, true);
            d.server.host = crate::dev_settings::normalize_host(h).unwrap();
            d.server.tls = crate::dev_settings::host_is_public(&d.server.host);
            assert!(router_config(&d, BOTH).is_ok(), "{h}");
            assert_eq!(
                crate::dev_settings::host_is_public(&d.server.host),
                !flow_router::net::classify(&d.server.host).plaintext_ok(),
                "{h}"
            );
        }
    }
}
