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
    RemoteFactory, RouteStatus, Router, RouterConfig, RouterError, ServerConfig, SessionRoute,
    StartInfo,
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

    /// D36: the router's cleanup chain, kept by the model slot across
    /// reloads (without it the trait default dropped the server and cloud
    /// hops, and only this PC's LLM ever cleaned local dictations).
    fn install_cleanup(&self, plan: Option<flow_router::CleanupPlan>) {
        self.adapter.set_cleanup_plan(plan)
    }

    /// The on-device LLM only (Command Mode, D36 `server_cleans`).
    fn llm_cleaner(&self) -> Option<Arc<dyn flow_core::cleanup::Cleaner>> {
        self.adapter.cleaner()
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
    /// D22: release → result on this PC (network included for a server
    /// result); the latency to show. `None` = not measured by the router.
    pub client_ms: Option<u32>,
    pub wait_ms: u64,
    pub segments: u64,
    pub pre_asr_ms: u64,
    pub status: CommitStatus,
    pub empty_reason: Option<String>,
    pub error: Option<String>,
    /// D27: what the LLM did ("AI cleaned", "AI not needed (already
    /// clean)" for `llm_skipped_clean`, …); `None` = LLM off.
    pub llm: Option<String>,
    pub at_ms: u64,
    /// D28: the LLM step's engine (`local` / `cloud` / `server`), model
    /// and cloud cost; `llm_row` = "AI cleaned · OpenRouter · <model> · $x".
    pub llm_engine: Option<String>,
    pub llm_model: Option<String>,
    pub llm_cost_usd: Option<f64>,
    pub llm_row: Option<String>,
    /// D30: `cloud` / `local` / `rules` when AI cleanup was tried, and why
    /// its answer was not used.
    pub cleanup_route: Option<String>,
    pub cloud_fallback: Option<String>,
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
        // D28: OpenRouter is the only cloud provider.
        let provider_label = provider
            .as_ref()
            .map(|_| crate::cloud::PROVIDER_NAME.to_string());
        let model = route.and_then(|x| x.model.clone());
        let cost_usd = route.and_then(|x| x.cost_usd).map(|c| c.0);
        let cost_estimated = route.and_then(|x| x.cost_estimated).unwrap_or(false);
        let is_cloud = route.is_some_and(|x| x.engine == "cloud");
        let row = is_cloud.then(|| {
            crate::cloud::engine_row(model.as_deref().unwrap_or(""), cost_usd, cost_estimated)
        });
        let llm =
            crate::core_api::llm_outcome_label(crate::core_api::llm_outcome(Some(r)).as_deref());
        let llm_engine = route
            .and_then(|x| x.llm_engine.clone())
            .or_else(|| r.llm.as_ref().and_then(|u| u.engine.clone()));
        let llm_model = route
            .and_then(|x| x.llm_model.clone())
            .or_else(|| r.llm.as_ref().and_then(|u| u.model.clone()));
        let llm_cost_usd = route
            .and_then(|x| x.llm_cost_usd)
            .or_else(|| r.llm.as_ref().and_then(|u| u.cost_usd))
            .map(|c| c.0);
        let llm_row = crate::cloud::llm_row(
            llm.as_deref(),
            llm_engine.as_deref(),
            llm_model.as_deref(),
            llm_cost_usd,
            r.llm.as_ref().is_some_and(|u| u.cost_estimated),
        );
        // D30: the route actually used when AI cleanup's answer was not.
        let cleanup_route = route.and_then(|x| x.cleanup_route.clone());
        let cloud_fallback = route.and_then(|x| x.cloud_fallback.clone());
        let llm_row = crate::cloud::with_cleanup_route(
            llm_row,
            cleanup_route.as_deref(),
            cloud_fallback.as_deref(),
        );
        LastDictation {
            llm_engine,
            llm_model,
            llm_cost_usd,
            llm_row,
            cleanup_route,
            cloud_fallback,
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
            client_ms: route.and_then(|x| x.client_ms),
            wait_ms: r.timings.wait_ms,
            segments: r.timings.segments,
            pre_asr_ms: r.timings.pre_asr_ms,
            status: r.status,
            empty_reason: r.empty_reason.clone(),
            error: r.error.clone(),
            llm,
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
        "cloud_not_configured"
            | "budget_exceeded"
            | "quota_exceeded"
            | "provider_error"
            | "no_private_route"
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
    cfg.validate().map_err(config_error)?;
    Ok(cfg)
}

/// Same, for the unsaved form (Test connection) with the token in clear.
pub fn server_config(dev: &DevSettings, token: &str) -> Result<ServerConfig, String> {
    let json = dev.router_config_json(BuildCaps::current(), token);
    let cfg: RouterConfig = serde_json::from_value(json).map_err(|e| e.to_string())?;
    cfg.validate().map_err(config_error)?;
    Ok(cfg.server)
}

/// A refused config as setup and Settings show it: the router's sentence
/// without its `bad_config:` code (D76).
fn config_error(e: flow_router::RouterError) -> String {
    match e {
        flow_router::RouterError::BadConfig(m) => m,
        e => e.to_string(),
    }
}

/// The remote side of this build: `None` without the `remote` feature.
pub fn remote_factory() -> Option<RemoteFactory> {
    #[cfg(feature = "remote")]
    {
        Some(flow_remote::factory(&format!(
            "flow-windows {}",
            flow_build_info::FULL
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

    /// D24 never-fail fallback: with `keep_local_warm` off the model slot is
    /// never loaded at startup, so a server that can't be reached used to
    /// fail the dictation ("local model not ready"). With `Some(spec)` the
    /// router loads it on demand (on its failover thread, while the audio is
    /// buffered) and replays the audio into it. `None` = no on-demand load
    /// (the slot is warm, or the client route is off). Re-call on every
    /// prefs save.
    pub fn set_on_demand(&self, spec: Option<crate::core_adapter::ModelSpec>) {
        self.set_on_demand_with(spec, None);
    }

    /// [`RouterAdapter::set_on_demand`] plus the per-language model (D33
    /// Hebrew route): it starts loading in the background once the default
    /// model is in, so the fallback dictation itself is not delayed.
    pub fn set_on_demand_with(
        &self,
        spec: Option<crate::core_adapter::ModelSpec>,
        lang: Option<crate::core_adapter::LanguageSpec>,
    ) {
        let (Some(adapter), Some(spec)) = (self.local.clone(), spec) else {
            self.router.set_local_loader(None);
            return;
        };
        self.router.set_local_loader(Some(Arc::new(move || {
            if !adapter.is_ready() {
                let st = adapter.load_now(&spec);
                if !st.is_ready() {
                    return Err(st
                        .problem()
                        .unwrap_or_else(|| "speech model failed to load".into()));
                }
                if lang.is_some() && adapter.language_spec() != lang {
                    adapter.load_language(lang.clone());
                }
            }
            Ok(LocalBackend::new(adapter.clone()) as Arc<dyn Backend>)
        })));
    }

    /// D24: open the server connection ahead of a dictation (blocking up to
    /// the connect timeout; call off the UI / hook thread).
    pub fn prewarm(&self) -> bool {
        self.router.prewarm()
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

    /// Hand the OpenRouter key (Credential Manager
    /// `flow-clone/cloud/openrouter`, bound to openrouter.ai) to the router,
    /// or forget it. A key stored for another host is deleted, not sent.
    /// The optional custom cleanup server key (D52, `flow-clone/cloud/custom`)
    /// goes to the router under its own id: it never stands in for the
    /// OpenRouter key, nor the reverse. Startup and after every key change.
    pub fn load_cloud_keys(&self, store: &dyn crate::credentials::CredentialStore) {
        let key = crate::credentials::openrouter_key(store);
        self.router.set_cloud_key(
            crate::credentials::OPENROUTER,
            key.map(flow_router::SecretString::new),
        );
        let custom = crate::credentials::custom_key(store);
        self.router.set_cloud_key(
            crate::credentials::CUSTOM,
            custom.map(flow_router::SecretString::new),
        );
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
            && cfg.cloud.openrouter.stt.consent_ok()
            && self.router.has_cloud_key(crate::credentials::OPENROUTER)
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
        let spent = self.router.cloud_usage().budget_cost();
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
            RouterError::ServerUnavailable(why) if needs_tls(&why) => SERVER_NEEDS_TLS.into(),
            RouterError::ServerUnavailable(why) => format!("Server unreachable ({why})"),
            RouterError::BadConfig(m) => format!("Developer settings are invalid: {m}"),
        }
    }
}

/// Pill text for flow-remote's `connect_failed … plaintext refused …`:
/// `ws://` to an address that is not private/VPN (review S6).
pub const SERVER_NEEDS_TLS: &str =
    "Server needs TLS — its address is not private; turn on TLS in Developer settings";

/// A router/remote error that means "plaintext refused".
pub fn needs_tls(detail: &str) -> bool {
    detail.contains("plaintext refused") || detail.contains("plaintext to public address")
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
        self.start_session_info(cfg).map(|_| ())
    }

    fn start_session_info(&self, cfg: SessionConfig) -> Result<Option<SessionRoute>, String> {
        // Keep the old "dictation during model load waits a moment" rule
        // when the local engine is the one that will run.
        if let Some(l) = &self.local {
            if self.local_first() {
                let _ = l.wait_ready(LOAD_WAIT);
            }
        }
        self.router
            .start_session_info(to_core_session(&cfg))
            .map(|info| info.route)
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

    fn describe_route(&self, r: &SessionRoute) -> crate::core_api::LiveRouteLabel {
        crate::core_api::live_route_label(r, &|_| {
            Some((
                crate::cloud::PROVIDER_NAME.to_string(),
                crate::cloud::PROVIDER_HOST.to_string(),
            ))
        })
    }

    fn poll(&self) -> crate::core_api::Poll {
        let p = self.router.poll();
        crate::core_api::Poll {
            partial: p.partial.clone().filter(|s| !s.trim().is_empty()),
            endpoint_reached: p.endpoint_reached,
            route: p.route,
        }
    }

    fn commit(&self) -> CommitResult {
        let mut r = from_core_result(self.router.commit());
        // Esc / timeout during the commit (C4): dropped, not a failure.
        if r.status == CommitStatus::Error && r.error.as_deref() == Some(crate::core_api::CANCELLED)
        {
            return r;
        }
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
            } else if needs_tls(&detail) {
                r.error = Some(SERVER_NEEDS_TLS.into());
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

    /// Command Mode on the dictation cleanup route: the on-device LLM,
    /// and/or OpenRouter when cloud cleanup is allowed (the router's gate
    /// re-checks consent and budget per call). The server route never
    /// runs commands.
    fn command(
        &self,
        instruction: &str,
        selected: Option<&str>,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let base = self.local.as_ref().and_then(|a| a.cleaner());
        let cleaner = self.router.command_cleaner(base);
        let loading = self.local_llm_loading();
        if cleaner.is_none() && loading && !instruction.trim().is_empty() {
            return Err(LLM_LOADING_MSG.into());
        }
        flow_core::command::run(cleaner, instruction, selected, timeout).map_err(|e| e.to_string())
    }

    /// Transforms on the same route as Command Mode (consent, budget,
    /// OpenRouter then this PC); a selection keeps the surrounding-text
    /// gate, a dictation's transcript doesn't need it.
    fn transform(
        &self,
        instruction: &str,
        text: &str,
        source: flow_core::transform::Source,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let base = self.local.as_ref().and_then(|a| a.cleaner());
        let cleaner = self.router.command_cleaner(base);
        let loading = self.local_llm_loading();
        if cleaner.is_none() && loading && !text.trim().is_empty() {
            return Err(LLM_LOADING_MSG.into());
        }
        flow_core::transform::run_transform(cleaner, instruction, text, source, timeout)
            .map_err(|e| e.to_string())
    }
}

impl RouterAdapter {
    /// This PC's LLM is still loading (the app just started).
    fn local_llm_loading(&self) -> bool {
        self.local.as_ref().is_some_and(|a| {
            matches!(
                a.llm_status(),
                crate::core_adapter::LlmStatus::Loading { .. }
            )
        })
    }

    /// D53: what a meeting summary runs on: the user's whole AI cleanup
    /// order (this PC's LLM when loaded, the server, OpenRouter under the
    /// cleanup consent), as for a dictation's cleanup. While this PC's LLM
    /// is still loading (the app just started) it waits up to `wait` so
    /// the model is not skipped. `None` = no AI engine is on.
    pub fn summary_cleaner(
        &self,
        wait: std::time::Duration,
    ) -> Option<Arc<dyn flow_core::cleanup::Cleaner>> {
        let t0 = Instant::now();
        while t0.elapsed() < wait && self.local_llm_loading() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let base = self.local.as_ref().and_then(|a| a.cleaner());
        self.router.cleanup_cleaner(base)
    }
}

/// Command Mode pill while the on-device LLM is still loading.
pub const LLM_LOADING_MSG: &str = "AI model is still loading, try again in a moment";

// ------------------------------------------------------------------ meetings

/// The on-device side of a meeting engine (D45).
#[derive(Debug, Clone)]
pub struct MeetingLocal {
    pub spec: crate::core_adapter::ModelSpec,
    pub lang: Option<crate::core_adapter::LanguageSpec>,
    /// Load it before the first chunk (the client route is primary or kept
    /// warm); otherwise it loads on demand when the server fails (D24).
    pub warm: bool,
}

/// How a meeting job's engine is built from the saved settings (D47).
#[derive(Debug, Clone)]
pub struct MeetingPlan {
    /// The meeting order as a router config ([`DevSettings::for_meetings`]).
    pub cfg: RouterConfig,
    /// Meetings may use this PC: the job gets its own on-device slot.
    pub local: bool,
    /// This PC is the first route meetings try: load its model before the
    /// first chunk. Otherwise it loads only when the earlier routes fail
    /// (D45: a server-routed meeting loads no second model).
    pub warm: bool,
}

/// D47: the meeting engine's plan from the saved settings: the meeting
/// order (not dictation's). Err when no route is on for meetings.
pub fn meeting_plan(dev: &DevSettings, caps: BuildCaps) -> Result<MeetingPlan, String> {
    let usable = dev.meeting_routes_usable(caps);
    if usable.is_empty() {
        return Err(crate::dev_settings::NO_MEETING_ROUTE_MSG.into());
    }
    Ok(MeetingPlan {
        cfg: router_config(&dev.for_meetings(), caps)?,
        local: usable.contains(&crate::dev_settings::Route::Client),
        warm: usable[0] == crate::dev_settings::Route::Client,
    })
}

/// What a meeting transcription runs on (D45): **its own** router and
/// on-device slot, built for one job and dropped after it. The dictation
/// router holds one session at a time, so a meeting chunk on it would
/// cancel a dictation and vice versa; with its own engine both run.
pub struct MeetingEngine {
    adapter: Arc<RouterAdapter>,
}

impl MeetingEngine {
    /// `cfg` = the meeting route order ([`meeting_plan`]), `local` = the
    /// on-device model when this build has one, `remote` =
    /// [`remote_factory`], `keys` = where the OpenRouter key is.
    pub fn new(
        cfg: RouterConfig,
        local: Option<MeetingLocal>,
        remote: Option<RemoteFactory>,
        keys: Option<&dyn crate::credentials::CredentialStore>,
    ) -> Arc<MeetingEngine> {
        let slot = local.as_ref().map(|_| FlowCoreAdapter::new());
        let adapter = RouterAdapter::new(cfg, slot.clone(), remote);
        if let Some(k) = keys {
            adapter.load_cloud_keys(k);
        }
        if let (Some(core), Some(l)) = (&slot, local) {
            if l.warm {
                core.load_now(&l.spec);
                core.load_language_now(l.lang.clone());
                adapter.refresh_local();
            }
            adapter.set_on_demand_with(Some(l.spec), l.lang);
        }
        Arc::new(MeetingEngine { adapter })
    }

    pub fn adapter(&self) -> &Arc<RouterAdapter> {
        &self.adapter
    }
}

impl flow_router::meeting::ChunkTranscriber for MeetingEngine {
    fn transcribe_chunk(
        &self,
        pcm: &[i16],
        cfg: &flow_core::config::SessionConfig,
    ) -> flow_core::CommitResult {
        self.adapter.router().transcribe_chunk(pcm, cfg)
    }

    fn abort(&self) {
        self.adapter.router().cancel();
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
        /// Answer like flow-remote refusing ws:// to a public address (S6).
        public: AtomicBool,
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
                public: AtomicBool::new(false),
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
            if self.0.public.load(Ordering::SeqCst) {
                return Err(BackendError::new(
                    Failure::ConnectFailed,
                    "box.example:7313: plaintext refused: 203.0.113.5 is not private",
                ));
            }
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
        let mut d = DevSettings {
            allow_server: server,
            allow_client: client,
            server: ServerSettings {
                host: "100.64.0.10".into(),
                ..ServerSettings::default()
            },
            ..DevSettings::for_build(BOTH)
        };
        // D36: server cleanup is its own switch (a configured server with
        // it on is reached for text even when it does not transcribe);
        // "server off" here means not used at all.
        d.set_cleanup_on(flow_router::CleanupRoute::Server, server);
        d
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

    /// Echoes the command prompt's user message (proves the selection and
    /// instruction reached the LLM).
    struct EchoLlm;
    impl flow_core::cleanup::Cleaner for EchoLlm {
        fn clean(
            &self,
            p: &flow_core::cleanup::CleanPrompt,
            _deadline: Instant,
        ) -> Result<String, flow_core::cleanup::CleanError> {
            // Upper-case the selection (a transform's text) out of the
            // user message.
            let tag = if p.user.contains("<selected_text>") {
                "selected_text"
            } else {
                "text"
            };
            let sel = p
                .user
                .split(&format!("<{tag}>\n"))
                .nth(1)
                .and_then(|s| s.split(&format!("\n</{tag}>")).next())
                .unwrap_or_default();
            Ok(format!("Here's the result:\n\"{}\"", sel.to_uppercase()))
        }
    }

    fn command_rig(llm: bool) -> Arc<RouterAdapter> {
        use crate::core_adapter::LlmSpec;
        let local = FlowCoreAdapter::with_llm_loader(Arc::new(|_s: &LlmSpec| {
            Ok(crate::core_adapter::LlmLoaded {
                cleaner: Arc::new(EchoLlm),
                plugin: "fake-plugin".into(),
                backend: None,
            })
        }));
        assert!(local
            .load_now(&ModelSpec::Mock { text: "x".into() })
            .is_ready());
        if llm {
            local.load_llm_now(Some(LlmSpec {
                model_path: "/m/qwen.gguf".into(),
                lib_path: None,
                threads: Some(1),
            }));
        }
        RouterAdapter::new(
            router_config(&dev(false, true), BOTH).unwrap(),
            Some(local),
            None,
        )
    }

    #[test]
    fn command_runs_on_the_local_llm_and_sanitises_the_answer() {
        let r = command_rig(true);
        let t = std::time::Duration::from_secs(5);
        assert_eq!(
            r.command("shout it", Some("hello there"), t).as_deref(),
            Ok("HELLO THERE")
        );
    }

    #[test]
    fn transform_runs_on_the_local_llm_for_a_selection_and_a_dictation() {
        use flow_core::transform::Source;
        let r = command_rig(true);
        let t = std::time::Duration::from_secs(5);
        for src in [Source::Selection, Source::Dictation] {
            assert_eq!(
                r.transform("shout it", "hello there", src, t).as_deref(),
                Ok("HELLO THERE")
            );
        }
        let e = command_rig(false)
            .transform("polish", "hey", Source::Selection, t)
            .unwrap_err();
        assert!(e.starts_with("Transforms need AI"), "{e}");
    }

    #[test]
    fn summaries_run_on_the_cleanup_order_with_this_pcs_llm() {
        let t = std::time::Duration::ZERO;
        let r = command_rig(true);
        let cl = r.summary_cleaner(t).expect("the local LLM is on");
        assert_eq!(flow_core::cleanup::first_ready_engine(&cl), "local");
        // No LLM loaded and nothing else on: no engine.
        assert!(command_rig(false).summary_cleaner(t).is_none());
    }

    #[test]
    fn command_without_any_llm_is_a_clear_error() {
        let r = command_rig(false);
        let e = r
            .command(
                "make it formal",
                Some("hey"),
                std::time::Duration::from_secs(5),
            )
            .unwrap_err();
        assert_eq!(e, flow_core::command::CommandError::NoLlm.to_string());
    }

    #[test]
    fn last_dictation_carries_the_d22_timings_and_client_ms() {
        let cfg = router_config(&dev(true, true), BOTH).unwrap();
        let mut r = CommitResult::text(CommitStatus::RawTranscript, "hi", false);
        r.timings.total_ms = 400;
        r.timings.wait_ms = 50;
        r.timings.segments = 4;
        r.timings.pre_asr_ms = 2100;
        r.route = Some(crate::core_api::RouteInfo {
            engine: "remote".into(),
            client_ms: Some(620),
            ..Default::default()
        });
        let l = LastDictation::of(&r, &cfg);
        assert_eq!(
            (l.client_ms, l.wait_ms, l.segments, l.pre_asr_ms),
            (Some(620), 50, 4, 2100)
        );
        let j = serde_json::to_value(&l).unwrap();
        assert_eq!(j["client_ms"], 620);
        r.route = None;
        assert_eq!(LastDictation::of(&r, &cfg).client_ms, None);
    }

    #[test]
    fn last_dictation_shows_llm_skipped_clean_as_a_normal_result() {
        let cfg = router_config(&dev(true, true), BOTH).unwrap();
        let mut r = CommitResult::text(CommitStatus::RawTranscript, "hi", false);
        assert_eq!(LastDictation::of(&r, &cfg).llm, None, "LLM off");
        r.fallback_reason = Some("llm_skipped_clean".into());
        assert_eq!(
            LastDictation::of(&r, &cfg).llm.as_deref(),
            Some("AI not needed (already clean)")
        );
        r.fallback_reason = Some("llm_timeout".into());
        assert_eq!(
            LastDictation::of(&r, &cfg).llm.as_deref(),
            Some("AI too slow, formatted text used")
        );
        r.fallback_reason = None;
        r.status = CommitStatus::Formatted;
        assert_eq!(
            LastDictation::of(&r, &cfg).llm.as_deref(),
            Some("AI cleaned")
        );
    }

    #[test]
    fn a_real_router_commit_reports_client_ms() {
        let d = dev(false, true);
        let rig = rig(&d);
        let out = dictate(&rig.router).unwrap();
        let ms = out.route.as_ref().and_then(|x| x.client_ms);
        assert!(ms.is_some(), "{:?}", out.route);
        assert_eq!(rig.router.last().unwrap().client_ms, ms);
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

    /// D24: keep_local_warm off, server down: the slot is loaded on demand
    /// and the dictation still produces text (it used to fail with
    /// "server_unavailable: …; local model not ready").
    #[test]
    fn server_down_with_a_cold_local_slot_loads_on_demand() {
        let mut d = dev(true, true);
        d.keep_local_warm = false;
        let server = FakeServer::new("from the server");
        server.up.store(false, Ordering::SeqCst);
        let s2 = server.clone();
        let factory: RemoteFactory = Box::new(move |_cfg: &ServerConfig| {
            Arc::new(FakeRemote(s2.clone())) as Arc<dyn Backend>
        });
        let local = FlowCoreAdapter::new();
        assert!(!local.is_ready(), "cold: never loaded at startup");
        let r = RouterAdapter::new(
            router_config(&d, BOTH).unwrap(),
            Some(local.clone()),
            Some(factory),
        );
        // Without the on-demand loader: the old failure.
        let e = dictate(&r).unwrap_err();
        assert!(
            e.contains("Server unreachable") || e.contains("server"),
            "{e}"
        );
        r.router().reset_circuit();
        r.set_on_demand(Some(ModelSpec::Mock {
            text: "loaded on demand".into(),
        }));
        let out = dictate(&r).unwrap();
        assert_eq!(out.final_text.trim(), "loaded on demand", "{out:?}");
        let route = out.route.unwrap();
        assert_eq!(route.engine, "local");
        assert_eq!(route.fallback.as_deref(), Some("connect_failed"));
        assert!(local.is_ready(), "stays loaded for the next dictation");
        let diag = r.router().route_status().last_diag.unwrap();
        assert!(diag.local_cold && diag.local_load_ms.is_some(), "{diag:?}");
        // Next one: circuit open → straight to the now-warm slot.
        let out = dictate(&r).unwrap();
        assert_eq!(out.route.unwrap().fallback.as_deref(), Some("circuit_open"));
    }

    /// D45: a meeting chunk runs on its own engine while a dictation is
    /// open on the dictation router; neither cancels the other.
    #[test]
    fn a_meeting_chunk_does_not_disturb_an_open_dictation() {
        use flow_router::meeting::ChunkTranscriber;
        let d = dev(false, true);
        let rig = rig(&d);
        rig.router.start_session(session()).unwrap();
        rig.router.push_audio(&[3000; 16_000]);
        let meeting = MeetingEngine::new(
            router_config(&d, BOTH).unwrap(),
            Some(MeetingLocal {
                spec: ModelSpec::Mock {
                    text: "meeting words".into(),
                },
                lang: None,
                warm: true,
            }),
            None,
            None,
        );
        let cfg = flow_core::config::SessionConfig {
            raw_mode: true,
            ..flow_core::config::SessionConfig::default()
        };
        let chunk = meeting.transcribe_chunk(&[3000; 32_000], &cfg);
        assert_eq!(chunk.final_text.trim(), "meeting words", "{chunk:?}");
        rig.router.push_audio(&[3000; 16_000]);
        let out = rig.router.commit();
        assert_eq!(out.final_text.trim(), "from this pc", "{out:?}");
        assert!(!Arc::ptr_eq(
            meeting.adapter().local().unwrap(),
            rig.router.local().unwrap()
        ));
    }

    /// Probe (D45): the extra memory of a meeting engine next to a loaded
    /// dictation model. Linux only (reads VmRSS). Run with
    /// `FLOW_RSS_MODEL=<ggml .bin> cargo test --lib meeting_engine_rss --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "probe: needs a real model in FLOW_RSS_MODEL"]
    fn meeting_engine_rss() {
        use flow_router::meeting::ChunkTranscriber;
        let Ok(path) = std::env::var("FLOW_RSS_MODEL") else {
            return;
        };
        let rss_mb = || {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    let l = s.lines().find(|l| l.starts_with("VmRSS:"))?.to_string();
                    l.split_whitespace().nth(1)?.parse::<u64>().ok()
                })
                .unwrap_or(0)
                / 1024
        };
        let spec = ModelSpec::Whisper {
            path: path.into(),
            threads: None,
        };
        let base = rss_mb();
        let dictation = FlowCoreAdapter::new();
        dictation.load_now(&spec);
        let one = rss_mb();
        let meeting = MeetingEngine::new(
            router_config(&dev(false, true), BOTH).unwrap(),
            Some(MeetingLocal {
                spec: spec.clone().background(),
                lang: None,
                warm: true,
            }),
            None,
            None,
        );
        let loaded = rss_mb();
        let pcm: Vec<i16> = (0..16_000 * 20)
            .map(|i| ((i as f32 * 0.07).sin() * 6000.0) as i16)
            .collect();
        let cfg = flow_core::config::SessionConfig {
            raw_mode: true,
            max_samples: 16_000 * 60,
            ..flow_core::config::SessionConfig::default()
        };
        let r = meeting.transcribe_chunk(&pcm, &cfg);
        let ran = rss_mb();
        eprintln!(
            "RSS MB: start {base}, dictation model {one} (+{}), meeting engine loaded {loaded} (+{}), after a 20 s chunk {ran} (+{}); status {:?}",
            one - base,
            loaded - one,
            ran - one,
            r.status
        );
        drop(meeting);
        eprintln!("RSS MB after the job's engine is dropped: {}", rss_mb());
    }

    #[test]
    fn a_cold_meeting_engine_loads_its_model_on_demand() {
        use flow_router::meeting::ChunkTranscriber;
        let mut d = dev(true, true);
        d.keep_local_warm = false;
        let server = FakeServer::new("from the server");
        server.up.store(false, Ordering::SeqCst);
        let s2 = server.clone();
        let factory: RemoteFactory = Box::new(move |_cfg: &ServerConfig| {
            Arc::new(FakeRemote(s2.clone())) as Arc<dyn Backend>
        });
        let meeting = MeetingEngine::new(
            router_config(&d, BOTH).unwrap(),
            Some(MeetingLocal {
                spec: ModelSpec::Mock {
                    text: "local meeting".into(),
                },
                lang: None,
                warm: false,
            }),
            Some(factory),
            None,
        );
        assert!(!meeting.adapter().local().unwrap().is_ready());
        let cfg = flow_core::config::SessionConfig::default();
        let r = meeting.transcribe_chunk(&[3000; 32_000], &cfg);
        assert_eq!(r.final_text.trim(), "local meeting", "{r:?}");
    }

    fn meeting_order(d: &mut DevSettings, order: &[(Route, bool)]) {
        let list: Vec<crate::dev_settings::MeetingRoute> = order
            .iter()
            .map(|(route, enabled)| crate::dev_settings::MeetingRoute {
                route: *route,
                enabled: *enabled,
            })
            .collect();
        d.set_meeting_order(&list).unwrap();
    }

    fn meeting_engine(plan: MeetingPlan, server: &Arc<FakeServer>) -> Arc<MeetingEngine> {
        let s2 = server.clone();
        let factory: RemoteFactory = Box::new(move |_cfg: &ServerConfig| {
            Arc::new(FakeRemote(s2.clone())) as Arc<dyn Backend>
        });
        MeetingEngine::new(
            plan.cfg,
            plan.local.then(|| MeetingLocal {
                spec: ModelSpec::Mock {
                    text: "local meeting".into(),
                },
                lang: None,
                warm: plan.warm,
            }),
            Some(factory),
            None,
        )
    }

    /// D47: server first for meetings, this PC second. The server answers
    /// while it is up and no second model loads; when it is unreachable
    /// the chunk falls back to this PC, loaded on demand.
    #[test]
    fn a_meeting_follows_its_order_and_falls_back() {
        use flow_router::meeting::ChunkTranscriber;
        let mut d = dev(true, true);
        d.route_order = vec![Route::Client, Route::Server];
        meeting_order(&mut d, &[(Route::Server, true), (Route::Client, true)]);
        let plan = meeting_plan(&d, BOTH).unwrap();
        assert!(plan.local && !plan.warm, "{plan:?}");
        let cfg = flow_core::config::SessionConfig {
            raw_mode: true,
            ..flow_core::config::SessionConfig::default()
        };

        let up = FakeServer::new("from the server");
        let m = meeting_engine(plan.clone(), &up);
        let r = m.transcribe_chunk(&[3000; 32_000], &cfg);
        assert_eq!(r.final_text.trim(), "from the server", "{r:?}");
        assert!(
            !m.adapter().local().unwrap().is_ready(),
            "no second model while the server answers"
        );

        let down = FakeServer::new("from the server");
        down.up.store(false, Ordering::SeqCst);
        let m = meeting_engine(plan, &down);
        let r = m.transcribe_chunk(&[3000; 32_000], &cfg);
        assert_eq!(r.final_text.trim(), "local meeting", "{r:?}");
        assert_eq!(r.route.unwrap().fallback.as_deref(), Some("connect_failed"));
    }

    /// D47: this PC first for meetings while dictation prefers the server:
    /// meetings never reach the server, dictation still does.
    #[test]
    fn meeting_order_does_not_change_dictation() {
        use flow_router::meeting::ChunkTranscriber;
        let mut d = dev(true, true);
        meeting_order(&mut d, &[(Route::Client, true), (Route::Server, false)]);
        let plan = meeting_plan(&d, BOTH).unwrap();
        assert!(plan.local && plan.warm, "{plan:?}");
        let server = FakeServer::new("from the server");
        let m = meeting_engine(plan, &server);
        let r = m.transcribe_chunk(
            &[3000; 32_000],
            &flow_core::config::SessionConfig::default(),
        );
        assert_eq!(r.final_text.trim(), "local meeting", "{r:?}");
        assert_eq!(
            server.starts.load(Ordering::SeqCst),
            0,
            "server is off for meetings"
        );
        let rig = rig(&d);
        let out = dictate(&rig.router).unwrap();
        assert_eq!(out.final_text.trim(), "from the server", "{out:?}");
    }

    /// D47: no route on for meetings = the job fails with a message
    /// instead of three failed chunks; a server-only meeting gets no slot.
    #[test]
    fn meeting_plan_needs_a_route() {
        let mut d = dev(true, true);
        meeting_order(&mut d, &[(Route::Server, false), (Route::Client, false)]);
        assert_eq!(
            meeting_plan(&d, BOTH).unwrap_err(),
            crate::dev_settings::NO_MEETING_ROUTE_MSG
        );
        meeting_order(&mut d, &[(Route::Server, true), (Route::Client, false)]);
        let plan = meeting_plan(&d, BOTH).unwrap();
        assert!(!plan.local && !plan.warm);
        // Unedited: dictation's order (server first, this PC on demand).
        let plan = meeting_plan(&dev(true, true), BOTH).unwrap();
        assert!(plan.local && !plan.warm);
    }

    #[test]
    fn on_demand_load_failure_says_why() {
        let mut d = dev(true, true);
        d.keep_local_warm = false;
        let server = FakeServer::new("x");
        server.up.store(false, Ordering::SeqCst);
        let s2 = server.clone();
        let factory: RemoteFactory = Box::new(move |_cfg: &ServerConfig| {
            Arc::new(FakeRemote(s2.clone())) as Arc<dyn Backend>
        });
        let r = RouterAdapter::new(
            router_config(&d, BOTH).unwrap(),
            Some(FlowCoreAdapter::new()),
            Some(factory),
        );
        r.set_on_demand(Some(ModelSpec::Whisper {
            path: "/nonexistent/ggml-base.bin".into(),
            threads: None,
        }));
        let out = dictate(&r).unwrap();
        assert_eq!(out.status, CommitStatus::Error);
        let e = out.error.unwrap();
        assert!(
            e.contains("phone model failed to load") || e.contains("failed to load"),
            "{e}"
        );
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
    fn plaintext_refused_reads_server_needs_tls() {
        let r = rig(&dev(true, false));
        r.server.public.store(true, Ordering::SeqCst);
        let err = match dictate(&r.router) {
            Err(e) => e,
            Ok(res) => res.error.unwrap_or_default(),
        };
        assert_eq!(err, SERVER_NEEDS_TLS);
        assert!(needs_tls(
            "connect_failed: plaintext to public address (enable TLS)"
        ));
        assert!(!needs_tls("connect_failed: refused"));
    }

    #[test]
    fn a_cancelled_commit_is_returned_as_is() {
        let r = rig(&dev(true, true));
        CoreApi::start_session(&*r.router, session()).unwrap();
        CoreApi::push_audio(&*r.router, &[0i16; 1600]);
        let router = r.router.clone();
        let t = std::thread::spawn(move || CoreApi::commit(&*router));
        std::thread::sleep(std::time::Duration::from_millis(5));
        CoreApi::cancel(&*r.router);
        let res = t.join().unwrap();
        // Either the commit finished first, or it was cancelled, or (slow
        // runner: the commit thread had not started within 5 ms) the cancel
        // ended the session before commit ran. A cancel is never rewritten
        // into a "Server unreachable" pill.
        if res.status == CommitStatus::Error {
            let err = res.error.as_deref();
            assert!(
                err == Some(crate::core_api::CANCELLED) || err == Some("no active session"),
                "{res:?}"
            );
        }
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
        // D36: the shell always sends the whole cleanup order, which the
        // pre-D36 fixtures leave out (= the same behaviour): compared
        // without it here, checked in tests/cleanup_order.rs.
        let fixture_cfg = fixture;
        let fixture = |name: &str| -> serde_json::Value {
            let mut j = fixture_cfg(name);
            assert!(j.get("cleanup_order").is_none(), "{name}");
            j.as_object_mut().unwrap().remove("cleanup_off");
            j
        };
        let without_cleanup = |mut j: serde_json::Value| -> serde_json::Value {
            let o = j.as_object_mut().unwrap();
            assert_eq!(
                o.remove("cleanup_order").unwrap().as_array().unwrap().len(),
                3
            );
            o.remove("cleanup_off");
            j
        };
        let def = DevSettings::for_build(BOTH);
        assert_eq!(
            without_cleanup(def.router_config_json(BOTH, "")),
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
            without_cleanup(hybrid.router_config_json(BOTH, "dev-token-0123456789abcdef")),
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
            without_cleanup(local_only.router_config_json(local_caps, "")),
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
            without_cleanup(
                remote_only.router_config_json(remote_caps, "dev-token-0123456789abcdef")
            ),
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
            let c: RouterConfig = serde_json::from_value(fixture_cfg(f)).unwrap();
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

    /// D76: setup's "Where should your voice become text?" saves through
    /// `server_config`, which validates the whole config. Custom cleanup
    /// picked without a model (a fresh pick, or a config kept from an
    /// earlier install) is "not configured"; a real bad model id is a
    /// sentence, not `bad_config: invalid custom cleanup model id …`.
    #[test]
    fn custom_cleanup_without_model_does_not_block_setup() {
        let mut d = DevSettings::default();
        d.cloud.cleanup_provider = flow_router::CleanupProvider::Custom;
        assert!(server_config(&d, "").is_ok());
        assert!(router_config(&d, BOTH).is_ok());
        d.cloud.custom_cleanup.model = "two words".into();
        let e = router_config(&d, BOTH).unwrap_err();
        assert!(e.starts_with("The AI cleanup server's model name"), "{e}");
        assert!(!e.contains("bad_config"), "{e}");
    }
}
