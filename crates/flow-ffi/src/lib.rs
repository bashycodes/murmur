//! Thin C ABI (+ feature `jni`) over flow-core. Configs and results cross
//! as JSON strings; the handle is an opaque pointer (a `jlong` on Android).
//!
//! Conventions: functions returning `c_int` give 0 = ok, -1 = error (see
//! `flow_last_error`), 1 = ignored (no active session). Returned strings
//! are owned by the caller and must be freed with `flow_string_free`.
//! No function panics across the boundary.
//!
//! D17: a handle is either a bare `FlowCore` (`flow_load_models`) or a
//! `flow_router::Router` (`flow_open_router`, which may own a FlowCore as its
//! local backend). Every session function works on both kinds. The network
//! client (`flow-remote`) is linked only with feature `remote`.
//!
//! D18: with feature `cloud` the router also gets the third-party cloud STT
//! route (`flow_cloud::attach_with`); keys enter only via `flow_set_cloud_key`.
//! Without it every cloud function answers "not in this build".

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};

use std::sync::Arc;

use flow_core::config::{FieldContext, ModelConfig, SessionConfig};
use flow_core::{CommitResult, FlowCore};
use flow_router::{
    Backend, CloudProviderConfig, CloudUsage, Route, Router, RouterConfig, RouterPoll,
    SecretString, ServerConfig, StartInfo,
};

#[cfg(feature = "jni")]
pub mod jni_api;

/// Opaque handle: a bare engine or the router (D17 §6.2).
pub enum FlowHandle {
    Core(FlowCore),
    // Boxed: the router is ~1.3 KB, the core a few words.
    Router(Box<Router>),
}

impl FlowHandle {
    /// A core handle reports no route (`StartInfo::default()`).
    fn start_session(&self, cfg: SessionConfig) -> Result<StartInfo, (&'static str, String)> {
        match self {
            FlowHandle::Core(c) => c
                .start_session(cfg)
                .map(|_| StartInfo::default())
                .map_err(|e| ("error", e.to_string())),
            FlowHandle::Router(r) => r
                .start_session_info(cfg)
                .map_err(|e| (e.kind(), e.to_string())),
        }
    }
    /// `Router::poll`; a core handle gives `FlowCore::poll` with no route.
    fn poll(&self) -> RouterPoll {
        match self {
            FlowHandle::Core(c) => RouterPoll {
                session: c.poll(),
                route: None,
                loading: false,
            },
            FlowHandle::Router(r) => r.poll(),
        }
    }
    fn push_audio(&self, pcm: &[i16]) -> bool {
        match self {
            FlowHandle::Core(c) => c.push_audio(pcm),
            FlowHandle::Router(r) => r.push_audio(pcm),
        }
    }
    fn set_context(&self, ctx: FieldContext) {
        match self {
            FlowHandle::Core(c) => c.set_context(ctx),
            FlowHandle::Router(r) => r.set_context(ctx),
        }
    }
    fn partial(&self) -> Option<String> {
        match self {
            FlowHandle::Core(c) => c.partial(),
            FlowHandle::Router(r) => r.partial(),
        }
    }
    fn commit(&self) -> CommitResult {
        match self {
            FlowHandle::Core(c) => c.commit(),
            FlowHandle::Router(r) => r.commit(),
        }
    }
    fn cancel(&self) {
        match self {
            FlowHandle::Core(c) => c.cancel(),
            FlowHandle::Router(r) => r.cancel(),
        }
    }
    fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<flow_core::learn::Candidate> {
        match self {
            FlowHandle::Core(c) => c.learn_edit(pasted, edited),
            FlowHandle::Router(r) => r.learn_edit(pasted, edited),
        }
    }
    pub fn router(&self) -> Option<&Router> {
        match self {
            FlowHandle::Router(r) => Some(r),
            FlowHandle::Core(_) => None,
        }
    }
}

/// What this library was built with: local engines (ASR features) and the server route.
fn build_local_engines() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(feature = "whisper") {
        v.push("whisper".to_string());
    }
    if cfg!(feature = "parakeet") {
        v.push("parakeet".to_string());
    }
    v
}

fn remote_factory() -> Option<flow_router::RemoteFactory> {
    #[cfg(feature = "remote")]
    {
        Some(flow_remote::factory(&format!(
            "flow-ffi {}",
            flow_build_info::FULL
        )))
    }
    #[cfg(not(feature = "remote"))]
    {
        None
    }
}

fn new_router(cfg: RouterConfig, local: Option<Arc<FlowCore>>) -> Router {
    let engines = build_local_engines();
    // A passed-in core (even a mock-ASR one) is a local engine; with none, the
    // build's ASR features decide (allow_client without a model = local_not_ready).
    let capable = local.is_some() || !engines.is_empty();
    let backend = local.clone().map(|c| c as Arc<dyn Backend>);
    let router = Router::new(cfg, backend, remote_factory())
        .with_local_capability(capable)
        .with_local_info(engines, cfg!(feature = "llm"));
    attach_cloud(router, local)
}

/// D18: the cloud route shares the local core's LLM cleaner (if one is loaded),
/// so provider text goes through the same local pipeline (C5).
#[cfg(feature = "cloud")]
fn attach_cloud(router: Router, local: Option<Arc<FlowCore>>) -> Router {
    flow_cloud::attach_with(router, move || local.as_ref().and_then(|c| c.cleaner()))
}

#[cfg(not(feature = "cloud"))]
fn attach_cloud(router: Router, _local: Option<Arc<FlowCore>>) -> Router {
    router
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
    static LAST_ERROR_KIND: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: impl Into<String>) {
    set_error_kind("error", msg);
}

fn set_error_kind(kind: &str, msg: impl Into<String>) {
    let s = CString::new(msg.into().replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = s);
    let k = CString::new(kind).unwrap_or_default();
    LAST_ERROR_KIND.with(|e| *e.borrow_mut() = k);
}

fn clear_error() {
    set_error_kind("", "");
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_error("panic inside flow-core");
        fallback
    })
}

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn out_string(s: String) -> *mut c_char {
    CString::new(s.replace('\0', " ")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Shared JSON-level API, reused by the JNI layer.
pub mod json_api {
    use super::*;

    pub fn load_models(cfg_json: &str) -> Result<FlowCore, String> {
        load_models_kind(cfg_json).map_err(|(_, msg)| msg)
    }

    /// Like [`load_models`], with the error's stable kind
    /// (`bad_config`, or [`flow_core::CoreError::kind`]).
    pub fn load_models_kind(cfg_json: &str) -> Result<FlowCore, (&'static str, String)> {
        let cfg: ModelConfig = serde_json::from_str(cfg_json)
            .map_err(|e| ("bad_config", format!("bad model config: {e}")))?;
        FlowCore::load_models(&cfg).map_err(|e| (e.kind(), e.to_string()))
    }

    pub fn start_session(h: &FlowHandle, cfg_json: &str) -> Result<(), (&'static str, String)> {
        start_session_info(h, cfg_json).map(|_| ())
    }

    /// Like [`start_session`], returning the `StartInfo` JSON
    /// (`{"queued":bool,"route"?:SessionRoute}`; no `route` for a core handle).
    pub fn start_session_info(
        h: &FlowHandle,
        cfg_json: &str,
    ) -> Result<String, (&'static str, String)> {
        let cfg: SessionConfig = serde_json::from_str(cfg_json)
            .map_err(|e| ("bad_config", format!("bad session config: {e}")))?;
        let info = h.start_session(cfg)?;
        Ok(serde_json::to_string(&info).unwrap_or_else(|_| "{\"queued\":false}".into()))
    }

    /// `RouterPoll` JSON: partial + hands-free endpoint (D19) + live `route`
    /// (router handles while a session is live). `None` handle = idle poll.
    pub fn poll(h: Option<&FlowHandle>) -> String {
        let p = h.map(FlowHandle::poll).unwrap_or_default();
        serde_json::to_string(&p).unwrap_or_else(|_| idle_poll())
    }

    pub fn idle_poll() -> String {
        serde_json::to_string(&RouterPoll::default()).unwrap_or_else(|_| "{}".into())
    }

    pub fn set_context(core: &FlowHandle, ctx_json: &str) -> Result<(), String> {
        let ctx: FieldContext =
            serde_json::from_str(ctx_json).map_err(|e| format!("bad context: {e}"))?;
        core.set_context(ctx);
        Ok(())
    }

    pub fn commit(core: Option<&FlowHandle>) -> String {
        match core {
            Some(c) => serde_json::to_string(&c.commit())
                .unwrap_or_else(|e| error_result(&format!("serialize: {e}"))),
            None => error_result("null handle"),
        }
    }

    /// A `CommitResult` JSON with status ERROR, for failures outside the core.
    pub fn error_result(msg: &str) -> String {
        serde_json::json!({
            "status": "ERROR", "asr_text": "", "final_text": "", "language": null,
            "timings": {"audio_ms": 0, "asr_ms": 0, "llm_ms": 0, "postprocess_ms": 0,
                        "total_ms": 0, "dropped_samples": 0, "asr_audio_ms": 0},
            "press_enter": false, "fallback_reason": null, "truncated": false, "error": msg,
            "segments": [], "plain_text": "", "empty_reason": null,
        })
        .to_string()
    }

    pub fn learn_edit(core: &FlowHandle, pasted: &str, edited: &str) -> String {
        serde_json::to_string(&core.learn_edit(pasted, edited)).unwrap()
    }

    /// `flow_router::Capabilities` of this build (no handle needed).
    pub fn capabilities() -> String {
        serde_json::to_string(&new_router(RouterConfig::default(), None).capabilities())
            .unwrap_or_else(|_| "{}".into())
    }

    /// Parse + validate a `RouterConfig` (`bad_config` on failure).
    pub fn router_config(json: &str) -> Result<RouterConfig, (&'static str, String)> {
        let cfg: RouterConfig = serde_json::from_str(json)
            .map_err(|e| ("bad_config", format!("bad_config: bad router config: {e}")))?;
        cfg.validate().map_err(|e| (e.kind(), e.to_string()))?;
        Ok(cfg)
    }

    /// Router over an optional local engine. Both routes off is refused
    /// here (`no_engine_enabled`) so the shell can say so before a session.
    pub fn open_router(
        json: &str,
        local: Option<FlowCore>,
    ) -> Result<FlowHandle, (&'static str, String)> {
        let cfg = router_config(json)?;
        // A cloud-only config (D18) is an engine too, in builds that have it.
        let cloud = cfg!(feature = "cloud") && cfg.allow_cloud && cfg.order.contains(&Route::Cloud);
        if !cfg.allow_server && !cfg.allow_client && !cloud {
            let e = flow_router::RouterError::NoEngineEnabled;
            return Err((e.kind(), e.to_string()));
        }
        Ok(FlowHandle::Router(Box::new(new_router(
            cfg,
            local.map(Arc::new),
        ))))
    }

    pub fn update_router(h: &FlowHandle, json: &str) -> Result<(), (&'static str, String)> {
        let r = h
            .router()
            .ok_or(("bad_config", "not a router handle".to_string()))?;
        r.update_config(router_config(json)?);
        Ok(())
    }

    /// D24 never-fail fallback: give the router a way to load the local
    /// engine on demand (`keep_local_warm = false`). `json` = an array of
    /// `ModelConfig`s tried in order (e.g. Parakeet + LLM, Parakeet, whisper);
    /// the first that loads is used. `[]` removes the loader.
    pub fn set_local_loader(h: &FlowHandle, json: &str) -> Result<(), (&'static str, String)> {
        let r = h
            .router()
            .ok_or(("bad_config", "not a router handle".to_string()))?;
        let attempts: Vec<ModelConfig> = serde_json::from_str(json)
            .map_err(|e| ("bad_config", format!("bad_config: bad model configs: {e}")))?;
        if attempts.is_empty() {
            r.set_local_loader(None);
            return Ok(());
        }
        r.set_local_loader(Some(Arc::new(move || {
            let mut errors = Vec::new();
            for c in &attempts {
                match FlowCore::load_models(c) {
                    Ok(core) => return Ok(Arc::new(core) as Arc<dyn Backend>),
                    Err(e) => errors.push(e.to_string()),
                }
            }
            Err(errors.join("; "))
        })));
        Ok(())
    }

    /// D24 pre-warm (blocking up to the connect timeout): `Router::prewarm`.
    pub fn prewarm(h: &FlowHandle) -> bool {
        h.router().is_some_and(Router::prewarm)
    }

    /// Blocking probe of an unsaved server config (Developer "Test connection").
    pub fn probe_server(server_json: &str) -> String {
        let report = match serde_json::from_str::<ServerConfig>(server_json) {
            Ok(server) => new_router(RouterConfig::default(), None).probe_config(&server),
            Err(e) => flow_router::HealthReport::failed(
                flow_router::EngineKind::Remote,
                &flow_router::BackendError::new(
                    flow_router::Failure::Error,
                    format!("bad server config: {e}"),
                ),
            ),
        };
        serde_json::to_string(&report).unwrap_or_else(|_| "{\"ok\":false}".into())
    }

    pub fn probe_router(h: &FlowHandle) -> String {
        match h.router() {
            Some(r) => {
                serde_json::to_string(&r.probe_server()).unwrap_or_else(|_| "{\"ok\":false}".into())
            }
            None => "{\"ok\":false,\"error\":\"error\",\"message\":\"not a router handle\"}".into(),
        }
    }

    pub fn route_status(h: &FlowHandle) -> String {
        match h.router() {
            Some(r) => serde_json::to_string(&r.route_status()).unwrap_or_else(|_| "{}".into()),
            None => "{}".into(),
        }
    }

    // ---- D18 cloud STT (docs/cloud-stt.md "What A needs") ----------------

    /// `Router::set_cloud_key`; `None` / empty forgets the key. The key is
    /// moved into a `SecretString` (wiped on drop) and never logged or echoed.
    pub fn set_cloud_key(
        h: &FlowHandle,
        provider_id: &str,
        key: Option<String>,
    ) -> Result<(), (&'static str, String)> {
        let r = h
            .router()
            .ok_or(("error", "not a router handle".to_string()))?;
        r.set_cloud_key(provider_id, key.map(SecretString::new));
        Ok(())
    }

    /// `Router::set_cloud_key_for`: the key is bound to this provider's
    /// base-URL host + adapter (review S4).
    pub fn set_cloud_key_for(
        h: &FlowHandle,
        provider_json: &str,
        key: Option<String>,
    ) -> Result<(), (&'static str, String)> {
        let r = h
            .router()
            .ok_or(("error", "not a router handle".to_string()))?;
        let p: CloudProviderConfig = serde_json::from_str(provider_json).map_err(|e| {
            (
                "bad_config",
                format!("bad_config: bad provider config: {e}"),
            )
        })?;
        r.set_cloud_key_for(&p, key.map(SecretString::new));
        Ok(())
    }

    /// Blocking "Test provider" `HealthReport` for an unsaved provider
    /// config. Without a router handle a temporary router is used (then the
    /// key must be passed).
    pub fn probe_cloud(h: Option<&FlowHandle>, provider_json: &str, key: Option<String>) -> String {
        let report = match serde_json::from_str::<CloudProviderConfig>(provider_json) {
            Ok(p) => {
                let key = key.map(SecretString::new);
                match h.and_then(FlowHandle::router) {
                    Some(r) => r.probe_cloud(&p, key),
                    None => new_router(RouterConfig::default(), None).probe_cloud(&p, key),
                }
            }
            Err(e) => flow_router::HealthReport::failed(
                flow_router::EngineKind::Cloud,
                &flow_router::BackendError::new(
                    flow_router::Failure::CloudNotConfigured,
                    format!("bad provider config: {e}"),
                ),
            ),
        };
        serde_json::to_string(&report).unwrap_or_else(|_| "{\"ok\":false}".into())
    }

    /// `CloudUsage` JSON (an empty month for a core handle).
    pub fn cloud_usage(h: &FlowHandle) -> String {
        let u = h.router().map(Router::cloud_usage).unwrap_or_default();
        serde_json::to_string(&u).unwrap_or_else(|_| "{}".into())
    }

    pub fn set_cloud_usage(h: &FlowHandle, json: &str) -> Result<(), (&'static str, String)> {
        let r = h
            .router()
            .ok_or(("error", "not a router handle".to_string()))?;
        let u: CloudUsage = serde_json::from_str(json)
            .map_err(|e| ("bad_config", format!("bad_config: bad cloud usage: {e}")))?;
        r.set_cloud_usage(u);
        Ok(())
    }

    pub fn reset_cloud_usage(h: &FlowHandle) {
        if let Some(r) = h.router() {
            r.reset_cloud_usage();
        }
    }

    /// `CloudCapabilities` of this build (`cloud:false, adapters:[]` without `cloud`).
    pub fn cloud_capabilities() -> String {
        serde_json::to_string(&new_router(RouterConfig::default(), None).cloud_capabilities())
            .unwrap_or_else(|_| "{\"cloud\":false,\"adapters\":[]}".into())
    }

    /// Provider presets (`flow_cloud::presets()`), `[]` without `cloud`.
    pub fn cloud_presets() -> String {
        #[cfg(feature = "cloud")]
        let presets = flow_cloud::presets();
        #[cfg(not(feature = "cloud"))]
        let presets: Vec<CloudProviderConfig> = Vec::new();
        serde_json::to_string(&presets).unwrap_or_else(|_| "[]".into())
    }
}

/// Load models; returns NULL on failure (see `flow_last_error`).
///
/// # Safety
/// `model_config_json` must be NULL or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn flow_load_models(model_config_json: *const c_char) -> *mut FlowHandle {
    guard(std::ptr::null_mut(), || {
        let Some(j) = str_arg(model_config_json) else {
            set_error_kind("bad_config", "bad model config: null or non-UTF-8");
            return std::ptr::null_mut();
        };
        match json_api::load_models_kind(j) {
            Ok(c) => {
                clear_error();
                Box::into_raw(Box::new(FlowHandle::Core(c)))
            }
            Err((kind, e)) => {
                set_error_kind(kind, e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Free models and the handle. NULL is a no-op.
///
/// # Safety
/// `h` must be NULL or a handle from `flow_load_models`, not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn flow_unload(h: *mut FlowHandle) {
    if !h.is_null() {
        guard((), || drop(Box::from_raw(h)));
    }
}

/// # Safety
/// `h` valid handle or NULL; `session_config_json` valid string or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_start_session(
    h: *mut FlowHandle,
    session_config_json: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(session_config_json)) else {
            set_error("null handle or config");
            return -1;
        };
        json_api::start_session(h, j).map_or_else(
            |(kind, e)| {
                set_error_kind(kind, e);
                -1
            },
            |_| 0,
        )
    })
}

/// `flow_start_session` returning the `StartInfo` JSON
/// (`{"queued":bool,"route"?:{"engine","provider"?,"model"?,"server"?,"fallback"?}}`),
/// or NULL with the same last error / kind as `flow_start_session`.
///
/// # Safety
/// As `flow_start_session`.
#[no_mangle]
pub unsafe extern "C" fn flow_start_session_info(
    h: *mut FlowHandle,
    session_config_json: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(session_config_json)) else {
            set_error("null handle or config");
            return std::ptr::null_mut();
        };
        match json_api::start_session_info(h, j) {
            Ok(s) => {
                clear_error();
                out_string(s)
            }
            Err((kind, e)) => {
                set_error_kind(kind, e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Non-blocking `RouterPoll` JSON: `{"partial","endpoint_reached","speech_ms",
/// "trailing_silence_ms","route"?}`. NULL for a NULL handle.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_poll(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || match h.as_ref() {
        Some(h) => out_string(json_api::poll(Some(h))),
        None => std::ptr::null_mut(),
    })
}

/// Non-blocking copy of PCM16 16 kHz mono samples into the ring buffer.
///
/// # Safety
/// `pcm` must point to `len` readable samples (may be NULL when len == 0).
#[no_mangle]
pub unsafe extern "C" fn flow_push_audio(h: *mut FlowHandle, pcm: *const i16, len: usize) -> c_int {
    guard(-1, || {
        let Some(h) = h.as_ref() else { return -1 };
        let s = if len == 0 || pcm.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(pcm, len)
        };
        if h.push_audio(s) {
            0
        } else {
            1
        }
    })
}

/// # Safety
/// As `flow_start_session`.
#[no_mangle]
pub unsafe extern "C" fn flow_set_context(
    h: *mut FlowHandle,
    context_json: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(context_json)) else {
            return -1;
        };
        json_api::set_context(h, j).map_or_else(
            |e| {
                set_error(e);
                -1
            },
            |_| 0,
        )
    })
}

/// Latest partial transcript, or NULL.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_partial(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        match h.as_ref().and_then(|h| h.partial()) {
            Some(p) => out_string(p),
            None => std::ptr::null_mut(),
        }
    })
}

/// Blocking: finish the session and return the CommitResult JSON.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_commit(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        out_string(json_api::commit(h.as_ref()))
    })
}

/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_cancel(h: *mut FlowHandle) {
    if let Some(h) = h.as_ref() {
        guard((), || h.cancel());
    }
}

/// JSON array of `{original, corrected}` dictionary candidates.
///
/// # Safety
/// `h` valid handle; strings valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_learn_edit(
    h: *mut FlowHandle,
    pasted: *const c_char,
    edited: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (Some(h), Some(p), Some(e)) = (h.as_ref(), str_arg(pasted), str_arg(edited)) else {
            return std::ptr::null_mut();
        };
        out_string(json_api::learn_edit(h, p, e))
    })
}

// ---- D17 router (docs/hybrid-architecture.md §6.2) ----------------------

/// `Capabilities` JSON of this build. Free with `flow_string_free`.
#[no_mangle]
pub extern "C" fn flow_capabilities() -> *mut c_char {
    guard(
        std::ptr::null_mut(),
        || out_string(json_api::capabilities()),
    )
}

/// Open a router over `local` (a `flow_load_models` handle or NULL). Takes
/// ownership of `local` whatever the outcome (do not unload it afterwards).
/// NULL + last error kind `no_engine_enabled` / `bad_config` on failure.
///
/// # Safety
/// `json` valid string or NULL; `local` NULL or a core handle not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn flow_open_router(
    json: *const c_char,
    local: *mut FlowHandle,
) -> *mut FlowHandle {
    guard(std::ptr::null_mut(), || {
        let local = if local.is_null() {
            None
        } else {
            Some(*Box::from_raw(local))
        };
        let local = match local {
            None => None,
            Some(FlowHandle::Core(c)) => Some(c),
            Some(FlowHandle::Router(_)) => {
                set_error_kind("bad_config", "bad_config: local handle is a router");
                return std::ptr::null_mut();
            }
        };
        let Some(j) = str_arg(json) else {
            set_error_kind("bad_config", "bad_config: null router config");
            return std::ptr::null_mut();
        };
        match json_api::open_router(j, local) {
            Ok(h) => {
                clear_error();
                Box::into_raw(Box::new(h))
            }
            Err((kind, e)) => {
                set_error_kind(kind, e);
                std::ptr::null_mut()
            }
        }
    })
}

/// `Router::update_config` (from the next session). 0 ok, -1 error.
///
/// # Safety
/// `h` a router handle or NULL; `json` valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_update_router(h: *mut FlowHandle, json: *const c_char) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(json)) else {
            set_error_kind("bad_config", "null handle or config");
            return -1;
        };
        json_api::update_router(h, j).map_or_else(
            |(kind, e)| {
                set_error_kind(kind, e);
                -1
            },
            |_| 0,
        )
    })
}

/// Blocking `HealthReport` JSON for a `ServerConfig` (unsaved form values).
///
/// # Safety
/// `json` valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_probe_server(json: *const c_char) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        out_string(json_api::probe_server(str_arg(json).unwrap_or("")))
    })
}

/// Blocking probe of the router's saved server (updates circuit + health).
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_probe_router(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || match h.as_ref() {
        Some(h) => out_string(json_api::probe_router(h)),
        None => std::ptr::null_mut(),
    })
}

/// `RouteStatus` JSON (`{}` for a core handle).
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_route_status(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || match h.as_ref() {
        Some(h) => out_string(json_api::route_status(h)),
        None => std::ptr::null_mut(),
    })
}

/// Close the circuit breaker. No-op for a core handle.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_reset_circuit(h: *mut FlowHandle) {
    if let Some(r) = h.as_ref().and_then(FlowHandle::router) {
        guard((), || r.reset_circuit());
    }
}

// ---- D18 cloud STT (docs/cloud-stt.md "What A needs") -----------------------

/// Set (`key` non-NULL, non-empty) or forget (NULL / "") a provider's API key.
/// 0 ok, -1 + kind `error` for a core / NULL handle. Never logs the key.
///
/// # Safety
/// `h` a handle or NULL; strings valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_set_cloud_key(
    h: *mut FlowHandle,
    provider_id: *const c_char,
    key_or_null: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(id)) = (h.as_ref(), str_arg(provider_id)) else {
            set_error("null handle or provider id");
            return -1;
        };
        let key = str_arg(key_or_null).map(str::to_owned);
        json_api::set_cloud_key(h, id, key).map_or_else(
            |(kind, e)| {
                set_error_kind(kind, e);
                -1
            },
            |_| 0,
        )
    })
}

/// Like `flow_set_cloud_key`, bound to exactly this `CloudProviderConfig`
/// (its base-URL host + adapter): the key is never sent anywhere else, and a
/// later config that changes either drops it. 0 ok, -1 (`bad_config` for bad
/// JSON, `error` for a core handle). Non-blocking.
///
/// # Safety
/// `h` a handle or NULL; strings valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_set_cloud_key_for(
    h: *mut FlowHandle,
    provider_json: *const c_char,
    key_or_null: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(p)) = (h.as_ref(), str_arg(provider_json)) else {
            set_error("null handle or provider");
            return -1;
        };
        let key = str_arg(key_or_null).map(str::to_owned);
        json_api::set_cloud_key_for(h, p, key).map_or_else(
            |(kind, e)| {
                set_error_kind(kind, e);
                -1
            },
            |_| 0,
        )
    })
}

/// Blocking "Test provider" `HealthReport` JSON for an unsaved
/// `CloudProviderConfig`. `key` NULL = the stored key; `h` NULL = a temporary
/// router (a key is then required).
///
/// # Safety
/// `h` a handle or NULL; strings valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_probe_cloud(
    h: *mut FlowHandle,
    provider_json: *const c_char,
    key_or_null: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let key = str_arg(key_or_null).map(str::to_owned);
        out_string(json_api::probe_cloud(
            h.as_ref(),
            str_arg(provider_json).unwrap_or(""),
            key,
        ))
    })
}

/// This month's `CloudUsage` JSON.
///
/// # Safety
/// `h` a handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_cloud_usage(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || match h.as_ref() {
        Some(h) => out_string(json_api::cloud_usage(h)),
        None => std::ptr::null_mut(),
    })
}

/// Restore persisted `CloudUsage` (other months are dropped). 0 ok, -1 error.
///
/// # Safety
/// `h` a handle or NULL; `usage_json` valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_set_cloud_usage(
    h: *mut FlowHandle,
    usage_json: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(usage_json)) else {
            set_error("null handle or usage");
            return -1;
        };
        json_api::set_cloud_usage(h, j).map_or_else(
            |(kind, e)| {
                set_error_kind(kind, e);
                -1
            },
            |_| 0,
        )
    })
}

/// Developer "Reset counters". No-op for a core / NULL handle.
///
/// # Safety
/// `h` a handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_reset_cloud_usage(h: *mut FlowHandle) {
    if let Some(h) = h.as_ref() {
        guard((), || json_api::reset_cloud_usage(h));
    }
}

/// `CloudCapabilities` JSON of this build.
#[no_mangle]
pub extern "C" fn flow_cloud_capabilities() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        out_string(json_api::cloud_capabilities())
    })
}

/// Provider presets as a JSON array (`[]` without feature `cloud`).
#[no_mangle]
pub extern "C" fn flow_cloud_presets() -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        out_string(json_api::cloud_presets())
    })
}

/// Last error on this thread ("" if none). Valid until the next call on it.
#[no_mangle]
pub extern "C" fn flow_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// Stable kind of the last error on this thread: "" (none / last load
/// succeeded), "bad_config", "model_not_found", "llm_lib_not_found",
/// "load", the router kinds ("no_engine_enabled", "local_not_ready",
/// "server_not_configured", "server_unavailable"), or "error". Valid until the next call on it.
#[no_mangle]
pub extern "C" fn flow_last_error_kind() -> *const c_char {
    LAST_ERROR_KIND.with(|e| e.borrow().as_ptr())
}

/// # Safety
/// `s` must be NULL or a string returned by this library.
#[no_mangle]
pub unsafe extern "C" fn flow_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
