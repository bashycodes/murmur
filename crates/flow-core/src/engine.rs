//! Session engine: handle, ring buffer, core-owned worker thread, commit
//! pipeline (ASR -> optional LLM -> deterministic post-processing).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::asr::{
    build_initial_prompt, dedupe_terms, AbortFlag, AsrEngine, AsrPurpose, AsrRequest,
};
use crate::cleanup::{self, CleanOutcome, Cleaner};
use crate::config::{CleanupLevel, FieldContext, ModelConfig, SessionConfig, SAMPLE_RATE};
use crate::dictionary::Dictionary;
use crate::endpoint::{EndpointState, Endpointer};
use crate::learn::{self, Candidate};
use crate::paste::{self, PasteSegment};
use crate::postprocess::{self, DictMatcher};
use crate::{hallucination, vad};

/// Partials decode at most this much trailing audio (Whisper's window).
const PARTIAL_WINDOW: usize = SAMPLE_RATE * 30;
const WORKER_TICK: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommitStatus {
    /// LLM cleanup ran and passed the guardrail.
    Formatted,
    /// Post-processed ASR text (LLM off, skipped, failed or timed out).
    RawTranscript,
    /// No speech.
    Empty,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timings {
    pub audio_ms: u64,
    pub asr_ms: u64,
    pub llm_ms: u64,
    pub postprocess_ms: u64,
    /// Wall time inside `commit()` (release -> text ready).
    pub total_ms: u64,
    /// Samples lost because the ring buffer was full.
    pub dropped_samples: u64,
    /// Audio actually given to ASR after VAD trimming (0 = ASR skipped).
    #[serde(default)]
    pub asr_audio_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitResult {
    pub status: CommitStatus,
    pub asr_text: String,
    pub final_text: String,
    pub language: Option<String>,
    pub timings: Timings,
    /// Shell should send Enter after inserting (spoken "press enter").
    pub press_enter: bool,
    /// Why the LLM output was not used, when it should have been.
    pub fallback_reason: Option<String>,
    pub truncated: bool,
    pub error: Option<String>,
    /// `final_text` as a paste plan: text runs split at the `{{BREAK}}` /
    /// `{{TAB}}` markers, plus a trailing Enter when `press_enter` is set.
    /// Empty when there is nothing to paste. Added 2026-10 (CR-2).
    #[serde(default)]
    pub segments: Vec<PasteSegment>,
    /// `final_text` without paste markers (history, "paste last").
    #[serde(default)]
    pub plain_text: String,
    /// Why an EMPTY result is empty: `no_audio`, `no_speech` (VAD: ASR was
    /// skipped), `blank_asr`, or `hallucination` (`asr_text` holds the
    /// suppressed phrase). `None` for other statuses.
    #[serde(default)]
    pub empty_reason: Option<String>,
    /// Which engine produced this result (hybrid router, D17). `FlowCore`
    /// never sets it; `flow_router::Router` always does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteInfo>,
}

/// One poll of the live session (`FlowCore::poll`, `Router::poll`): the
/// latest partial plus the hands-free endpoint signal (D19). Added
/// 2026-10-22; every field defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionPoll {
    /// Same as `partial()`.
    pub partial: Option<String>,
    /// `SessionConfig.endpoint_silence_ms` of trailing silence followed
    /// speech: a hands-free shell should commit now. Latched; always false
    /// when endpointing is off or no session is active.
    pub endpoint_reached: bool,
    /// Speech heard so far (0 when endpointing is off).
    pub speech_ms: u64,
    /// Audio since the last speech (0 when endpointing is off).
    pub trailing_silence_ms: u64,
}

impl SessionPoll {
    pub fn new(partial: Option<String>, ep: Option<EndpointState>) -> Self {
        let ep = ep.unwrap_or_default();
        Self {
            partial,
            endpoint_reached: ep.reached,
            speech_ms: ep.speech_ms,
            trailing_silence_ms: ep.trailing_silence_ms,
        }
    }
}

/// Where a result came from (`docs/hybrid-architecture.md` §1.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteInfo {
    /// `"local"` or `"remote"`.
    pub engine: String,
    /// Why the preferred engine was not used: `connect_failed`,
    /// `auth_failed`, `proto_mismatch`, `busy`, `timeout`, `disconnected`,
    /// `circuit_open`, `server_not_configured`, `local_not_ready`.
    pub fallback: Option<String>,
    /// `"host:port"` of the server, when one was involved.
    pub server: Option<String>,
    pub rtt_ms: Option<u32>,
    pub queue_ms: Option<u32>,
    /// The server's flow-core version (pipeline skew detection).
    pub server_core: Option<String>,
    /// Engine provider id for engines with several providers (opt-in cloud
    /// STT: the configured provider id, e.g. `"openrouter-1"`; `None` for
    /// local / own server).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Cloud STT model id (D18).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Audio the cloud provider billed (D18).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billed_audio_ms: Option<u32>,
    /// Cloud cost of this dictation: provider-reported, or an estimate
    /// from the price table when `cost_estimated` (D18).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<Usd>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_estimated: Option<bool>,
}

/// An amount in US dollars. A plain JSON number; a newtype only so the
/// structs holding it keep `Eq` (compared by bit pattern).
#[derive(Debug, Clone, Copy, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Usd(pub f64);

impl Eq for Usd {}

impl std::hash::Hash for Usd {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.to_bits().hash(h)
    }
}

impl std::fmt::Display for Usd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${:.4}", self.0)
    }
}

impl CommitResult {
    fn status(status: CommitStatus) -> Self {
        Self {
            status,
            asr_text: String::new(),
            final_text: String::new(),
            language: None,
            timings: Timings::default(),
            press_enter: false,
            fallback_reason: None,
            truncated: false,
            error: None,
            segments: Vec::new(),
            plain_text: String::new(),
            empty_reason: None,
            route: None,
        }
    }
    fn error(msg: impl Into<String>) -> Self {
        Self {
            error: Some(msg.into()),
            ..Self::status(CommitStatus::Error)
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("core worker is gone")]
    WorkerGone,
    /// The ASR or LLM model file does not exist (checked before the engine
    /// is invoked). CR-1.
    #[error("model not found: {path}")]
    ModelNotFound { path: String },
    /// The `flow-llm` plugin library could not be opened from any lookup
    /// location (`tried`, in order, each with its loader error).
    #[error("llm plugin not found ({})", tried.join("; "))]
    LlmLibNotFound { tried: Vec<String> },
}

impl CoreError {
    /// Stable snake_case kind for FFI callers (`flow_last_error_kind`).
    pub fn kind(&self) -> &'static str {
        match self {
            CoreError::Load(_) => "load",
            CoreError::WorkerGone => "worker_gone",
            CoreError::ModelNotFound { .. } => "model_not_found",
            CoreError::LlmLibNotFound { .. } => "llm_lib_not_found",
        }
    }
}

#[cfg(any(feature = "whisper", feature = "llm"))]
fn require_file(path: &str) -> Result<(), CoreError> {
    if std::path::Path::new(path).is_file() {
        Ok(())
    } else {
        Err(CoreError::ModelNotFound { path: path.into() })
    }
}

/// Parakeet's `asr_path` is a model directory, not a file.
#[cfg(feature = "parakeet")]
fn require_dir(path: &str) -> Result<(), CoreError> {
    if std::path::Path::new(path).is_dir() {
        Ok(())
    } else {
        Err(CoreError::ModelNotFound { path: path.into() })
    }
}

enum Ctrl {
    Start {
        id: u64,
        cfg: Box<SessionConfig>,
        consumer: rtrb::Consumer<i16>,
    },
    Commit {
        id: u64,
        /// The session's field context, snapshotted when `commit` was
        /// called: a `start_session` overlapping this commit resets the
        /// shared one for the next dictation (review C6).
        ctx: Box<FieldContext>,
        reply: mpsc::Sender<CommitResult>,
        /// Set by `cancel()` while this commit runs (review C4).
        abort: Arc<AtomicBool>,
    },
    Cancel {
        id: u64,
    },
    Shutdown,
}

struct Producer {
    id: u64,
    prod: rtrb::Producer<i16>,
    /// Hands-free endpointing, fed on the pushing thread so a long partial
    /// decode on the worker cannot delay the signal.
    endpointer: Option<Endpointer>,
}

struct Shared {
    producer: Mutex<Option<Producer>>,
    context: Mutex<FieldContext>,
    partial: Mutex<Option<String>>,
    dictionary: Mutex<Dictionary>,
    dropped: AtomicU64,
    next_id: AtomicU64,
    /// Bumped by every `set_context` (drives LLM prefill-ahead).
    context_ver: AtomicU64,
    /// DictMatcher compilations (the cache is keyed by dictionary).
    dict_builds: AtomicU64,
    /// Set by commit/cancel so an in-flight partial decode stops early.
    stop_partial: Arc<AtomicBool>,
    /// Id of the session whose partial may be shown (0 = none). A partial
    /// decode publishes only while this still equals its session id, so a
    /// decode of a cancelled/committed session that finishes late never
    /// shows up in the next session's preview (review S11).
    current: AtomicU64,
    /// The commit in progress: its session id and abort flag.
    inflight: Mutex<Option<(u64, Arc<AtomicBool>)>>,
}

impl Shared {
    /// Session `id` ended: hide its preview and stop late decodes from
    /// publishing one (a newer session keeps its own).
    fn clear_partial(&self, id: u64) {
        let mut p = self.partial.lock().unwrap();
        if self
            .current
            .compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            *p = None;
        }
    }
}

/// Opaque core handle (§9 `Handle`). One active session at a time.
pub struct FlowCore {
    shared: Arc<Shared>,
    cleaner_info: Option<Arc<dyn Cleaner>>,
    ctrl: mpsc::Sender<Ctrl>,
    worker: Option<JoinHandle<()>>,
}

impl FlowCore {
    /// Load models and keep them warm until [`FlowCore::unload`] / drop.
    pub fn load_models(cfg: &ModelConfig) -> Result<Self, CoreError> {
        use crate::config::AsrBackend;
        let asr: Arc<dyn AsrEngine> = match &cfg.asr_backend {
            AsrBackend::Mock { text } => Arc::new(crate::asr::MockAsr::new(text)),
            #[cfg(feature = "whisper")]
            AsrBackend::Whisper => {
                require_file(&cfg.asr_path)?;
                let mut w = crate::whisper::WhisperEngine::load(&cfg.asr_path, cfg.threads)
                    .map_err(|e| CoreError::Load(e.to_string()))?
                    .with_fit_ctx(cfg.whisper_fit_ctx);
                if let Some(v) = cfg.vad_model_path.as_deref().filter(|v| !v.is_empty()) {
                    require_file(v)?;
                    w = w.with_silero_vad(v);
                }
                Arc::new(w)
            }
            #[cfg(not(feature = "whisper"))]
            AsrBackend::Whisper => {
                return Err(CoreError::Load(
                    "built without the `whisper` feature".into(),
                ))
            }
            #[cfg(feature = "parakeet")]
            AsrBackend::Parakeet => {
                require_dir(&cfg.asr_path)?;
                Arc::new(
                    crate::parakeet::ParakeetAsr::load(&cfg.asr_path, cfg.threads)
                        .map_err(|e| CoreError::Load(e.to_string()))?,
                )
            }
            #[cfg(not(feature = "parakeet"))]
            AsrBackend::Parakeet => {
                return Err(CoreError::Load(
                    "built without the `parakeet` feature".into(),
                ))
            }
        };
        let cleaner: Option<Arc<dyn Cleaner>> = match cfg.llm_path.as_deref() {
            None | Some("") => None,
            #[cfg(feature = "llm")]
            Some(p) => {
                use crate::llama::{LlamaCleaner, LlamaLoadError};
                require_file(p)?;
                Some(Arc::new(
                    LlamaCleaner::load(p, cfg.threads, cfg.llm_lib.as_deref()).map_err(
                        |e| match e {
                            LlamaLoadError::PluginNotFound { tried } => {
                                CoreError::LlmLibNotFound { tried }
                            }
                            e => CoreError::Load(e.to_string()),
                        },
                    )?,
                ))
            }
            // Asking for cleanup from a build that cannot do it is a config
            // error, not something to drop silently.
            #[cfg(not(feature = "llm"))]
            Some(_) => {
                return Err(CoreError::Load(
                    "llm_path set but built without the `llm` feature".into(),
                ))
            }
        };
        Ok(Self::from_engines(asr, cleaner))
    }

    /// Build a core around caller-supplied engines (tests, custom backends).
    pub fn from_engines(asr: Arc<dyn AsrEngine>, cleaner: Option<Arc<dyn Cleaner>>) -> Self {
        let shared = Arc::new(Shared {
            producer: Mutex::new(None),
            context: Mutex::new(FieldContext::default()),
            partial: Mutex::new(None),
            dictionary: Mutex::new(Dictionary::default()),
            dropped: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            dict_builds: AtomicU64::new(0),
            context_ver: AtomicU64::new(0),
            stop_partial: Arc::new(AtomicBool::new(false)),
            current: AtomicU64::new(0),
            inflight: Mutex::new(None),
        });
        let (tx, rx) = mpsc::channel();
        let ws = shared.clone();
        let cleaner_info = cleaner.clone();
        let worker = std::thread::Builder::new()
            .name("flow-core-worker".into())
            .spawn(move || {
                Worker {
                    shared: ws,
                    asr,
                    cleaner,
                    session: None,
                    dict_cache: None,
                }
                .run(rx)
            })
            .expect("spawn worker");
        Self {
            shared,
            cleaner_info,
            ctrl: tx,
            worker: Some(worker),
        }
    }

    /// Free models and stop the worker (Android onTrimMemory, idle timeout).
    /// Dropping joins the worker, which drops the ASR engine and the cleaner;
    /// `LlamaCleaner` joins its own thread, freeing context then model.
    pub fn unload(self) {}

    /// Begin a dictation. Replaces any session still open.
    pub fn start_session(&self, cfg: SessionConfig) -> Result<(), CoreError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (prod, consumer) = rtrb::RingBuffer::new(cfg.ring_capacity_samples.max(SAMPLE_RATE));
        *self.shared.context.lock().unwrap() = FieldContext::default();
        {
            let mut p = self.shared.partial.lock().unwrap();
            self.shared.current.store(id, Ordering::SeqCst);
            *p = None;
        }
        *self.shared.dictionary.lock().unwrap() = cfg.dictionary.clone();
        self.shared.dropped.store(0, Ordering::Relaxed);
        self.shared.stop_partial.store(false, Ordering::Relaxed);
        let mut slot = self.shared.producer.lock().unwrap();
        *slot = Some(Producer {
            id,
            prod,
            endpointer: Endpointer::for_session(&cfg),
        });
        self.ctrl
            .send(Ctrl::Start {
                id,
                cfg: Box::new(cfg),
                consumer,
            })
            .map_err(|_| CoreError::WorkerGone)
    }

    /// Non-blocking: copy PCM16 16 kHz mono into the ring buffer. Returns
    /// false when no session is active. Overflow is counted, never waited on.
    pub fn push_audio(&self, pcm: &[i16]) -> bool {
        let mut slot = self.shared.producer.lock().unwrap();
        let Some(p) = slot.as_mut() else { return false };
        let n = pcm.len().min(p.prod.slots());
        if n > 0 {
            if let Ok(chunk) = p.prod.write_chunk_uninit(n) {
                chunk.fill_from_iter(pcm[..n].iter().copied());
            }
        }
        if let Some(ep) = p.endpointer.as_mut() {
            ep.push_i16(pcm);
        }
        let lost = (pcm.len() - n) as u64;
        if lost > 0 {
            self.shared.dropped.fetch_add(lost, Ordering::Relaxed);
        }
        true
    }

    /// Field context; may arrive any time before `commit`.
    pub fn set_context(&self, ctx: FieldContext) {
        *self.shared.context.lock().unwrap() = ctx;
        self.shared.context_ver.fetch_add(1, Ordering::Relaxed);
    }

    /// Cleanup backend counters (JSON), when the backend reports them.
    pub fn llm_info(&self) -> Option<String> {
        self.cleaner_info.as_ref().and_then(|c| c.info_json())
    }

    /// The LLM cleaner this core was built with, if any. Lets a second
    /// core (e.g. the cloud STT backend) share the loaded model instead of
    /// loading it twice. Additive (D18).
    pub fn cleaner(&self) -> Option<Arc<dyn Cleaner>> {
        self.cleaner_info.clone()
    }

    /// Latest live-preview transcript, if partials are enabled.
    pub fn partial(&self) -> Option<String> {
        self.shared.partial.lock().unwrap().clone()
    }

    /// Hands-free endpoint state, when `endpoint_silence_ms` is set and a
    /// session is active.
    pub fn endpoint(&self) -> Option<EndpointState> {
        let slot = self.shared.producer.lock().unwrap();
        slot.as_ref()?.endpointer.as_ref().map(Endpointer::state)
    }

    /// True once the active session's endpoint was reached (D19).
    pub fn endpoint_reached(&self) -> bool {
        self.endpoint().is_some_and(|e| e.reached)
    }

    /// Partial + endpoint in one call (what a shell polls while recording).
    pub fn poll(&self) -> SessionPoll {
        SessionPoll::new(self.partial(), self.endpoint())
    }

    /// Stop capture and run the full pipeline. Blocks until the result is ready.
    pub fn commit(&self) -> CommitResult {
        let t0 = Instant::now();
        // Snapshot the per-session state now: a `start_session` overlapping
        // this commit (hands-free back-to-back) resets it (review C6).
        // `dropped` only grows under the producer lock, taken here.
        let (id, dropped) = {
            let mut slot = self.shared.producer.lock().unwrap();
            let Some(Producer { id, .. }) = slot.take() else {
                return CommitResult::error("no active session");
            };
            (id, self.shared.dropped.load(Ordering::Relaxed))
        };
        let ctx = Box::new(self.shared.context.lock().unwrap().clone());
        self.shared.stop_partial.store(true, Ordering::Relaxed);
        self.shared.clear_partial(id);
        let (tx, rx) = mpsc::channel();
        let abort = Arc::new(AtomicBool::new(false));
        *self.shared.inflight.lock().unwrap() = Some((id, abort.clone()));
        if self
            .ctrl
            .send(Ctrl::Commit {
                id,
                ctx,
                reply: tx,
                abort,
            })
            .is_err()
        {
            return CommitResult::error("core worker is gone");
        }
        let mut r = rx
            .recv()
            .unwrap_or_else(|_| CommitResult::error("core worker is gone"));
        {
            let mut g = self.shared.inflight.lock().unwrap();
            if g.as_ref().is_some_and(|(i, _)| *i == id) {
                *g = None;
            }
        }
        r.timings.total_ms = t0.elapsed().as_millis() as u64;
        r.timings.dropped_samples = dropped;
        r
    }

    /// Abort the session; audio is discarded. With no open session, abort
    /// the commit running on another thread instead: its ASR gets the abort
    /// flag, the LLM step is skipped, and that `commit` returns ERROR
    /// `cancelled` (review C4).
    pub fn cancel(&self) {
        if let Some(Producer { id, .. }) = self.shared.producer.lock().unwrap().take() {
            self.shared.stop_partial.store(true, Ordering::Relaxed);
            self.shared.clear_partial(id);
            let _ = self.ctrl.send(Ctrl::Cancel { id });
        } else if let Some((_, abort)) = self.shared.inflight.lock().unwrap().as_ref() {
            abort.store(true, Ordering::SeqCst);
        }
    }

    /// How many times a dictionary matcher was compiled (tests, benchmarks).
    /// Sessions with an unchanged dictionary reuse the previous one.
    #[doc(hidden)]
    pub fn dict_matcher_builds(&self) -> u64 {
        self.shared.dict_builds.load(Ordering::Relaxed)
    }

    /// Word-align pasted vs edited and return dictionary candidates.
    pub fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<Candidate> {
        learn::learn_edit(pasted, edited, &self.shared.dictionary.lock().unwrap())
    }
}

impl Drop for FlowCore {
    fn drop(&mut self) {
        let _ = self.ctrl.send(Ctrl::Shutdown);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Session {
    id: u64,
    cfg: SessionConfig,
    consumer: rtrb::Consumer<i16>,
    buf: Vec<f32>,
    truncated: bool,
    last_partial_len: usize,
    /// Compiled dictionary for this session (shared with later sessions
    /// that use the same dictionary).
    matcher: Arc<DictMatcher>,
    /// (context version, partial) of the last LLM prefill hint.
    prefilled: Option<(u64, Option<String>)>,
}

impl Session {
    fn drain(&mut self) {
        let n = self.consumer.slots();
        if n == 0 {
            return;
        }
        let Ok(chunk) = self.consumer.read_chunk(n) else {
            return;
        };
        let room = self.cfg.max_samples.saturating_sub(self.buf.len());
        if n > room {
            self.truncated = true;
        }
        self.buf
            .extend(chunk.into_iter().take(room).map(|s| s as f32 / 32768.0));
    }
}

struct Worker {
    shared: Arc<Shared>,
    asr: Arc<dyn AsrEngine>,
    cleaner: Option<Arc<dyn Cleaner>>,
    session: Option<Session>,
    dict_cache: Option<(Dictionary, Arc<DictMatcher>)>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Ctrl>) {
        loop {
            match rx.recv_timeout(WORKER_TICK) {
                Ok(Ctrl::Start { id, cfg, consumer }) => {
                    let matcher = self.matcher_for(&cfg.dictionary);
                    self.session = Some(Session {
                        id,
                        cfg: *cfg,
                        consumer,
                        buf: Vec::new(),
                        truncated: false,
                        last_partial_len: 0,
                        matcher,
                        prefilled: None,
                    });
                    self.maybe_prefill();
                }
                Ok(Ctrl::Commit {
                    id,
                    ctx,
                    reply,
                    abort,
                }) => {
                    let r = match self.session.take() {
                        Some(mut s) if s.id == id => {
                            if let Some(cl) = &self.cleaner {
                                cl.cancel_prefill();
                            }
                            s.drain();
                            let r = self.finalize(s, &ctx, &abort);
                            if abort.load(Ordering::SeqCst) {
                                CommitResult::error("cancelled")
                            } else {
                                r
                            }
                        }
                        other => {
                            self.session = other;
                            CommitResult::error("session superseded")
                        }
                    };
                    let _ = reply.send(r);
                }
                Ok(Ctrl::Cancel { id }) => {
                    if self.session.as_ref().is_some_and(|s| s.id == id) {
                        self.session = None;
                    }
                }
                Ok(Ctrl::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    self.tick();
                    self.maybe_prefill();
                }
            }
        }
    }

    /// Hint the cleaner with the prompt it will probably get at commit:
    /// current context + latest partial. Sent only when either changed,
    /// and only when the LLM would run for this session.
    fn maybe_prefill(&mut self) {
        let (Some(cl), Some(s)) = (&self.cleaner, self.session.as_mut()) else {
            return;
        };
        if !s.cfg.llm_prefill || self.shared.stop_partial.load(Ordering::Relaxed) {
            return;
        }
        let ver = self.shared.context_ver.load(Ordering::Relaxed);
        let partial = self.shared.partial.lock().unwrap().clone();
        let key = (ver, partial);
        if s.prefilled.as_ref() == Some(&key) {
            return;
        }
        let mut ctx = self.shared.context.lock().unwrap().clone();
        ctx.app_category = postprocess::effective_category(s.cfg.platform, &ctx);
        let transcript = key.1.clone().unwrap_or_default();
        s.prefilled = Some(key);
        if !cleanup::should_run(&s.cfg, ctx.app_category) {
            return;
        }
        let style = s.cfg.styles.for_category(ctx.app_category);
        cl.prefill(&cleanup::build_prompt(
            &transcript,
            s.cfg.cleanup_level,
            style,
            &ctx,
            &s.cfg.dictionary,
        ));
    }

    /// The compiled matcher for `dict`, rebuilt only when it changed.
    fn matcher_for(&mut self, dict: &Dictionary) -> Arc<DictMatcher> {
        if let Some((d, m)) = &self.dict_cache {
            if d == dict {
                return m.clone();
            }
        }
        let m = Arc::new(DictMatcher::new(&dict.entries, &[]));
        self.shared.dict_builds.fetch_add(1, Ordering::Relaxed);
        self.dict_cache = Some((dict.clone(), m.clone()));
        m
    }

    fn asr_request(cfg: &SessionConfig, ctx: &FieldContext, purpose: AsrPurpose) -> AsrRequest {
        let mut seen = Vec::new();
        let dictionary_terms = dedupe_terms(cfg.dictionary.vocabulary(), &mut seen);
        let screen_terms = dedupe_terms(ctx.screen_words.iter().map(String::as_str), &mut seen);
        AsrRequest {
            initial_prompt: build_initial_prompt(&cfg.dictionary, &ctx.screen_words),
            language: cfg.forced_language().map(str::to_string),
            abort: AbortFlag::default(),
            dictionary_terms,
            screen_terms,
            purpose,
        }
    }

    fn tick(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        s.drain();
        let Some(ms) = s.cfg.partial_interval_ms else {
            return;
        };
        let step = ms as usize * SAMPLE_RATE / 1000;
        if s.buf.len() < s.last_partial_len + step.max(1) {
            return;
        }
        s.last_partial_len = s.buf.len();
        let start = s.buf.len().saturating_sub(PARTIAL_WINDOW);
        if self.shared.stop_partial.load(Ordering::Relaxed) {
            return;
        }
        let s = self.session.as_ref().unwrap();
        let mut window = &s.buf[start..];
        if s.cfg.vad {
            // No speech yet: nothing to preview, and no CPU spent on it.
            let Some(sp) = vad::detect(window) else {
                return;
            };
            window = &window[sp.start..sp.end];
        }
        let ctx = self.shared.context.lock().unwrap().clone();
        let mut req = Self::asr_request(&s.cfg, &ctx, AsrPurpose::Partial);
        req.abort = AbortFlag(Some(self.shared.stop_partial.clone()));
        if let Ok(out) = self.asr.transcribe(window, &req) {
            let t = out.text.trim();
            if !t.is_empty() {
                let mut p = self.shared.partial.lock().unwrap();
                // Only while this session is still the shown one (S11).
                if self.shared.current.load(Ordering::SeqCst) == s.id {
                    *p = Some(t.to_string());
                }
            }
        }
    }

    fn finalize(&self, s: Session, ctx: &FieldContext, abort: &Arc<AtomicBool>) -> CommitResult {
        let audio_ms = (s.buf.len() * 1000 / SAMPLE_RATE) as u64;
        let mut r = CommitResult::status(CommitStatus::Empty);
        r.truncated = s.truncated;
        r.timings.audio_ms = audio_ms;
        if s.buf.is_empty() {
            r.empty_reason = Some("no_audio".into());
            return r;
        }
        let (pcm, speech_ms) = if s.cfg.vad {
            match vad::detect(&s.buf) {
                Some(sp) => (&s.buf[sp.start..sp.end], Some(sp.speech_ms())),
                None => {
                    r.empty_reason = Some("no_speech".into());
                    return r;
                }
            }
        } else {
            (&s.buf[..], None)
        };
        r.timings.asr_audio_ms = (pcm.len() * 1000 / SAMPLE_RATE) as u64;
        let mut req = Self::asr_request(&s.cfg, ctx, AsrPurpose::Final);
        req.abort = AbortFlag(Some(abort.clone()));
        let t = Instant::now();
        let out = match self.asr.transcribe(pcm, &req) {
            Ok(o) => o,
            Err(e) => {
                let mut e = CommitResult::error(e.to_string());
                e.timings = r.timings;
                return e;
            }
        };
        r.timings.asr_ms = t.elapsed().as_millis() as u64;
        r.language = out.language;
        r.asr_text = out.text.trim().to_string();
        if r.asr_text.is_empty() {
            r.empty_reason = Some("blank_asr".into());
            return r;
        }
        let evidence = hallucination::Evidence {
            speech_ms,
            no_speech_prob: out.no_speech_prob,
        };
        if s.cfg.hallucination_guard && hallucination::is_hallucination(&r.asr_text, evidence) {
            r.empty_reason = Some("hallucination".into());
            return r;
        }
        let mut ctx = ctx.clone();
        ctx.app_category = postprocess::effective_category(s.cfg.platform, &ctx);
        let mut text = r.asr_text.clone();
        // A whole-utterance snippet trigger is pasted as its expansion; the
        // LLM must not see (or reword) it (03 §7.1).
        let snippet = s.matcher.whole_snippet(&r.asr_text).is_some();
        if s.cfg.spoken_cleanup && s.cfg.cleanup_level != CleanupLevel::None && !snippet {
            text = postprocess::spoken::spoken_cleanup(&text);
            // Numbered lists and email structure without the LLM (the
            // LLM, when it runs, starts from this layout).
            text = postprocess::spoken::layout(&text, ctx.app_category);
            if text.trim().is_empty() {
                r.empty_reason = Some("only_filler".into());
                return r;
            }
        }
        r.status = CommitStatus::RawTranscript;
        if abort.load(Ordering::SeqCst) {
            return CommitResult::error("cancelled");
        }
        let run_llm = cleanup::should_run(&s.cfg, ctx.app_category) && !snippet;
        if let (true, Some(cl)) = (run_llm, &self.cleaner) {
            let style = s.cfg.styles.for_category(ctx.app_category);
            let prompt =
                cleanup::build_prompt(&text, s.cfg.cleanup_level, style, &ctx, &s.cfg.dictionary);
            // The edit-distance guard only runs when a shell asks for it;
            // the default guard is `guard::check` (support + coverage).
            let max_div = s.cfg.divergence_threshold.unwrap_or(f64::INFINITY);
            let t = Instant::now();
            match cleanup::run_cleanup(
                cl.clone(),
                prompt,
                &text,
                Duration::from_millis(s.cfg.llm_timeout_ms),
                max_div,
            ) {
                CleanOutcome::Formatted(f) => {
                    text = f;
                    r.status = CommitStatus::Formatted;
                }
                CleanOutcome::Fallback(why) => r.fallback_reason = Some(why.as_str()),
            }
            r.timings.llm_ms = t.elapsed().as_millis() as u64;
        }
        let t = Instant::now();
        let post = postprocess::apply_with(&text, &r.asr_text, &s.cfg, &ctx, &s.matcher);
        r.timings.postprocess_ms = t.elapsed().as_millis() as u64;
        r.segments = paste::segments(&post.text, post.press_enter);
        r.plain_text = paste::plain_text(&post.text);
        r.final_text = post.text;
        r.press_enter = post.press_enter;
        r
    }
}
