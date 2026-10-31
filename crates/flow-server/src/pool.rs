//! `workers` × `FlowCore`, loaded at startup; lease at `start`, return at
//! result / cancel / disconnect.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_core::config::ModelConfig;
use flow_core::{CoreError, FlowCore};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct Pool {
    cores: Vec<Arc<FlowCore>>,
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
    pub core: Arc<FlowCore>,
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

fn backend_name(backend: &flow_core::config::AsrBackend, path: &str) -> (String, String) {
    let name = backend.name();
    let model = std::path::Path::new(path)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    (name.into(), model)
}

impl Pool {
    /// Load `workers` cores (blocking; seconds for real models).
    pub fn load(model: &ModelConfig, workers: usize, max_queue: usize) -> Result<Self, CoreError> {
        let cores = (0..workers.max(1))
            .map(|_| FlowCore::load_models(model).map(Arc::new))
            .collect::<Result<Vec<_>, _>>()?;
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
        let req = flow_core::cleanup::CleanRequest {
            text: WARM_TEXT.into(),
            timeout_ms: WARM_TIMEOUT_MS,
            ..Default::default()
        };
        let mut warmed = 0;
        for (worker, core) in self.cores.iter().enumerate() {
            let Some(cl) = core.cleaner().filter(|c| c.available()) else {
                continue;
            };
            let (r, _, took) = flow_core::cleanup::clean_text(cl, &req, WARM_TIMEOUT_MS);
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

    pub fn from_cores(cores: Vec<Arc<FlowCore>>, model: &ModelConfig, max_queue: usize) -> Self {
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
    /// (FlowCore's routing, `ModelConfig::language_route`).
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
