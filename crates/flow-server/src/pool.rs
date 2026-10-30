//! `workers` × `FlowCore`, loaded at startup; lease at `start`, return at
//! result / cancel / disconnect.

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

pub enum LeaseError {
    /// Waited `queue_timeout_ms` without a free worker.
    Timeout,
}

fn backend_name(m: &ModelConfig) -> (String, String) {
    let name = m.asr_backend.name();
    let model = std::path::Path::new(&m.asr_path)
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
        Ok(Self::from_cores(cores, model, max_queue))
    }

    pub fn from_cores(cores: Vec<Arc<FlowCore>>, model: &ModelConfig, max_queue: usize) -> Self {
        let (asr_backend, asr_model) = backend_name(model);
        let n = cores.len();
        Self {
            free: Mutex::new((0..n).collect()),
            sem: Arc::new(Semaphore::new(n)),
            cores,
            waiting: AtomicUsize::new(0),
            max_queue,
            force_busy: AtomicBool::new(false),
            asr_backend,
            asr_model,
            llm: model.llm_path.as_deref().is_some_and(|p| !p.is_empty()),
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
