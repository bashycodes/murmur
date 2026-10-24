//! `impl Backend for FlowCore`: the in-process engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::config::{FieldContext, SessionConfig};
use flow_core::{CommitResult, FlowCore};

use crate::{
    now_unix_ms, Backend, BackendError, CleanupPlan, EngineKind, Failure, HealthReport, StartInfo,
};

impl Backend for FlowCore {
    fn kind(&self) -> EngineKind {
        EngineKind::Local
    }

    /// A `FlowCore` exists only with its models loaded.
    fn ready(&self) -> bool {
        true
    }

    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        FlowCore::start_session(self, cfg.clone())
            .map(|_| StartInfo::default())
            .map_err(|e| BackendError::new(Failure::Error, e.to_string()))
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        FlowCore::push_audio(self, pcm)
    }

    fn set_context(&self, ctx: &FieldContext) {
        FlowCore::set_context(self, ctx.clone())
    }

    fn partial(&self) -> Option<String> {
        FlowCore::partial(self)
    }

    /// In-process commit cannot be abandoned half way; the deadline is the
    /// remote backend's concern.
    fn commit(&self, _deadline: Instant) -> Result<CommitResult, BackendError> {
        Ok(FlowCore::commit(self))
    }

    fn cancel(&self) {
        FlowCore::cancel(self)
    }

    /// D28: `plan(built-in LLM)` becomes the cleaner for later commits.
    fn install_cleanup(&self, plan: Option<CleanupPlan>) {
        self.set_cleaner(plan.and_then(|p| p(self.cleaner())));
    }

    fn probe(&self) -> HealthReport {
        HealthReport {
            ok: true,
            engine: EngineKind::Local,
            postprocess_rev: Some(flow_core::POSTPROCESS_REV.into()),
            postprocess_match: Some(true),
            checked_at_ms: now_unix_ms(),
            ..Default::default()
        }
    }
}

/// Builds the local engine on demand (loads its models). Called on a
/// router helper thread, never on the audio or UI thread; blocking is fine.
pub type LocalLoader = Arc<dyn Fn() -> Result<Arc<dyn Backend>, String> + Send + Sync>;

/// A local engine that is loaded the first time a session needs it
/// (never-fail fallback, D24): the shell did not keep a model warm
/// (`keep_local_warm = false`) but the client route is allowed. Once
/// loaded it stays loaded until the shell calls `set_local` /
/// `set_local_loader` again.
pub(crate) struct LazyLocal {
    loader: LocalLoader,
    core: Mutex<Option<Arc<dyn Backend>>>,
    /// One load at a time (a second caller waits and reuses it).
    loading: Mutex<()>,
    busy: AtomicBool,
    /// D28 cleanup plan, applied to the engine once it is loaded.
    plan: Mutex<Option<CleanupPlan>>,
}

/// What one on-demand load did.
#[derive(Debug, Clone)]
pub(crate) struct LoadNote {
    pub ms: u32,
    pub error: Option<String>,
}

impl LazyLocal {
    pub(crate) fn new(loader: LocalLoader) -> Self {
        Self {
            loader,
            core: Mutex::new(None),
            loading: Mutex::new(()),
            busy: AtomicBool::new(false),
            plan: Mutex::new(None),
        }
    }

    pub(crate) fn loaded(&self) -> Option<Arc<dyn Backend>> {
        self.core
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|c| c.ready())
    }

    /// A load is running right now.
    pub(crate) fn is_loading(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// The engine, loading it first if needed. `Some(note)` when this
    /// call ran the loader.
    pub(crate) fn ensure(&self) -> (Result<Arc<dyn Backend>, String>, Option<LoadNote>) {
        let _g = self.loading.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = self.loaded() {
            return (Ok(c), None);
        }
        self.busy.store(true, Ordering::SeqCst);
        let t0 = Instant::now();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.loader)()))
            .unwrap_or_else(|_| Err("model loader panicked".into()));
        let ms = t0.elapsed().as_millis().min(u32::MAX as u128) as u32;
        self.busy.store(false, Ordering::SeqCst);
        match r {
            Ok(c) => {
                let plan = self.plan.lock().unwrap_or_else(|e| e.into_inner()).clone();
                c.install_cleanup(plan);
                *self.core.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
                (Ok(c), Some(LoadNote { ms, error: None }))
            }
            Err(e) => (Err(e.clone()), Some(LoadNote { ms, error: Some(e) })),
        }
    }
}

impl Backend for LazyLocal {
    fn kind(&self) -> EngineKind {
        EngineKind::Local
    }

    /// Ready = loaded. A cold one is still a usable fallback (the router
    /// knows it can load it).
    fn ready(&self) -> bool {
        self.loaded().is_some()
    }

    fn start_session(&self, cfg: &SessionConfig) -> Result<StartInfo, BackendError> {
        match self.ensure().0 {
            Ok(c) => c.start_session(cfg),
            Err(e) => Err(BackendError::new(
                Failure::LocalNotReady,
                format!("phone model failed to load: {e}"),
            )),
        }
    }

    fn push_audio(&self, pcm: &[i16]) -> bool {
        self.loaded().is_some_and(|c| c.push_audio(pcm))
    }

    fn set_context(&self, ctx: &FieldContext) {
        if let Some(c) = self.loaded() {
            c.set_context(ctx)
        }
    }

    fn partial(&self) -> Option<String> {
        self.loaded()?.partial()
    }

    fn commit(&self, deadline: Instant) -> Result<CommitResult, BackendError> {
        match self.loaded() {
            Some(c) => c.commit(deadline),
            None => Err(BackendError::new(
                Failure::LocalNotReady,
                "local model not loaded",
            )),
        }
    }

    fn cancel(&self) {
        if let Some(c) = self.loaded() {
            c.cancel()
        }
    }

    fn install_cleanup(&self, plan: Option<CleanupPlan>) {
        *self.plan.lock().unwrap_or_else(|e| e.into_inner()) = plan.clone();
        if let Some(c) = self.loaded() {
            c.install_cleanup(plan);
        }
    }

    fn probe(&self) -> HealthReport {
        match self.loaded() {
            Some(c) => c.probe(),
            None => HealthReport::failed(
                EngineKind::Local,
                &BackendError::new(Failure::LocalNotReady, "loaded on demand"),
            ),
        }
    }
}
