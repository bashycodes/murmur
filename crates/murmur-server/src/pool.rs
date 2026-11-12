//! `workers` × `MurmurCore`, loaded at startup; lease at `start`, return at
//! result / cancel / disconnect.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use murmur_core::config::ModelConfig;
use murmur_core::load_timings::{LoadTimings, ModelKind};
use murmur_core::{CoreError, MurmurCore};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct Pool {
    cores: Vec<Arc<MurmurCore>>,
    free: Mutex<Vec<usize>>,
    sem: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_queue: usize,
    /// Test hook: pretend every worker is leased.
    pub(crate) force_busy: AtomicBool,
    pub asr_backend: String,
    pub asr_model: String,
    /// Per-language models (`[model.languages.<code>]`, D33): language
    /// key -> (backend, model name), for `hello_ok` and debug capture.
    pub languages: BTreeMap<String, (String, String)>,
    /// The loaded config, for [`Pool::asr_for`]'s routing.
    model: ModelConfig,
    pub llm: bool,
}

pub struct Lease {
    pub core: Arc<MurmurCore>,
    idx: usize,
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // A no-op after commit; frees the worker after a disconnect.
        self.core.cancel();
        self.pool.free.lock().unwrap().push(self.idx);
    }
}

/// Synthetic dictation [`Pool::warm_llm`] cleans once per worker.
const WARM_TEXT: &str = "so um the meeting moved to thursday can you tell the team";
/// Budget of that warm-up call (a cold CPU box can take seconds).
const WARM_TIMEOUT_MS: u64 = 60_000;

pub enum LeaseError {
    /// Waited `queue_timeout_ms` without a free worker.
    Timeout,
}

fn backend_name(backend: &murmur_core::config::AsrBackend, path: &str) -> (String, String) {
    let name = backend.name();
    let model = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    (name.into(), model)
}

/// D69: the drafting length the LLM plugin reports when it is not the one
/// the config asks for (`None` when they match, or no LLM or no drafting
/// was asked for). A plugin without `murmur_llm_set_draft` reports no
/// `draft` and runs its default (off): that is `Some(0)`.
fn draft_not_applied(want: Option<usize>, info: Option<&str>) -> Option<usize> {
    let want = want.filter(|&n| n > 0)?;
    let info: serde_json::Value = serde_json::from_str(info?).ok()?;
    let got = info.get("draft").and_then(|d| d.as_u64()).unwrap_or(0) as usize;
    // The plugin caps the length at its DRAFT_MAX (8).
    (got != want.min(8)).then_some(got)
}

/// Where each whisper speech model runs, for the models a GPU plugin was
/// asked for (`[model] whisper_lib`, D84): `(warn, line)`. A model on the
/// CPU despite the plugin is a warning that says why (no driver, no GPU,
/// the GPU failed to load it).
fn asr_placement(t: &LoadTimings) -> Vec<(bool, String)> {
    t.models
        .iter()
        .filter(|m| m.kind == ModelKind::Asr)
        .filter_map(|m| {
            let device = m.device.as_deref()?;
            let model = match m.language.as_deref() {
                Some(l) => format!("speech model ({l})"),
                None => "speech model".to_string(),
            };
            Some(match (m.gpu_error.as_deref(), device) {
                (Some(e), _) => (
                    true,
                    format!("{model}: whisper runs on the CPU, the GPU is not used: {e}"),
                ),
                (None, "CPU") => (
                    true,
                    format!("{model}: whisper runs on the CPU, the GPU plugin found no GPU"),
                ),
                (None, d) => (false, format!("{model}: whisper runs on the GPU ({d})")),
            })
        })
        .collect()
}

impl Pool {
    /// Load `workers` cores (blocking; seconds for real models).
    pub fn load(model: &ModelConfig, workers: usize, max_queue: usize) -> Result<Self, CoreError> {
        let cores = (0..workers.max(1))
            .map(|_| MurmurCore::load_models(model).map(Arc::new))
            .collect::<Result<Vec<_>, _>>()?;
        for (worker, core) in cores.iter().enumerate() {
            if let Some(got) = draft_not_applied(model.llm_draft, core.llm_info().as_deref()) {
                tracing::warn!(
                    worker,
                    want = model.llm_draft,
                    got,
                    "llm_draft not applied (plugin without murmur_llm_set_draft, or MURMUR_LLM_DRAFT set)"
                );
            }
        }
        if let Some(core) = cores.first() {
            for (warn, line) in asr_placement(core.load_timings()) {
                if warn {
                    tracing::warn!("{line}");
                } else {
                    tracing::info!("{line}");
                }
            }
        }
        let pool = Self::from_cores(cores, model, max_queue);
        pool.warm_llm();
        Ok(pool)
    }

    /// Run one throwaway cleanup on every worker's LLM, so the first
    /// dictation after a restart does not pay the cold first decode (page
    /// faults on the mmapped weights, compute buffers, thread pool): on a
    /// 4-core CPU box that first call blew the client's 3 s budget while
    /// later ones fit. Blocking; a failure is logged, not fatal. Returns
    /// how many workers warmed.
    pub fn warm_llm(&self) -> usize {
        let req = murmur_core::cleanup::CleanRequest {
            text: WARM_TEXT.into(),
            timeout_ms: WARM_TIMEOUT_MS,
            ..Default::default()
        };
        let mut warmed = 0;
        for (worker, core) in self.cores.iter().enumerate() {
            let Some(cl) = core.cleaner().filter(|c| c.available()) else {
                continue;
            };
            let (r, _, took) = murmur_core::cleanup::clean_text(cl, &req, WARM_TIMEOUT_MS);
            let llm_ms = took.as_millis() as u64;
            match r {
                Ok(_) => {
                    warmed += 1;
                    tracing::info!(worker, llm_ms, "llm warmed");
                }
                Err(e) => tracing::warn!(worker, llm_ms, error = %e, "llm warm-up failed"),
            }
        }
        warmed
    }

    pub fn from_cores(cores: Vec<Arc<MurmurCore>>, model: &ModelConfig, max_queue: usize) -> Self {
        let (asr_backend, asr_model) = backend_name(&model.asr_backend, &model.asr_path);
        let languages = model
            .languages
            .iter()
            .map(|(k, m)| (k.clone(), backend_name(&m.asr_backend, &m.asr_path)))
            .collect();
        let n = cores.len();
        let has_cleaner = cores.first().is_some_and(|c| c.cleaner().is_some());
        Self {
            free: Mutex::new((0..n).collect()),
            sem: Arc::new(Semaphore::new(n)),
            cores,
            waiting: AtomicUsize::new(0),
            max_queue,
            force_busy: AtomicBool::new(false),
            asr_backend,
            asr_model,
            languages,
            model: model.clone(),
            // A configured model, or (tests, embedders) cores built with a
            // cleaner.
            llm: model.llm_path.as_deref().is_some_and(|p| !p.is_empty()) || has_cleaner,
        }
    }

    /// (backend, model name) that decodes a session with this forced
    /// language: its `[model.languages]` entry, else the default model
    /// (MurmurCore's routing, `ModelConfig::language_route`).
    pub fn asr_for(&self, language: Option<&str>) -> (&str, &str) {
        match self
            .model
            .language_route(language)
            .and_then(|k| self.languages.get(k))
        {
            Some((b, m)) => (b, m),
            None => (&self.asr_backend, &self.asr_model),
        }
    }

    pub fn workers(&self) -> u32 {
        self.cores.len() as u32
    }

    /// Leased workers.
    pub fn busy(&self) -> u32 {
        if self.force_busy.load(Ordering::Relaxed) {
            return self.workers();
        }
        (self.cores.len() - self.sem.available_permits()) as u32
    }

    fn take(self: &Arc<Self>, permit: OwnedSemaphorePermit) -> Lease {
        let idx = self
            .free
            .lock()
            .unwrap()
            .pop()
            .expect("permit implies a free core");
        Lease {
            core: self.cores[idx].clone(),
            idx,
            pool: self.clone(),
            _permit: permit,
        }
    }

    pub fn try_lease(self: &Arc<Self>) -> Option<Lease> {
        if self.force_busy.load(Ordering::Relaxed) {
            return None;
        }
        let p = self.sem.clone().try_acquire_owned().ok()?;
        Some(self.take(p))
    }

    /// Reserve a queue slot (false = queue full).
    pub fn enqueue(&self) -> bool {
        let mut cur = self.waiting.load(Ordering::SeqCst);
        loop {
            if cur >= self.max_queue {
                return false;
            }
            match self
                .waiting
                .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(c) => cur = c,
            }
        }
    }

    /// Wait for a worker after a successful `enqueue`.
    pub async fn wait_lease(self: &Arc<Self>, timeout: Duration) -> Result<Lease, LeaseError> {
        // Releases the queue slot even when the waiting task is aborted.
        struct Slot<'a>(&'a AtomicUsize);
        impl Drop for Slot<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _slot = Slot(&self.waiting);
        let deadline = tokio::time::Instant::now() + timeout;
        let r = loop {
            let acquire = self.sem.clone().acquire_owned();
            match tokio::time::timeout_at(deadline, acquire).await {
                Err(_) => break Err(LeaseError::Timeout),
                Ok(Err(_)) => break Err(LeaseError::Timeout),
                Ok(Ok(p)) if self.force_busy.load(Ordering::Relaxed) => {
                    drop(p);
                    if tokio::time::timeout_at(
                        deadline,
                        tokio::time::sleep(Duration::from_millis(20)),
                    )
                    .await
                    .is_err()
                    {
                        break Err(LeaseError::Timeout);
                    }
                }
                Ok(Ok(p)) => break Ok(self.take(p)),
            }
        };
        r
    }
}

#[cfg(test)]
mod tests {
    use super::{asr_placement, draft_not_applied};
    use murmur_core::load_timings::{LoadBackend, LoadTimings, ModelKind, ModelTiming};

    #[test]
    fn a_plugin_that_ignores_llm_draft_is_reported() {
        // Applied, or nothing asked for: quiet.
        assert_eq!(draft_not_applied(Some(4), Some(r#"{"draft":4}"#)), None);
        assert_eq!(draft_not_applied(Some(0), Some(r#"{}"#)), None);
        assert_eq!(draft_not_applied(None, Some(r#"{}"#)), None);
        assert_eq!(draft_not_applied(Some(4), None), None);
        // A plugin without murmur_llm_set_draft reports no length: off.
        assert_eq!(
            draft_not_applied(Some(4), Some(r#"{"load_ms":1}"#)),
            Some(0)
        );
        // MURMUR_LLM_DRAFT overrode the config.
        assert_eq!(draft_not_applied(Some(4), Some(r#"{"draft":2}"#)), Some(2));
    }

    fn asr(language: Option<&str>, device: Option<&str>, gpu_error: Option<&str>) -> ModelTiming {
        ModelTiming {
            kind: ModelKind::Asr,
            language: language.map(str::to_string),
            backend: LoadBackend::Whisper,
            ms: 1,
            phases: Vec::new(),
            reused: false,
            device: device.map(str::to_string),
            gpu_error: gpu_error.map(str::to_string),
        }
    }

    #[test]
    fn where_whisper_runs_is_logged_when_a_gpu_plugin_was_asked_for() {
        let t = LoadTimings {
            total_ms: 1,
            models: vec![
                asr(None, Some("CUDA: NVIDIA L4"), None),
                asr(
                    Some("he"),
                    Some("CPU"),
                    Some("whisper GPU plugin not found (libcuda.so.1: cannot open)"),
                ),
                // No plugin asked for (built-in whisper, Parakeet): quiet.
                asr(Some("ar"), None, None),
            ],
        };
        let lines = asr_placement(&t);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(
            lines[0],
            (
                false,
                "speech model: whisper runs on the GPU (CUDA: NVIDIA L4)".into()
            )
        );
        assert!(lines[1].0, "a CPU fallback is a warning");
        assert!(
            lines[1]
                .1
                .starts_with("speech model (he): whisper runs on the CPU")
                && lines[1].1.contains("libcuda.so.1"),
            "{}",
            lines[1].1
        );
        let no_gpu = asr_placement(&LoadTimings {
            total_ms: 1,
            models: vec![asr(None, Some("CPU"), None)],
        });
        assert_eq!(
            no_gpu,
            vec![(
                true,
                "speech model: whisper runs on the CPU, the GPU plugin found no GPU".into()
            )]
        );
    }
}
