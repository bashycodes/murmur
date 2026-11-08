//! Session engine: handle, ring buffer, core-owned worker thread, commit
//! pipeline (ASR -> optional LLM -> deterministic post-processing).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::asr::{
    build_initial_prompt, dedupe_terms, prior_words, AbortFlag, AsrEngine, AsrError, AsrOutput,
    AsrPurpose, AsrRequest,
};
use crate::cleanup::{self, CleanOutcome, Cleaner};
use crate::config::{
    english_rules, primary_language, CleanupLevel, FieldContext, LanguageModel, ModelConfig,
    SessionConfig, SAMPLE_RATE,
};
use crate::dictionary::Dictionary;
use crate::endpoint::{EndpointState, Endpointer};
use crate::learn::{self, Candidate};
use crate::lid::{self, LanguageId};
use crate::llm_keep::{self, KeptLlm, LlmKey, LlmSlot};
use crate::load_timings::{ms_since, LoadBackend, LoadTimings, ModelKind, ModelTiming, Phase};
use crate::paste::{self, PasteSegment};
use crate::postprocess::{self, DictMatcher};
use crate::segment::{self, SegmentParams, Segmenter};
use crate::{hallucination, vad};

/// Partials decode at most this much trailing audio. 30 s (Whisper's
/// window) made every preview cost about as much as the final decode, and
/// `commit` waits for the one in flight (neither whisper's encoder nor a
/// sherpa decode can be interrupted): field data showed up to 0.9 s of
/// `total − asr`. 12 s bounds that wait (proposal Phase 0). Older audio is
/// shown as a leading "… ".
const PARTIAL_WINDOW: usize = SAMPLE_RATE * 12;
/// Preview prefix when the window does not reach back to the start.
const PARTIAL_ELLIPSIS: &str = "… ";
/// No new preview once this much trailing silence follows speech that the
/// last preview already covered: the user is probably about to release,
/// and the decode would only delay the commit.
const PARTIAL_SKIP_SILENCE_MS: u64 = 400;
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
    /// Time `commit` waited for the worker to finish what it was doing (a
    /// live-preview or segment decode in flight) before its own work began.
    /// Included in `total_ms`, not in `asr_ms`. Omitted when 0. 2026-10-23.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wait_ms: u64,
    /// Pause-segmented decoding (D22): pieces that went to ASR (closed
    /// segments + tail). 0 = segmentation off. Omitted when 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub segments: u64,
    /// D22: ASR time spent while recording on pieces whose text was used
    /// (not part of `asr_ms`, which is ASR time inside `commit`). Omitted
    /// when 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pre_asr_ms: u64,
    /// VAD speech heard in the whole dictation (sum over pieces when
    /// segmented); `Some(0)` = VAD found none, `None` = VAD off. With
    /// `empty_reason: blank_asr` it tells "the ASR heard speech and
    /// returned nothing" from "the VAD failed open on noise". 2026-10-24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech_ms: Option<u64>,
    /// Peak sample level of the session audio in dBFS, rounded
    /// ([`PEAK_DBFS_FLOOR`] for digital silence); `None` = no audio.
    /// 2026-10-24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_dbfs: Option<i32>,
    /// H6 (D34): time spent identifying the spoken language of an
    /// auto-detect dictation (while recording, or at commit for a short
    /// one). Omitted when 0 (language ID off).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lid_ms: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// `Timings::peak_dbfs` of all-zero audio.
pub const PEAK_DBFS_FLOOR: i32 = -120;

/// Peak level of `pcm` (samples in -1..1) in whole dBFS.
fn peak_dbfs(pcm: &[f32]) -> i32 {
    let peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak <= 0.0 {
        return PEAK_DBFS_FLOOR;
    }
    ((20.0 * peak.log10()).round() as i32).clamp(PEAK_DBFS_FLOOR, 0)
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
    /// The LLM step of this commit, when it ran (engine `local`/`cloud`,
    /// model, cost, generation id, TTFT; D28 debug-log fields). Omitted
    /// while `None` (LLM off or skipped). Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<cleanup::CleanUsage>,
    /// D30: what produced `final_text`: `"cloud"` (AI cleanup), `"local"`
    /// (fallback rules, then the on-device LLM) or `"rules"` (fallback
    /// rules alone). Set only when AI cleanup was tried (a cloud cleaner
    /// is active), so results without cloud cleanup serialise as before.
    /// Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_route: Option<String>,
    /// D30: why AI cleanup gave no answer (`llm_timeout`, `llm_empty`,
    /// `llm_error: <code>`) when it was tried and the fallback pipeline
    /// ran. Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_fallback: Option<String>,
    /// Why AI cleanup fell back, in detail: the timeout, the error or the
    /// empty answer, for the debug log (2026-10-27). Set only with
    /// `cloud_fallback`. Additive. (Until D59 a `rejected_text` field
    /// carried the answer an output guard rejected; there is no guard now.)
    #[serde(
        default,
        rename = "guard_detail",
        skip_serializing_if = "Option::is_none"
    )]
    pub fallback_detail: Option<String>,
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
    /// Wall time of `Router::commit` on the client (release → result, so
    /// for a server result it includes the upload tail, the network and
    /// the server's queue). Set by `flow_router::Router`; `FlowCore` never
    /// sets it. 2026-10-23.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ms: Option<u32>,
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
    /// D28: engine of the LLM step (`"local"`, `"cloud"`; `"server"` for a
    /// server-cleaned result), its model and cloud cost. Set by the router
    /// from `CommitResult::llm`. Omitted while `None`. Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_cost_usd: Option<Usd>,
    /// D30: `CommitResult::cleanup_route` / `cloud_fallback` (which
    /// cleanup produced the text, and why AI cleanup's answer was not
    /// used), for the live route label and the debug log. Omitted while
    /// `None` (no cloud cleanup). Additive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_route: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud_fallback: Option<String>,
    /// `CommitResult::fallback_detail`, for the debug log only (never the
    /// route label). Omitted while `None`. Additive.
    #[serde(rename = "guard_detail", skip_serializing_if = "Option::is_none")]
    pub fallback_detail: Option<String>,
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
            llm: None,
            cleanup_route: None,
            cloud_fallback: None,
            fallback_detail: None,
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

/// The ASR engines of a core (D33): the default one, plus one per
/// language from `ModelConfig::languages`.
#[derive(Clone)]
struct AsrRoutes {
    default: Arc<dyn AsrEngine>,
    /// `(language key, engine)`; keys as configured (`"he"`).
    by_language: Vec<(String, Arc<dyn AsrEngine>)>,
    /// H6 (D34): identifies an auto-detect dictation's language so it can
    /// take a `by_language` route. `None` = auto-detect uses `default`.
    lid: Option<Arc<dyn LanguageId>>,
}

impl AsrRoutes {
    /// The engine for a session, and the language key when it is a
    /// language route: the forced language's entry, else the default
    /// engine (auto-detect included).
    fn pick(&self, cfg: &SessionConfig) -> (Arc<dyn AsrEngine>, Option<String>) {
        match cfg.forced_language().and_then(|l| self.route(l)) {
            Some((k, e)) => (e.clone(), Some(k.clone())),
            None => (self.default.clone(), None),
        }
    }

    /// The `by_language` entry for `language` (matched by primary code).
    fn route(&self, language: &str) -> Option<&(String, Arc<dyn AsrEngine>)> {
        let w = primary_language(language)?;
        self.by_language
            .iter()
            .find(|(k, _)| primary_language(k).as_deref() == Some(w.as_str()))
    }

    /// H6: true when a session must have its language identified before
    /// decoding: auto-detect, with an identifier and a language route.
    fn needs_lid(&self, cfg: &SessionConfig) -> bool {
        self.lid.is_some() && !self.by_language.is_empty() && cfg.forced_language().is_none()
    }

    /// The languages an identifier chooses between: the session's own
    /// list when it names several, else the default model's language
    /// ([`DEFAULT_LANGUAGE`]) and every route.
    fn lid_candidates(&self, cfg: &SessionConfig) -> Vec<String> {
        let mut c: Vec<String> = Vec::new();
        let listed: Vec<String> = cfg
            .languages
            .iter()
            .filter_map(|l| primary_language(l))
            .collect();
        let src = if listed.len() >= 2 {
            listed
        } else {
            std::iter::once(DEFAULT_LANGUAGE.to_string())
                .chain(
                    self.by_language
                        .iter()
                        .filter_map(|(k, _)| primary_language(k)),
                )
                .collect()
        };
        for l in src {
            if !c.contains(&l) {
                c.push(l);
            }
        }
        c
    }
}

/// The language of the default model as far as language ID is concerned
/// (D34): auto-detect is English-first (D33 decision 6).
const DEFAULT_LANGUAGE: &str = "en";

/// Phases a sherpa engine reported, as [`Phase`]s.
#[cfg(feature = "parakeet")]
fn sherpa_phases(p: &[flow_asr_sherpa::LoadPhase]) -> Vec<Phase> {
    p.iter()
        .map(|p| Phase {
            name: p.name,
            ms: p.ms,
        })
        .collect()
}

/// A loaded engine and how long its load took.
type WithTiming<T> = (T, ModelTiming);

/// Build the engine for one model. `language` is the language the model
/// is configured for (`None` = the default model), reported by engines
/// that cannot tell (Parakeet, sherpa online). Adding a backend = one arm here (see
/// `config::AsrBackend`).
// `language` is used by the sherpa backends only, `threads` by the model backends.
#[cfg_attr(not(feature = "parakeet"), allow(unused_variables))]
fn load_asr(
    m: &LanguageModel,
    cfg: &ModelConfig,
    language: Option<&str>,
) -> Result<WithTiming<Arc<dyn AsrEngine>>, CoreError> {
    use crate::config::AsrBackend;
    let t = Instant::now();
    let threads = m.threads.unwrap_or(cfg.threads).max(1);
    let (engine, backend, phases): (Arc<dyn AsrEngine>, LoadBackend, Vec<Phase>) = match &m
        .asr_backend
    {
        AsrBackend::Mock { text } => (
            Arc::new(crate::asr::MockAsr::new(text)),
            LoadBackend::Mock,
            Vec::new(),
        ),
        #[cfg(feature = "whisper")]
        AsrBackend::Whisper => {
            require_file(&m.asr_path)?;
            let mut w = crate::whisper::WhisperEngine::load(&m.asr_path, threads)
                .map_err(|e| CoreError::Load(e.to_string()))?
                .with_fit_ctx(cfg.whisper_fit_ctx);
            if let Some(v) = cfg.vad_model_path.as_deref().filter(|v| !v.is_empty()) {
                require_file(v)?;
                w = w.with_silero_vad(v);
            }
            (Arc::new(w), LoadBackend::Whisper, Vec::new())
        }
        #[cfg(not(feature = "whisper"))]
        AsrBackend::Whisper => {
            return Err(CoreError::Load(
                "built without the `whisper` feature".into(),
            ))
        }
        #[cfg(feature = "parakeet")]
        AsrBackend::Parakeet => {
            require_dir(&m.asr_path)?;
            let p =
                crate::parakeet::ParakeetAsr::load_with(&m.asr_path, threads, m.parakeet_decoding)
                    .map_err(|e| CoreError::Load(e.to_string()))?
                    .with_language(language.unwrap_or("en"));
            let phases = sherpa_phases(p.load_phases());
            (Arc::new(p), LoadBackend::Parakeet, phases)
        }
        #[cfg(feature = "parakeet")]
        AsrBackend::SherpaOnline => {
            require_dir(&m.asr_path)?;
            let z = crate::sherpa_online::SherpaOnlineAsr::load(&m.asr_path, threads, language)
                .map_err(|e| CoreError::Load(e.to_string()))?;
            let phases = sherpa_phases(z.load_phases());
            (Arc::new(z), LoadBackend::SherpaOnline, phases)
        }
        #[cfg(not(feature = "parakeet"))]
        AsrBackend::Parakeet | AsrBackend::SherpaOnline => {
            return Err(CoreError::Load(
                "built without the `parakeet` feature".into(),
            ))
        }
    };
    let timing = ModelTiming {
        kind: ModelKind::Asr,
        language: language.map(str::to_string),
        backend,
        ms: ms_since(t),
        phases,
        reused: false,
    };
    Ok((engine, timing))
}

/// The language identifier of `cfg` ([`ModelConfig::lid_model`]): `None`
/// unless `lid_path` is set and a language model is configured (D34).
fn load_lid(cfg: &ModelConfig) -> Result<Option<WithTiming<Arc<dyn LanguageId>>>, CoreError> {
    let Some(path) = cfg.lid_model() else {
        return Ok(None);
    };
    #[cfg(feature = "whisper")]
    {
        require_file(path)?;
        let t = Instant::now();
        let lid = crate::lid::WhisperLid::load(path, cfg.threads, true).map_err(CoreError::Load)?;
        let timing = ModelTiming {
            kind: ModelKind::Lid,
            language: None,
            backend: LoadBackend::Whisper,
            ms: ms_since(t),
            phases: Vec::new(),
            reused: false,
        };
        Ok(Some((Arc::new(lid), timing)))
    }
    #[cfg(not(feature = "whisper"))]
    {
        let _ = path;
        Err(CoreError::Load(
            "lid_path set but built without the `whisper` feature".into(),
        ))
    }
}

/// One [`FlowCore`] load in progress: its timings, and the LLM a previous
/// engine kept for it ([`crate::llm_keep`]). That LLM is taken out first,
/// so a kept model that does not match is freed before anything loads; a
/// matching one goes back into the slot if the load fails before
/// [`Load::finish`] (a speech model that did not load), so a retry still
/// reuses it.
struct Load<'s> {
    slot: &'s LlmSlot,
    started: Instant,
    models: Vec<ModelTiming>,
    kept_llm: Option<KeptLlm>,
}

impl<'s> Load<'s> {
    fn start(slot: &'s LlmSlot, cfg: &ModelConfig) -> Self {
        Self {
            slot,
            started: Instant::now(),
            models: Vec::new(),
            kept_llm: slot.take(LlmKey::of(cfg).as_ref()),
        }
    }

    /// Record a loaded model's timing; returns the engine.
    fn add<T>(&mut self, (engine, timing): WithTiming<T>) -> T {
        self.models.push(timing);
        engine
    }

    /// The LLM (kept or loaded), then the core.
    fn finish(mut self, cfg: &ModelConfig, routes: AsrRoutes) -> Result<FlowCore, CoreError> {
        let llm = match LlmKey::of(cfg) {
            None => None,
            Some(key) => Some(match self.kept_llm.take() {
                Some(kept) => {
                    self.models.push(llm_timing(0, Vec::new(), true));
                    kept
                }
                None => {
                    let t = Instant::now();
                    let (c, phases) = load_llm(cfg, &key.path)?;
                    self.models.push(llm_timing(ms_since(t), phases, false));
                    (key, c)
                }
            }),
        };
        let mut core = FlowCore::from_routes(routes, llm.as_ref().map(|(_, c)| c.clone()));
        core.llm_key = llm.map(|(k, _)| k);
        core.load_timings = LoadTimings {
            total_ms: ms_since(self.started),
            models: std::mem::take(&mut self.models),
        };
        Ok(core)
    }
}

impl Drop for Load<'_> {
    fn drop(&mut self) {
        if let Some(kept) = self.kept_llm.take() {
            self.slot.keep(kept);
        }
    }
}

fn llm_timing(ms: u64, phases: Vec<Phase>, reused: bool) -> ModelTiming {
    ModelTiming {
        kind: ModelKind::Llm,
        language: None,
        backend: LoadBackend::Llama,
        ms,
        phases,
        reused,
    }
}

/// Load the on-device LLM at `path`, with the plugin's own `model` and
/// `warm` (static prompt prefill) times.
#[cfg(feature = "llm")]
fn load_llm(cfg: &ModelConfig, path: &str) -> Result<(Arc<dyn Cleaner>, Vec<Phase>), CoreError> {
    use crate::llama::{LlamaCleaner, LlamaLoadError};
    require_file(path)?;
    let c = LlamaCleaner::load(path, cfg.threads, cfg.llm_lib.as_deref()).map_err(|e| match e {
        LlamaLoadError::PluginNotFound { tried } => CoreError::LlmLibNotFound { tried },
        e => CoreError::Load(e.to_string()),
    })?;
    let info: serde_json::Value = serde_json::from_str(&c.info).unwrap_or_default();
    let phases = [("model", "load_ms"), ("warm", "warm_ms")]
        .into_iter()
        .filter_map(|(name, field)| {
            Some(Phase {
                name,
                ms: info.get(field)?.as_u64()?,
            })
        })
        .collect();
    Ok((Arc::new(c), phases))
}

/// Asking for cleanup from a build that cannot do it is a config error, not
/// something to drop silently.
#[cfg(not(feature = "llm"))]
fn load_llm(_: &ModelConfig, _: &str) -> Result<(Arc<dyn Cleaner>, Vec<Phase>), CoreError> {
    Err(CoreError::Load(
        "llm_path set but built without the `llm` feature".into(),
    ))
}

enum Ctrl {
    Start {
        id: u64,
        cfg: Box<SessionConfig>,
        consumer: rtrb::Consumer<i16>,
        /// Set by `cancel()` / a newer `start_session`: stops this
        /// session's segment decodes (D22).
        abort: Arc<AtomicBool>,
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
        /// When `commit()` sent it (`timings.wait_ms`).
        sent: Instant,
    },
    Cancel {
        id: u64,
    },
    /// Replace the cleaner from the next commit on (D28 `set_cleaner`).
    SetCleaner(Option<Arc<dyn Cleaner>>),
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
    /// Abort flag of the latest session's segment decodes (D22). Not set by
    /// `commit`: a segment decode in flight at release is work the commit
    /// needs anyway.
    session_abort: Mutex<Arc<AtomicBool>>,
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
    /// The cleaner the core was built with (the on-device LLM).
    cleaner_info: Option<Arc<dyn Cleaner>>,
    /// The cleaner commits use: `cleaner_info` unless `set_cleaner` replaced it.
    active_cleaner: Mutex<Option<Arc<dyn Cleaner>>>,
    load_timings: LoadTimings,
    /// The load `cleaner_info` came from, when it is a loaded on-device
    /// LLM ([`FlowCore::keep_llm`]).
    llm_key: Option<LlmKey>,
    ctrl: mpsc::Sender<Ctrl>,
    worker: Option<JoinHandle<()>>,
}

impl FlowCore {
    /// Load models and keep them warm until [`FlowCore::unload`] / drop:
    /// the default ASR model, every model in `cfg.languages` (D33) and the
    /// LLM. Sessions whose forced language has a map entry decode on that
    /// model, all others on the default one.
    pub fn load_models(cfg: &ModelConfig) -> Result<Self, CoreError> {
        let mut load = Load::start(&llm_keep::SLOT, cfg);
        let mut by_language = Vec::with_capacity(cfg.languages.len());
        for (k, m) in &cfg.languages {
            by_language.push((k.clone(), load.add(load_asr(m, cfg, Some(k))?)));
        }
        let default = load.add(load_asr(&cfg.default_model(), cfg, None)?);
        let lid = load_lid(cfg)?.map(|l| load.add(l));
        let routes = AsrRoutes {
            default,
            by_language,
            lid,
        };
        load.finish(cfg, routes)
    }

    /// Like [`FlowCore::load_models`] but with one ASR model in memory
    /// (D33 decision 4, for RAM-tight shells such as Android): the model
    /// that serves `language` ([`ModelConfig::language_route`]: the map
    /// entry, else the default model), plus the LLM. Every session then
    /// decodes on that model, whatever its language, so the shell reloads
    /// when the user switches to a language that routes elsewhere (compare
    /// `language_route` of the old and new language). Before freeing the
    /// old core it can [`FlowCore::keep_llm`], which this load then reuses
    /// instead of loading the LLM again.
    pub fn load_models_for(cfg: &ModelConfig, language: Option<&str>) -> Result<Self, CoreError> {
        Self::load_models_for_in(&llm_keep::SLOT, cfg, language)
    }

    fn load_models_for_in(
        slot: &LlmSlot,
        cfg: &ModelConfig,
        language: Option<&str>,
    ) -> Result<Self, CoreError> {
        let mut load = Load::start(slot, cfg);
        let asr = load.add(match cfg.language_route(language) {
            Some(k) => load_asr(&cfg.languages[k], cfg, Some(k))?,
            None => load_asr(&cfg.default_model(), cfg, None)?,
        });
        load.finish(
            cfg,
            AsrRoutes {
                default: asr,
                by_language: Vec::new(),
                lid: None,
            },
        )
    }

    /// Load 3: hold this core's on-device LLM for the next load that names
    /// the same model, thread count and plugin ([`crate::llm_keep`]), so a
    /// language switch reloads only the speech model. Call it before
    /// freeing this core. Returns whether there was an LLM to keep (none
    /// for cores built from engines, or without `llm_path`).
    pub fn keep_llm(&self) -> bool {
        self.keep_llm_in(&llm_keep::SLOT)
    }

    fn keep_llm_in(&self, slot: &LlmSlot) -> bool {
        let (Some(key), Some(c)) = (&self.llm_key, &self.cleaner_info) else {
            return false;
        };
        slot.keep((key.clone(), c.clone()));
        true
    }

    /// How long this core's load took, per model and phase (empty for
    /// cores built from engines).
    pub fn load_timings(&self) -> &LoadTimings {
        &self.load_timings
    }

    /// Build a core around caller-supplied engines (tests, custom backends).
    pub fn from_engines(asr: Arc<dyn AsrEngine>, cleaner: Option<Arc<dyn Cleaner>>) -> Self {
        Self::from_language_engines(asr, Vec::new(), cleaner)
    }

    /// [`FlowCore::from_engines`] with per-language engines (D33): a
    /// session whose forced language matches a key (`"he"`; matched like
    /// [`ModelConfig::language_route`]) decodes on that engine, every other
    /// session on `default`.
    pub fn from_language_engines(
        default: Arc<dyn AsrEngine>,
        by_language: Vec<(String, Arc<dyn AsrEngine>)>,
        cleaner: Option<Arc<dyn Cleaner>>,
    ) -> Self {
        Self::from_language_engines_with_lid(default, by_language, None, cleaner)
    }

    /// [`FlowCore::from_language_engines`] with a language identifier
    /// (H6, D34): an auto-detect session with `lid` and at least one
    /// language engine has its first [`crate::lid::LID_SAMPLES`] of speech
    /// identified and decodes on that language's engine when there is one.
    pub fn from_language_engines_with_lid(
        default: Arc<dyn AsrEngine>,
        by_language: Vec<(String, Arc<dyn AsrEngine>)>,
        lid: Option<Arc<dyn LanguageId>>,
        cleaner: Option<Arc<dyn Cleaner>>,
    ) -> Self {
        Self::from_routes(
            AsrRoutes {
                default,
                by_language,
                lid,
            },
            cleaner,
        )
    }

    fn from_routes(routes: AsrRoutes, cleaner: Option<Arc<dyn Cleaner>>) -> Self {
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
            session_abort: Mutex::new(Arc::new(AtomicBool::new(false))),
        });
        let (tx, rx) = mpsc::channel();
        let ws = shared.clone();
        let cleaner_info = cleaner.clone();
        let worker = std::thread::Builder::new()
            .name("flow-core-worker".into())
            .spawn(move || {
                Worker {
                    shared: ws,
                    routes,
                    cleaner,
                    session: None,
                    dict_cache: None,
                }
                .run(rx)
            })
            .expect("spawn worker");
        Self {
            shared,
            active_cleaner: Mutex::new(cleaner_info.clone()),
            cleaner_info,
            load_timings: LoadTimings::default(),
            llm_key: None,
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
        let abort = Arc::new(AtomicBool::new(false));
        std::mem::replace(
            &mut *self.shared.session_abort.lock().unwrap(),
            abort.clone(),
        )
        .store(true, Ordering::SeqCst);
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
                abort,
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
        self.active_cleaner().and_then(|c| c.info_json())
    }

    /// Replace the cleaner used by commits (D28: the router installs a
    /// `RoutedCleaner` of cloud + on-device LLM). `None` restores the one
    /// the core was built with. Applies from the next commit; a commit in
    /// progress keeps its cleaner. Additive.
    pub fn set_cleaner(&self, cleaner: Option<Arc<dyn Cleaner>>) {
        let eff = cleaner.or_else(|| self.cleaner_info.clone());
        *self.active_cleaner.lock().unwrap() = eff.clone();
        let _ = self.ctrl.send(Ctrl::SetCleaner(eff));
    }

    /// The cleaner commits use now ([`FlowCore::set_cleaner`], else the
    /// built-in one). Additive (D28).
    pub fn active_cleaner(&self) -> Option<Arc<dyn Cleaner>> {
        self.active_cleaner.lock().unwrap().clone()
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
                sent: Instant::now(),
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
        // Stops a segment decode in flight (an open session's, or the one
        // a running commit is queued behind).
        self.shared
            .session_abort
            .lock()
            .unwrap()
            .store(true, Ordering::SeqCst);
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
    /// The engine this session decodes on (D33 routing).
    asr: Arc<dyn AsrEngine>,
    /// The language key of a per-language route (`None` = default engine).
    route_language: Option<String>,
    /// H6 (D34): the language is not identified yet; segment decodes and
    /// previews wait for it (they would run on the wrong engine).
    lid_pending: bool,
    /// Buffer length at the last language-ID speech check.
    lid_checked: usize,
    /// Time spent on language ID (`Timings::lid_ms`).
    lid_ms: u64,
    consumer: rtrb::Consumer<i16>,
    buf: Vec<f32>,
    truncated: bool,
    last_partial_len: usize,
    /// End (buffer length) of the audio the last preview decode covered.
    partial_upto: usize,
    /// Streaming VAD over `buf` while previews are on (skip previews in
    /// trailing silence); fed up to `buf.len()`. `None` with `seg`, which
    /// has its own.
    track: Option<vad::StreamVad>,
    /// D22 pause segmenter, fed up to `buf.len()` (`None` = off).
    seg: Option<Segmenter>,
    /// Pieces decoded while recording, in order (`done[k]` ends at cut k).
    done: Vec<SegOut>,
    /// Boundary decodes, in order (`bridges[k]` spans cut k): the audio
    /// around a cut, so commit can punctuate across it.
    bridges: Vec<SegOut>,
    /// A segment decode failed: leave the rest to commit.
    seg_failed: bool,
    /// Cancel flag of this session's segment decodes.
    abort: Arc<AtomicBool>,
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
        let (x, y) = chunk.as_slices();
        let from = self.buf.len();
        self.buf
            .extend(x.iter().chain(y).take(room).map(|&s| s as f32 / 32768.0));
        // Free the whole chunk, the samples past `max_samples` too: left in
        // the ring they would fill it and count every later push as
        // dropped (review C18).
        chunk.commit_all();
        if let Some(t) = self.track.as_mut() {
            for &v in &self.buf[from..] {
                t.sample(v);
            }
        }
        if let Some(sg) = self.seg.as_mut() {
            sg.push(&self.buf[from..]);
        }
    }

    /// First sample not covered by a decoded piece.
    fn next(&self) -> usize {
        self.done.last().map_or(0, |d| d.range.end)
    }

    /// True when the preview should not be re-decoded now: speech was
    /// followed by at least `PARTIAL_SKIP_SILENCE_MS` of silence, and the
    /// last preview already covered that speech.
    fn preview_is_current(&self) -> bool {
        let (speech_end, trailing_ms) = match (&self.seg, &self.track) {
            (Some(sg), _) => (sg.speech_end_sample(), sg.trailing_silence_ms()),
            (None, Some(t)) => (
                t.speech_end() as usize * vad::FRAME,
                t.trailing_frames() * (vad::FRAME * 1000 / SAMPLE_RATE) as u64,
            ),
            (None, None) => return false,
        };
        trailing_ms >= PARTIAL_SKIP_SILENCE_MS && self.partial_upto >= speech_end
    }
}

/// How pieces are joined at commit. `Bridge` (the product); the others
/// exist only so `tests/segment_bench.rs` can compare candidates
/// (`FLOW_SEGMENT_JOIN=plain|smooth`, undocumented, bench use only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinMode {
    Bridge,
    Plain,
    Smooth,
}

impl JoinMode {
    fn from_env() -> Self {
        match std::env::var("FLOW_SEGMENT_JOIN").as_deref() {
            Ok("plain") => Self::Plain,
            Ok("smooth") => Self::Smooth,
            _ => Self::Bridge,
        }
    }
}

/// One piece of a segmented dictation (D22).
#[derive(Clone)]
struct SegOut {
    /// Sample range in the session buffer (before VAD trimming).
    range: std::ops::Range<usize>,
    /// The request it was decoded with (abort and purpose cleared), so
    /// commit can tell whether a late context or prompt change needs a
    /// re-decode.
    key: AsrRequest,
    /// `None` = no speech in the piece (ASR skipped).
    out: Option<AsrOutput>,
    /// VAD speech in the piece (`None` with the VAD off).
    speech_ms: Option<u64>,
    /// Samples given to ASR.
    asr_samples: usize,
    asr_ms: u64,
}

impl SegOut {
    /// The piece's text after the per-segment hallucination guard: a piece
    /// that is only a stock phrase on its own evidence is dropped, and a
    /// non-final piece loses a trailing "Thank you." whisper put into the
    /// pause.
    fn kept_text(&self, last: bool, guard: bool) -> &str {
        let Some(o) = &self.out else { return "" };
        let t = o.text.trim();
        if !guard {
            return t;
        }
        let ev = hallucination::Evidence {
            speech_ms: self.speech_ms,
            no_speech_prob: o.no_speech_prob,
        };
        if hallucination::is_hallucination(t, ev) {
            return "";
        }
        if !last
            && o.no_speech_prob
                .is_some_and(|p| p >= hallucination::NO_SPEECH_THOLD)
        {
            if let Some(h) = hallucination::strip_trailing_stock(t) {
                return h;
            }
        }
        t
    }
}

/// Join piece texts with one space, skipping empty ones.
fn join_pieces<'a>(texts: impl IntoIterator<Item = &'a str>) -> String {
    texts
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `prior_text` for the piece after `done`.
fn prior_for(done: &[SegOut], guard: bool) -> Option<String> {
    prior_words(&join_pieces(done.iter().map(|d| d.kept_text(false, guard))))
}

/// Audio before a piece that sets its gain ([`piece_gain`]): the same
/// 30 s the streaming VAD takes its noise floor from.
const GAIN_CONTEXT: usize = SAMPLE_RATE * 30;

/// The gain for a piece's VAD span `span` (D72): [`vad::asr_gain`] of the
/// span when the dictation is quiet, else none (1.0). Quiet means the
/// typical speech of the dictation up to the piece's end (at most
/// [`GAIN_CONTEXT`] before it, [`vad::speech_level`]) is under
/// [`vad::ASR_LEVEL_BELOW`]. The level is a property of the recording, not
/// of one piece: a pause cut can leave a piece that holds only a key click
/// on a near-silent floor, and raised on its own (+40 dB) the recognizer
/// writes a word for it, while a whole-clip decode hears the click at its
/// true level next to the speech. A median, not the peak, so one cough in a
/// quiet dictation does not stop the pieces after it from being raised. A
/// pure function of the audio, so pieces decoded while recording and at
/// commit agree.
fn piece_gain(buf: &[f32], range: std::ops::Range<usize>, span: &[f32]) -> f32 {
    let own = vad::asr_gain(span);
    if own <= 1.0 {
        return own;
    }
    let ctx = &buf[range.start.min(range.end.saturating_sub(GAIN_CONTEXT))..range.end];
    match vad::speech_level(ctx) {
        Some(level) if level >= vad::ASR_LEVEL_BELOW => 1.0,
        _ => own,
    }
}

#[cfg(test)]
mod piece_gain_tests {
    use super::*;

    const S: usize = SAMPLE_RATE;

    fn tone(buf: &mut [f32], from_s: f32, secs: f32, amp: f32) {
        let a = (from_s * S as f32) as usize;
        let b = a + (secs * S as f32) as usize;
        for (i, x) in buf[a..b].iter_mut().enumerate() {
            *x = (i as f32 * 0.05).sin() * amp;
        }
    }

    /// 3 s of normal speech ending just before the last [`GAIN_CONTEXT`]
    /// (30 s) of a quiet piece no longer counts: the piece is raised. Inside
    /// it, the speech makes the dictation normal-level: not raised.
    #[test]
    fn only_the_last_30_s_set_the_level() {
        let quiet = 0.0076; // frame RMS ≈ 0.0054: raised on its own
        let piece = 37 * S..40 * S;
        for (loud_at, raised) in [(6.9, true), (10.0, false)] {
            let mut buf = vec![0f32; 40 * S];
            tone(&mut buf, loud_at, 3.0, 0.1);
            tone(&mut buf, 37.5, 2.0, quiet);
            let sp = vad::detect(&buf[piece.clone()]).unwrap();
            let span = &buf[piece.start + sp.start..piece.start + sp.end];
            let g = piece_gain(&buf, piece.clone(), span);
            assert_eq!(g > 1.0, raised, "loud at {loud_at} s: gain {g}");
            if raised {
                assert_eq!(g, vad::asr_gain(span));
            }
        }
    }
}

/// Decode `buf[range]` (VAD-trimmed when `vad`). `key` is stored; `req`
/// is what the engine gets (with its abort flag and purpose).
fn decode_piece(
    asr: &dyn AsrEngine,
    buf: &[f32],
    range: std::ops::Range<usize>,
    use_vad: bool,
    key: AsrRequest,
    req: &AsrRequest,
) -> Result<SegOut, AsrError> {
    let mut pcm = &buf[range.clone()];
    let mut speech_ms = None;
    if use_vad {
        match vad::detect(pcm) {
            Some(sp) => {
                pcm = &pcm[sp.start..sp.end];
                speech_ms = Some(sp.speech_ms());
            }
            None => {
                return Ok(SegOut {
                    range,
                    key,
                    out: None,
                    speech_ms: Some(0),
                    asr_samples: 0,
                    asr_ms: 0,
                })
            }
        }
    }
    // Quiet speech the VAD passed: raise it above the engines' own gate,
    // by no more than the dictation around it would be raised.
    let level = if use_vad {
        vad::with_gain(pcm, piece_gain(buf, range.clone(), pcm))
    } else {
        pcm.into()
    };
    let t = Instant::now();
    let out = asr.transcribe(&level, req)?;
    Ok(SegOut {
        range,
        key,
        out: Some(out),
        speech_ms,
        asr_samples: pcm.len(),
        asr_ms: t.elapsed().as_millis() as u64,
    })
}

/// The pause segmenter (D22) or preview VAD a session on `asr` uses.
fn listeners(
    cfg: &SessionConfig,
    asr: &dyn AsrEngine,
) -> (Option<Segmenter>, Option<vad::StreamVad>) {
    let seg = SegmentParams::for_session(cfg)
        .filter(|_| asr.segmentable())
        .map(Segmenter::new);
    let track = cfg
        .partial_interval_ms
        .filter(|_| seg.is_none())
        .map(|_| vad::StreamVad::new());
    (seg, track)
}

/// Audio to re-check for speech before language ID gets another look
/// while recording (keeps the VAD scan off every 10 ms tick).
const LID_RECHECK: usize = SAMPLE_RATE / 4;

/// H6 (D34): identify the session's language once [`lid::LID_SAMPLES`] of
/// speech are in (`at_commit`: from whatever speech there is) and move the
/// session to that language's engine when it has a route. True once
/// decided; the session then decodes as if that language were forced.
fn identify_language(routes: &AsrRoutes, s: &mut Session, at_commit: bool) -> bool {
    let Some(id) = routes.lid.as_ref() else {
        s.lid_pending = false;
        return true;
    };
    if !at_commit && s.buf.len() < s.lid_checked + LID_RECHECK {
        return false;
    }
    s.lid_checked = s.buf.len();
    let range = match vad::detect(&s.buf) {
        Some(sp) if at_commit => sp.start..sp.end.min(sp.start + lid::LID_SAMPLES),
        Some(sp) if s.buf.len() >= sp.start + lid::LID_SAMPLES => {
            sp.start..sp.start + lid::LID_SAMPLES
        }
        // No speech at commit: nothing to decode, the default engine says so.
        None if at_commit => {
            s.lid_pending = false;
            return true;
        }
        _ => return false,
    };
    s.lid_pending = false;
    let t = Instant::now();
    let pcm = vad::asr_level(&s.buf[range]);
    let scores = id.scores(&pcm, &routes.lid_candidates(&s.cfg));
    s.lid_ms = (t.elapsed().as_millis() as u64).max(1);
    // An identifier error leaves the dictation on the default engine.
    let Some((key, engine)) = scores
        .ok()
        .and_then(|sc| lid::choose(&sc, DEFAULT_LANGUAGE))
        .and_then(|l| routes.route(&l))
    else {
        return true;
    };
    let was_segmentable = s.asr.segmentable();
    s.asr = engine.clone();
    s.route_language = Some(key.clone());
    s.cfg.languages = vec![key.clone()];
    if engine.segmentable() != was_segmentable {
        // Nothing was decoded yet; rebuild the listeners over the audio so far.
        let (mut seg, mut track) = listeners(&s.cfg, &*s.asr);
        if let Some(sg) = seg.as_mut() {
            sg.push(&s.buf);
        }
        if let Some(t) = track.as_mut() {
            for &v in &s.buf {
                t.sample(v);
            }
        }
        (s.seg, s.track) = (seg, track);
    }
    true
}

struct Worker {
    shared: Arc<Shared>,
    routes: AsrRoutes,
    cleaner: Option<Arc<dyn Cleaner>>,
    session: Option<Session>,
    dict_cache: Option<(Dictionary, Arc<DictMatcher>)>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Ctrl>) {
        loop {
            match rx.recv_timeout(WORKER_TICK) {
                Ok(Ctrl::Start {
                    id,
                    cfg,
                    consumer,
                    abort,
                }) => {
                    let matcher = self.matcher_for(&cfg.dictionary);
                    let (asr, route_language) = self.routes.pick(&cfg);
                    let lid_pending = self.routes.needs_lid(&cfg);
                    let (seg, track) = listeners(&cfg, &*asr);
                    self.session = Some(Session {
                        id,
                        cfg: *cfg,
                        asr,
                        route_language,
                        lid_pending,
                        lid_checked: 0,
                        lid_ms: 0,
                        consumer,
                        buf: Vec::new(),
                        truncated: false,
                        last_partial_len: 0,
                        partial_upto: 0,
                        track,
                        seg,
                        done: Vec::new(),
                        bridges: Vec::new(),
                        seg_failed: false,
                        abort,
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
                    sent,
                }) => {
                    let wait_ms = sent.elapsed().as_millis() as u64;
                    let mut r = match self.session.take() {
                        Some(mut s) if s.id == id => {
                            if let Some(cl) = &self.cleaner {
                                cl.cancel_prefill();
                            }
                            s.drain();
                            if s.lid_pending {
                                identify_language(&self.routes, &mut s, true);
                            }
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
                    r.timings.wait_ms = wait_ms;
                    let _ = reply.send(r);
                }
                Ok(Ctrl::SetCleaner(c)) => self.cleaner = c,
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
        cl.prefill(&cleanup::session_prompt(
            &transcript,
            &s.cfg,
            &ctx,
            cl.prompt_scope(),
            cleanup::PromptVersion::V2,
            s.cfg.forced_language(),
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
            prior_text: None,
        }
    }

    /// Publish `text` as the preview while session `id` is the shown one.
    fn publish_partial(&self, id: u64, text: String) {
        let mut p = self.shared.partial.lock().unwrap();
        // Only while this session is still the shown one (S11).
        if self.shared.current.load(Ordering::SeqCst) == id {
            *p = Some(text);
        }
    }

    /// D22: decode the oldest closed segment not decoded yet. True when it
    /// did (one decode per tick, so a commit waits for at most one).
    fn decode_closed_segment(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        if s.seg_failed {
            return false;
        }
        let Some(&cut) = s.seg.as_ref().and_then(|sg| sg.cuts().get(s.done.len())) else {
            return false;
        };
        let ctx = self.shared.context.lock().unwrap().clone();
        let mut key = Self::asr_request(&s.cfg, &ctx, AsrPurpose::Segment);
        key.prior_text = prior_for(&s.done, s.cfg.hallucination_guard);
        let mut req = key.clone();
        req.abort = AbortFlag(Some(s.abort.clone()));
        let range = s.next()..cut;
        match decode_piece(&*s.asr.clone(), &s.buf, range, s.cfg.vad, key, &req) {
            Ok(o) => s.done.push(o),
            Err(_) => {
                s.seg_failed = true;
                return true;
            }
        }
        let (id, text) = (s.id, Self::done_text(s));
        if !text.is_empty() && !self.shared.stop_partial.load(Ordering::Relaxed) {
            self.publish_partial(id, text);
        }
        true
    }

    /// D22: decode the audio around the oldest cut without a bridge once
    /// `BRIDGE_SAMPLES` follow it. True when it did.
    fn decode_bridge(&mut self) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        if s.seg_failed {
            return false;
        }
        let k = s.bridges.len();
        let Some(cuts) = s.seg.as_ref().map(|sg| sg.cuts()) else {
            return false;
        };
        let Some(&cut) = cuts.get(k) else {
            return false;
        };
        if s.buf.len() < cut + segment::BRIDGE_SAMPLES && cuts.get(k + 1).is_none() {
            return false;
        }
        let lo = if k == 0 { 0 } else { cuts[k - 1] };
        let hi = cuts.get(k + 1).copied().unwrap_or(s.buf.len());
        let range = segment::bridge_range(cut, lo, hi);
        let ctx = self.shared.context.lock().unwrap().clone();
        let key = Self::asr_request(&s.cfg, &ctx, AsrPurpose::Segment);
        let mut req = key.clone();
        req.abort = AbortFlag(Some(s.abort.clone()));
        match decode_piece(&*s.asr.clone(), &s.buf, range, s.cfg.vad, key, &req) {
            Ok(o) => s.bridges.push(o),
            Err(_) => s.seg_failed = true,
        }
        true
    }

    fn done_text(s: &Session) -> String {
        join_pieces(
            s.done
                .iter()
                .map(|d| d.kept_text(false, s.cfg.hallucination_guard)),
        )
    }

    fn tick(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        s.drain();
        // H6: nothing is decoded before the language is known.
        if s.lid_pending && !identify_language(&self.routes, s, false) {
            return;
        }
        // D22: closed segments first (their text is part of the result);
        // previews only cover the open one.
        if self.decode_closed_segment() || self.decode_bridge() {
            return;
        }
        let s = self.session.as_mut().unwrap();
        let Some(ms) = s.cfg.partial_interval_ms else {
            return;
        };
        let step = ms as usize * SAMPLE_RATE / 1000;
        if s.buf.len() < s.last_partial_len + step.max(1) {
            return;
        }
        s.last_partial_len = s.buf.len();
        let open = s.next();
        let start = s.buf.len().saturating_sub(PARTIAL_WINDOW).max(open);
        if self.shared.stop_partial.load(Ordering::Relaxed) || s.preview_is_current() {
            return;
        }
        s.partial_upto = s.buf.len();
        let s = self.session.as_ref().unwrap();
        let mut window = &s.buf[start..];
        if s.cfg.vad {
            // No speech yet: nothing to preview, and no CPU spent on it.
            let Some(sp) = vad::detect(window) else {
                return;
            };
            window = &window[sp.start..sp.end];
        }
        let window = if s.cfg.vad {
            vad::asr_level(window)
        } else {
            window.into()
        };
        let ctx = self.shared.context.lock().unwrap().clone();
        let mut req = Self::asr_request(&s.cfg, &ctx, AsrPurpose::Partial);
        req.abort = AbortFlag(Some(self.shared.stop_partial.clone()));
        if s.seg.is_some() {
            req.prior_text = prior_for(&s.done, s.cfg.hallucination_guard);
        }
        if let Ok(out) = s.asr.transcribe(&window, &req) {
            let t = out.text.trim();
            if !t.is_empty() {
                let t = if start > open {
                    format!("{PARTIAL_ELLIPSIS}{t}")
                } else {
                    t.to_string()
                };
                let done = Self::done_text(s);
                let t = if done.is_empty() {
                    t
                } else {
                    format!("{done} {t}")
                };
                self.publish_partial(s.id, t);
            }
        }
    }

    fn finalize(&self, s: Session, ctx: &FieldContext, abort: &Arc<AtomicBool>) -> CommitResult {
        let audio_ms = (s.buf.len() * 1000 / SAMPLE_RATE) as u64;
        let mut r = CommitResult::status(CommitStatus::Empty);
        r.truncated = s.truncated;
        r.timings.audio_ms = audio_ms;
        r.timings.lid_ms = s.lid_ms;
        if s.buf.is_empty() {
            r.empty_reason = Some("no_audio".into());
            return r;
        }
        r.timings.peak_dbfs = Some(peak_dbfs(&s.buf));
        if s.seg.is_some() {
            return self.finalize_segmented(s, ctx, abort, r);
        }
        let (pcm, speech_ms) = if s.cfg.vad {
            match vad::detect(&s.buf) {
                Some(sp) => (&s.buf[sp.start..sp.end], Some(sp.speech_ms())),
                None => {
                    r.timings.speech_ms = Some(0);
                    r.empty_reason = Some("no_speech".into());
                    return r;
                }
            }
        } else {
            (&s.buf[..], None)
        };
        r.timings.speech_ms = speech_ms;
        r.timings.asr_audio_ms = (pcm.len() * 1000 / SAMPLE_RATE) as u64;
        let mut req = Self::asr_request(&s.cfg, ctx, AsrPurpose::Final);
        req.abort = AbortFlag(Some(abort.clone()));
        let t = Instant::now();
        // Quiet speech the VAD passed: raise it above the engines' own gate.
        let level = if s.cfg.vad {
            vad::asr_level(pcm)
        } else {
            pcm.into()
        };
        let out = match s.asr.transcribe(&level, &req) {
            Ok(o) => o,
            Err(e) => {
                let mut e = CommitResult::error(e.to_string());
                e.timings = r.timings;
                return e;
            }
        };
        r.timings.asr_ms = t.elapsed().as_millis() as u64;
        r.language = out.language.or_else(|| s.route_language.clone());
        r.asr_text = out.text.trim().to_string();
        if r.asr_text.is_empty() {
            r.empty_reason = Some("blank_asr".into());
            return r;
        }
        let evidence = hallucination::Evidence {
            speech_ms,
            no_speech_prob: out.no_speech_prob,
        };
        self.finish(s, ctx, abort, r, evidence)
    }

    /// D22 commit: decode the closed segments still pending (or whose
    /// request changed since, e.g. a late context), then the tail, join,
    /// and run the rest of the pipeline on the joined text.
    fn finalize_segmented(
        &self,
        mut s: Session,
        ctx: &FieldContext,
        abort: &Arc<AtomicBool>,
        mut r: CommitResult,
    ) -> CommitResult {
        let guard = s.cfg.hallucination_guard;
        let mut bounds = vec![0];
        bounds.extend_from_slice(s.seg.as_ref().map_or(&[][..], |sg| sg.cuts()));
        if *bounds.last().unwrap() < s.buf.len() {
            bounds.push(s.buf.len());
        }
        let n = bounds.len() - 1;
        let base = Self::asr_request(&s.cfg, ctx, AsrPurpose::Segment);
        let mut pieces: Vec<SegOut> = Vec::with_capacity(n);
        let done = std::mem::take(&mut s.done);
        let t = Instant::now();
        for k in 0..n {
            let range = bounds[k]..bounds[k + 1];
            let mut key = base.clone();
            key.prior_text = prior_for(&pieces, guard);
            if let Some(d) = done.get(k).filter(|d| d.range == range && d.key == key) {
                r.timings.pre_asr_ms += d.asr_ms;
                pieces.push(d.clone());
                continue;
            }
            if abort.load(Ordering::SeqCst) {
                return CommitResult::error("cancelled");
            }
            let mut req = key.clone();
            req.abort = AbortFlag(Some(abort.clone()));
            if k + 1 == n {
                req.purpose = AsrPurpose::Final;
            }
            match decode_piece(&*s.asr, &s.buf, range, s.cfg.vad, key, &req) {
                Ok(o) => pieces.push(o),
                Err(e) => {
                    let mut e = CommitResult::error(e.to_string());
                    e.timings = r.timings;
                    return e;
                }
            }
        }
        let spoken: Vec<&SegOut> = pieces.iter().filter(|p| p.out.is_some()).collect();
        r.timings.segments = spoken.len() as u64;
        r.timings.speech_ms = s
            .cfg
            .vad
            .then(|| pieces.iter().filter_map(|p| p.speech_ms).sum());
        r.timings.asr_audio_ms =
            (pieces.iter().map(|p| p.asr_samples).sum::<usize>() * 1000 / SAMPLE_RATE) as u64;
        if spoken.is_empty() {
            r.empty_reason = Some("no_speech".into());
            return r;
        }
        // The language of the piece with the most audio.
        r.language = spoken
            .iter()
            .max_by_key(|p| p.asr_samples)
            .and_then(|p| p.out.as_ref()?.language.clone())
            .or_else(|| s.route_language.clone());
        let raw = join_pieces(
            spoken
                .iter()
                .map(|p| p.out.as_ref().map_or("", |o| o.text.trim())),
        );
        let mut texts: Vec<String> = pieces
            .iter()
            .enumerate()
            .map(|(k, p)| p.kept_text(k + 1 == n, guard).to_string())
            .collect();
        // Punctuate across each cut: a piece decoded alone ends every pause
        // with a sentence break ("ask not. What your country"). A decode of
        // the audio around the cut decides the words next to it, at least
        // the two at the cut when the wider splice is refused (it also
        // starts every piece with a capital: "familiar with Things", D70).
        let bridges = std::mem::take(&mut s.bridges);
        let join = JoinMode::from_env();
        for k in 0..n.saturating_sub(1) {
            if texts[k].is_empty() || texts[k + 1].is_empty() || join == JoinMode::Plain {
                continue;
            }
            if join == JoinMode::Smooth {
                if let Some((l, rt)) = segment::smooth_join(&texts[k], &texts[k + 1]) {
                    texts[k] = l;
                    texts[k + 1] = rt;
                }
                continue;
            }
            let range = segment::bridge_range(bounds[k + 1], bounds[k], bounds[k + 2]);
            let key = base.clone();
            let b = match bridges.get(k).filter(|b| b.range == range && b.key == key) {
                Some(b) => {
                    r.timings.pre_asr_ms += b.asr_ms;
                    b.clone()
                }
                None => {
                    if abort.load(Ordering::SeqCst) {
                        return CommitResult::error("cancelled");
                    }
                    let mut req = key.clone();
                    req.abort = AbortFlag(Some(abort.clone()));
                    match decode_piece(&*s.asr, &s.buf, range, s.cfg.vad, key, &req) {
                        Ok(b) => b,
                        Err(e) => {
                            let mut e = CommitResult::error(e.to_string());
                            e.timings = r.timings;
                            return e;
                        }
                    }
                }
            };
            let bt = b.out.as_ref().map_or("", |o| o.text.trim());
            let joined = segment::splice(&texts[k], &texts[k + 1], bt)
                .or_else(|| segment::respell_cut(&texts[k], &texts[k + 1], bt));
            if let Some((l, rt)) = joined {
                texts[k] = l;
                texts[k + 1] = rt;
            }
        }
        // Sentence-case repair across cuts is an English rule (D33).
        if join != JoinMode::Plain && english_rules(r.language.as_deref()) {
            segment::capitalize_joins(&mut texts);
        }
        r.timings.asr_ms = t.elapsed().as_millis() as u64;
        r.asr_text = join_pieces(texts.iter().map(String::as_str));
        if r.asr_text.is_empty() {
            if raw.is_empty() {
                r.empty_reason = Some("blank_asr".into());
            } else {
                r.empty_reason = Some("hallucination".into());
                r.asr_text = raw;
            }
            return r;
        }
        let evidence = hallucination::Evidence {
            speech_ms: s
                .cfg
                .vad
                .then(|| pieces.iter().filter_map(|p| p.speech_ms).sum()),
            no_speech_prob: spoken
                .iter()
                .filter_map(|p| p.out.as_ref()?.no_speech_prob)
                .reduce(f32::max),
        };
        self.finish(s, ctx, abort, r, evidence)
    }

    /// Guard, spoken cleanup, LLM and post-processing on `r.asr_text`.
    fn finish(
        &self,
        s: Session,
        ctx: &FieldContext,
        abort: &Arc<AtomicBool>,
        r: CommitResult,
        evidence: hallucination::Evidence,
    ) -> CommitResult {
        finish_text(
            self.cleaner.as_ref(),
            &s.cfg,
            &s.matcher,
            ctx,
            abort,
            r,
            evidence,
        )
    }
}

/// Guard, spoken cleanup, LLM and post-processing on `r.asr_text` (the
/// session's tail of `commit`, and [`refinish`]).
#[allow(clippy::too_many_arguments)]
fn finish_text(
    cleaner: Option<&Arc<dyn Cleaner>>,
    cfg: &SessionConfig,
    matcher: &DictMatcher,
    ctx: &FieldContext,
    abort: &Arc<AtomicBool>,
    mut r: CommitResult,
    evidence: hallucination::Evidence,
) -> CommitResult {
    if cfg.hallucination_guard && hallucination::is_hallucination(&r.asr_text, evidence) {
        r.empty_reason = Some("hallucination".into());
        return r;
    }
    let mut ctx = ctx.clone();
    ctx.app_category = postprocess::effective_category(cfg.platform, &ctx);
    let mut text = r.asr_text.clone();
    // D33: the dictation's language as the engine reported it. The
    // English-only rules (spoken cleanup with ITN, layout, the guard's
    // word lists) run only for English or unknown; the cleanup prompt
    // names any other language.
    let language = r.language.clone();
    let english = english_rules(language.as_deref());
    // A whole-utterance snippet trigger is pasted as its expansion; the
    // LLM must not see (or reword) it (03 §7.1).
    let snippet = matcher.whole_snippet(&r.asr_text).is_some();
    // D30: with a cloud cleaner active, AI cleanup gets the raw
    // transcript first (no rules pre-pass, no skip); the rules and the
    // on-device LLM below run only when it cannot or its answer is
    // rejected. Without one, nothing here changes.
    let mut local_cleaner = cleaner.cloned();
    let ai = (cleanup::should_run(cfg, ctx.app_category) && !snippet)
        .then(|| cleaner.and_then(cleanup::ai_split))
        .flatten();
    if let Some((cloud, local)) = ai {
        local_cleaner = local;
        let prompt = cleanup::session_prompt(
            &r.asr_text,
            cfg,
            &ctx,
            cloud.prompt_scope(),
            cleanup::PromptVersion::V5,
            language.as_deref(),
        );
        let timeout_ms = cloud.budget_ms(cfg.llm_budget_ms(&r.asr_text));
        let t = Instant::now();
        let (outcome, usage, detail) = cleanup::run_ai_cleanup_for(
            cloud,
            prompt,
            &r.asr_text,
            Duration::from_millis(timeout_ms),
            language.as_deref(),
        );
        r.timings.llm_ms = t.elapsed().as_millis() as u64;
        r.llm = usage.engine.is_some().then_some(usage);
        match outcome {
            CleanOutcome::Formatted(f) => {
                r.status = CommitStatus::Formatted;
                r.cleanup_route = Some("cloud".into());
                if abort.load(Ordering::SeqCst) {
                    return CommitResult::error("cancelled");
                }
                return post(cfg, matcher, &ctx, r, f);
            }
            CleanOutcome::Fallback(why) => {
                let why = why.as_str();
                r.cloud_fallback = Some(why.clone());
                r.fallback_reason = Some(why);
                r.fallback_detail = detail;
            }
        }
    }
    let tried_ai = r.cloud_fallback.is_some();
    // D36: a cleaner known to have nothing to run counts as none.
    let local_cleaner = local_cleaner.filter(|c| c.available());
    // D27: the deterministic layer follows `spoken_cleanup` alone.
    // `cleanup_level: none` and `raw_mode` only switch the LLM off
    // (Android sends both when "Clean up with local AI" is off, which
    // used to mean no cleanup at all: 17 % vs 44 % on the
    // formatting-gap eval). Verbatim = `spoken_cleanup: false` + none.
    if cfg.spoken_cleanup && !snippet {
        if english {
            text = postprocess::spoken::spoken_cleanup_keeping(&text, &cfg.dictionary.vocabulary());
            // Numbered lists and email structure without the LLM (the
            // LLM, when it runs, starts from this layout).
            text = postprocess::spoken::layout(&text, ctx.app_category);
        } else {
            // D33: English word rules would mangle other languages;
            // only the whitespace tidy is language-neutral.
            text = postprocess::spoken::tidy_whitespace(&text);
        }
        if text.trim().is_empty() {
            r.empty_reason = Some("only_filler".into());
            return r;
        }
    }
    if (cfg.spoken_cleanup || cfg.cleanup_level != CleanupLevel::None) && !snippet {
        // Parakeet-unified often ends without a period, whisper on a
        // comma; Wispr's server would close the sentence (D25).
        text = postprocess::terminal::close_sentence(&text);
    }
    r.status = CommitStatus::RawTranscript;
    if abort.load(Ordering::SeqCst) {
        return CommitResult::error("cancelled");
    }
    let mut run_llm = cleanup::should_run(cfg, ctx.app_category) && !snippet;
    // D27's "already clean" skip is for the on-device LLM only (D43,
    // owner decision 2026-11-02): it damages tidy text, a server or cloud
    // model does not. A tidy dictation whose first ready cleaner is the
    // server or the cloud is still cleaned, by the hops that are not
    // on-device (it never falls through to the on-device LLM).
    let mut local_cleaner = local_cleaner;
    if run_llm
        && cfg.llm_skip_clean
        && !cleanup::llm_can_help(&text, ctx.app_category, language.as_deref())
        && !cleanup::dictionary_can_help(&text, &cfg.dictionary)
    {
        if let Some(cl) = local_cleaner.take() {
            local_cleaner = match cleanup::first_ready_engine(&cl) {
                "local" => None,
                _ => cleanup::off_device(&cl),
            };
            if local_cleaner.is_none() {
                run_llm = false;
                r.fallback_reason = Some("llm_skipped_clean".into());
            }
        }
    }
    if let (true, Some(cl)) = (run_llm, &local_cleaner) {
        // D28: a cloud backend narrows what the prompt may carry.
        let prompt = cleanup::session_prompt(
            &text,
            cfg,
            &ctx,
            cl.prompt_scope(),
            cleanup::PromptVersion::V2,
            language.as_deref(),
        );
        // Scaled with the text's length when the session asks (a long
        // dictation decodes more tokens; flow-server sets the rate).
        let timeout_ms = cl.budget_ms(cfg.llm_budget_ms(&text));
        let t = Instant::now();
        let (outcome, usage) = cleanup::run_cleanup_metered_for(
            cl.clone(),
            prompt,
            &text,
            Duration::from_millis(timeout_ms),
            language.as_deref(),
        );
        // `None` when every backend refused up front (nothing ran).
        r.llm = usage.engine.is_some().then_some(usage);
        match outcome {
            CleanOutcome::Formatted(f) => {
                text = f;
                r.status = CommitStatus::Formatted;
            }
            // D36: refused up front by a cleaner that only now knows it
            // has nothing to run (first contact with a server without an
            // LLM): as if there were no cleaner.
            CleanOutcome::Fallback(_) if r.llm.is_none() && !cl.available() => {}
            CleanOutcome::Fallback(why) => r.fallback_reason = Some(why.as_str()),
        }
        // After a failed AI cleanup this adds to its time.
        r.timings.llm_ms += t.elapsed().as_millis() as u64;
    }
    if tried_ai {
        // D36: the fallback chain may have reached the user's server.
        let by_server = r.llm.as_ref().and_then(|l| l.engine.as_deref()) == Some("server");
        r.cleanup_route = Some(
            match (r.status == CommitStatus::Formatted, by_server) {
                (true, true) => "server",
                (true, false) => "local",
                (false, _) => "rules",
            }
            .into(),
        );
    }
    post(cfg, matcher, &ctx, r, text)
}

/// Post-processing (dictionary, cursor context, paste plan) on the
/// cleaned `text`.
fn post(
    cfg: &SessionConfig,
    matcher: &DictMatcher,
    ctx: &FieldContext,
    mut r: CommitResult,
    text: String,
) -> CommitResult {
    let t = Instant::now();
    let post = postprocess::apply_with(&text, &r.asr_text, cfg, ctx, matcher);
    r.timings.postprocess_ms = t.elapsed().as_millis() as u64;
    r.segments = paste::segments(&post.text, post.press_enter);
    r.plain_text = paste::plain_text(&post.text);
    r.final_text = post.text;
    r.press_enter = post.press_enter;
    r
}

/// D36: run the text half of a commit again on the client: spoken cleanup,
/// AI cleanup with `cleaner` (`None` = the deterministic text) and
/// post-processing, from `r.asr_text`. For a result that a server
/// transcribed without its own LLM (`cleanup_level: none`) while the
/// user's cleanup order picks another engine. ASR, the hallucination guard
/// and the timings of the server's work are kept; an `EMPTY` or `ERROR`
/// result, or one with no transcript, comes back unchanged.
pub fn refinish(
    cfg: &SessionConfig,
    ctx: &FieldContext,
    cleaner: Option<Arc<dyn Cleaner>>,
    mut r: CommitResult,
) -> CommitResult {
    if matches!(r.status, CommitStatus::Empty | CommitStatus::Error) || r.asr_text.is_empty() {
        return r;
    }
    let t = Instant::now();
    let mut cfg = cfg.clone();
    // The server already judged the transcript.
    cfg.hallucination_guard = false;
    let matcher = DictMatcher::new(&cfg.dictionary.entries, &[]);
    r.status = CommitStatus::RawTranscript;
    r.fallback_reason = None;
    r.llm = None;
    r.cleanup_route = None;
    r.cloud_fallback = None;
    r.fallback_detail = None;
    r.empty_reason = None;
    let server_llm_ms = r.timings.llm_ms;
    r.timings.llm_ms = 0;
    let abort = Arc::new(AtomicBool::new(false));
    let evidence = hallucination::Evidence {
        speech_ms: None,
        no_speech_prob: None,
    };
    let mut r = finish_text(cleaner.as_ref(), &cfg, &matcher, ctx, &abort, r, evidence);
    r.timings.llm_ms += server_llm_ms;
    r.timings.total_ms += t.elapsed().as_millis() as u64;
    r
}

#[cfg(test)]
mod keep_llm_tests {
    use std::sync::Weak;

    use super::*;
    use crate::cleanup::{CleanError, CleanPrompt};

    struct Fake;
    impl Cleaner for Fake {
        fn clean(&self, p: &CleanPrompt, _: Instant) -> Result<String, CleanError> {
            Ok(p.transcript.clone())
        }
    }

    /// A mock-ASR config naming an LLM no test loads: only a kept one can
    /// serve it.
    fn cfg() -> ModelConfig {
        ModelConfig {
            asr_backend: crate::config::AsrBackend::Mock { text: "hi".into() },
            llm_path: Some("invented.gguf".into()),
            ..Default::default()
        }
    }

    /// A core whose LLM was loaded for `cfg()`, as [`Load::finish`] builds it.
    fn core_with_llm() -> (FlowCore, Weak<dyn Cleaner>) {
        let c: Arc<dyn Cleaner> = Arc::new(Fake);
        let w = Arc::downgrade(&c);
        let mut core = FlowCore::from_engines(Arc::new(crate::asr::MockAsr::new("hi")), Some(c));
        core.llm_key = LlmKey::of(&cfg());
        (core, w)
    }

    fn same(a: &Arc<dyn Cleaner>, b: &Weak<dyn Cleaner>) -> bool {
        Weak::ptr_eq(&Arc::downgrade(a), b)
    }

    #[test]
    fn keep_holds_the_freed_engines_own_llm() {
        let slot = LlmSlot::new();
        let (old, old_llm) = core_with_llm();
        // Another engine loaded since (a router's on-demand local core):
        // keeping must not pick its LLM instead.
        let (_other, _) = core_with_llm();
        assert!(old.keep_llm_in(&slot));
        drop(old);
        let next = FlowCore::load_models_for_in(&slot, &cfg(), None).expect("reuses the kept LLM");
        assert!(same(&next.cleaner().unwrap(), &old_llm));
        assert!(next.load_timings().llm().unwrap().reused);
        // The new core can keep it again for the switch after.
        assert!(next.keep_llm_in(&slot));
    }

    #[test]
    fn a_core_without_a_loaded_llm_keeps_nothing() {
        let slot = LlmSlot::new();
        let built = FlowCore::from_engines(
            Arc::new(crate::asr::MockAsr::new("hi")),
            Some(Arc::new(Fake)),
        );
        assert!(!built.keep_llm_in(&slot));
        assert!(slot.take(LlmKey::of(&cfg()).as_ref()).is_none());
    }

    #[test]
    fn a_failed_speech_model_load_puts_the_kept_llm_back() {
        let slot = LlmSlot::new();
        let (old, old_llm) = core_with_llm();
        assert!(old.keep_llm_in(&slot));
        drop(old);
        let missing = ModelConfig {
            asr_backend: crate::config::AsrBackend::Whisper,
            asr_path: "invented/missing-model.bin".into(),
            ..cfg()
        };
        assert!(FlowCore::load_models_for_in(&slot, &missing, None).is_err());
        assert!(old_llm.upgrade().is_some(), "still kept");
        // The retry (model now there) reuses it.
        let retry = FlowCore::load_models_for_in(&slot, &cfg(), None).expect("retry");
        assert!(same(&retry.cleaner().unwrap(), &old_llm));
    }
}
