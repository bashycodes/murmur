//! Session engine: handle, ring buffer, core-owned worker thread, commit
//! pipeline (ASR -> optional LLM -> deterministic post-processing).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::asr::{build_initial_prompt, AbortFlag, AsrEngine, AsrRequest};
use crate::cleanup::{self, CleanOutcome, Cleaner};
use crate::config::{CleanupLevel, FieldContext, ModelConfig, SessionConfig, SAMPLE_RATE};
use crate::dictionary::Dictionary;
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

enum Ctrl {
    Start {
        id: u64,
        cfg: Box<SessionConfig>,
        consumer: rtrb::Consumer<i16>,
    },
    Commit {
        id: u64,
        reply: mpsc::Sender<CommitResult>,
    },
    Cancel {
        id: u64,
    },
    Shutdown,
}

struct Producer {
    id: u64,
    prod: rtrb::Producer<i16>,
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
                    .map_err(|e| CoreError::Load(e.to_string()))?;
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
            AsrBackend::Parakeet => Arc::new(
                crate::parakeet::ParakeetAsr::load(&cfg.asr_path, cfg.threads)
                    .map_err(|e| CoreError::Load(e.to_string()))?,
            ),
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
        *self.shared.partial.lock().unwrap() = None;
        *self.shared.dictionary.lock().unwrap() = cfg.dictionary.clone();
        self.shared.dropped.store(0, Ordering::Relaxed);
        self.shared.stop_partial.store(false, Ordering::Relaxed);
        let mut slot = self.shared.producer.lock().unwrap();
        *slot = Some(Producer { id, prod });
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

    /// Latest live-preview transcript, if partials are enabled.
    pub fn partial(&self) -> Option<String> {
        self.shared.partial.lock().unwrap().clone()
    }

    /// Stop capture and run the full pipeline. Blocks until the result is ready.
    pub fn commit(&self) -> CommitResult {
        let t0 = Instant::now();
        let Some(Producer { id, .. }) = self.shared.producer.lock().unwrap().take() else {
            return CommitResult::error("no active session");
        };
        self.shared.stop_partial.store(true, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        if self.ctrl.send(Ctrl::Commit { id, reply: tx }).is_err() {
            return CommitResult::error("core worker is gone");
        }
        let mut r = rx
            .recv()
            .unwrap_or_else(|_| CommitResult::error("core worker is gone"));
        r.timings.total_ms = t0.elapsed().as_millis() as u64;
        r.timings.dropped_samples = self.shared.dropped.load(Ordering::Relaxed);
        r
    }

    /// Abort the session; audio is discarded.
    pub fn cancel(&self) {
        if let Some(Producer { id, .. }) = self.shared.producer.lock().unwrap().take() {
            self.shared.stop_partial.store(true, Ordering::Relaxed);
            let _ = self.ctrl.send(Ctrl::Cancel { id });
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
                Ok(Ctrl::Commit { id, reply }) => {
                    let r = match self.session.take() {
                        Some(mut s) if s.id == id => {
                            if let Some(cl) = &self.cleaner {
                                cl.cancel_prefill();
                            }
                            s.drain();
                            self.finalize(s)
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

    fn asr_request(&self, cfg: &SessionConfig) -> AsrRequest {
        let ctx = self.shared.context.lock().unwrap();
        AsrRequest {
            initial_prompt: build_initial_prompt(&cfg.dictionary, &ctx.screen_words),
            language: cfg.forced_language().map(str::to_string),
            abort: AbortFlag::default(),
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
        let mut req = self.asr_request(&s.cfg);
        req.abort = AbortFlag(Some(self.shared.stop_partial.clone()));
        if let Ok(out) = self.asr.transcribe(window, &req) {
            let t = out.text.trim();
            if !t.is_empty() {
                *self.shared.partial.lock().unwrap() = Some(t.to_string());
            }
        }
    }

    fn finalize(&self, s: Session) -> CommitResult {
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
        let req = self.asr_request(&s.cfg);
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
        let mut ctx = self.shared.context.lock().unwrap().clone();
        ctx.app_category = postprocess::effective_category(s.cfg.platform, &ctx);
        let mut text = r.asr_text.clone();
        // A whole-utterance snippet trigger is pasted as its expansion; the
        // LLM must not see (or reword) it (03 §7.1).
        let snippet = s.matcher.whole_snippet(&r.asr_text).is_some();
        if s.cfg.spoken_cleanup && s.cfg.cleanup_level != CleanupLevel::None && !snippet {
            text = postprocess::spoken::spoken_cleanup(&text);
            if text.trim().is_empty() {
                r.empty_reason = Some("only_filler".into());
                return r;
            }
        }
        r.status = CommitStatus::RawTranscript;
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
