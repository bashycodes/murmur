//! The integration point with `flow-core`: implements [`CoreApi`] on top of
//! `flow_core::FlowCore` and owns model loading.
//!
//! * Models load on background threads at startup, so the tray comes up at
//!   once. A dictation started while the speech model loads waits up to
//!   [`LOAD_WAIT`] for it; a missing model file is a clear error state
//!   ([`ModelStatus::Missing`]) and is never downloaded (DECISIONS D9).
//! * **Two-phase load.** The speech engine and the optional LLM cleaner
//!   (llama.cpp in the `flow_llm` plugin, D10) load independently and are
//!   composed with `FlowCore::from_engines`. Dictation works as soon as ASR
//!   is ready (results are RAW_TRANSCRIPT); when the LLM finishes loading
//!   (10-50 s on CPU) the core is rebuilt around the same ASR engine plus
//!   the cleaner. An LLM failure never blocks dictation: it is reported as
//!   [`LlmStatus`] and the shell keeps pasting deterministic output.
//!   Toggling the LLM never reloads whisper.
//! * The shell's §9 types are mapped onto flow-core's here (pure functions,
//!   unit-tested), so the rest of the shell never names a flow-core type.
//! * Default models: `%APPDATA%\FlowClone\models\ggml-base.bin` (D4 CPU
//!   tier) and, when cleanup is on, `qwen2.5-0.5b-instruct-q4_k_m.gguf`
//!   (D11). Threads = flow-core's default unless prefs `asr_threads` says
//!   otherwise.

use crate::core_api::{
    AppKind, CleanupLevel, CommitResult, CommitStatus, CoreApi, DictEntry, FieldContext,
    LearnCandidate, SessionConfig, StyleBucket, Styles, Timings,
};
use flow_core::asr::AsrEngine;
use flow_core::cleanup::Cleaner;
use flow_core::config as fc;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL_FILE: &str = "ggml-base.bin";
/// D11: the 1.5 s CPU budget model. `qwen2.5-1.5b-instruct-q4_k_m.gguf` is
/// the quality step up (8/8 vs 4/8 on the messy-dictation set, ~2x slower).
pub const DEFAULT_LLM_FILE: &str = "qwen2.5-0.5b-instruct-q4_k_m.gguf";
/// How long a dictation started during model load waits for it. Short:
/// `start_session` runs on the controller loop, which must stay responsive.
pub const LOAD_WAIT: Duration = Duration::from_secs(3);

/// `%APPDATA%\FlowClone\models`.
pub fn models_dir(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join("models")
}

pub fn default_model_path(appdata: Option<&str>) -> PathBuf {
    models_dir(appdata).join(DEFAULT_MODEL_FILE)
}

pub fn default_llm_path(appdata: Option<&str>) -> PathBuf {
    models_dir(appdata).join(DEFAULT_LLM_FILE)
}

/// File name of the llama.cpp plugin the shell ships next to its exe.
pub fn llm_plugin_file() -> &'static str {
    flow_core::llama::PLUGIN_FILE
}

// ------------------------------------------------------------------ status

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    Loading {
        path: String,
    },
    Ready {
        path: String,
        load_ms: u64,
    },
    /// The model file does not exist. Never downloaded automatically.
    Missing {
        path: String,
    },
    Failed {
        path: String,
        message: String,
    },
}

impl ModelStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, ModelStatus::Ready { .. })
    }

    /// Text for the error pill / tray notice; `None` when dictation can run.
    pub fn problem(&self) -> Option<String> {
        match self {
            ModelStatus::Ready { .. } => None,
            ModelStatus::Loading { .. } => Some("Speech model is still loading".into()),
            ModelStatus::Missing { path } => Some(format!(
                "Speech model not found: {path}. Put a whisper.cpp ggml model there \
                 (e.g. ggml-base.bin) or choose one in Settings. Nothing is downloaded automatically."
            )),
            ModelStatus::Failed { path, message } => {
                Some(format!("Speech model failed to load ({path}): {message}"))
            }
        }
    }
}

/// State of the optional LLM cleanup (D5: off by default).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LlmStatus {
    /// Cleanup disabled in prefs: deterministic post-processing only.
    Off,
    Loading {
        path: String,
    },
    Ready {
        path: String,
        /// The `flow_llm` plugin that was loaded.
        plugin: String,
        load_ms: u64,
    },
    /// The GGUF file does not exist. Never downloaded automatically.
    Missing {
        path: String,
    },
    /// `flow_llm.dll` was not found next to the exe (or at `llm_lib_path`).
    PluginMissing {
        path: String,
        message: String,
    },
    Failed {
        path: String,
        message: String,
    },
}

impl LlmStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, LlmStatus::Ready { .. })
    }

    /// Settings / tray text when cleanup was asked for but cannot run.
    pub fn problem(&self) -> Option<String> {
        match self {
            LlmStatus::Off | LlmStatus::Ready { .. } | LlmStatus::Loading { .. } => None,
            LlmStatus::Missing { path } => Some(format!(
                "AI cleanup model not found: {path}. Put a GGUF instruct model there \
                 (e.g. {DEFAULT_LLM_FILE}) or choose one in Settings. Nothing is downloaded \
                 automatically; dictation keeps working without cleanup."
            )),
            LlmStatus::PluginMissing { message, .. } => Some(format!(
                "AI cleanup engine ({}) not found next to the app: {message}. \
                 Dictation keeps working without cleanup.",
                llm_plugin_file()
            )),
            LlmStatus::Failed { path, message } => Some(format!(
                "AI cleanup model failed to load ({path}): {message}. \
                 Dictation keeps working without cleanup."
            )),
        }
    }
}

/// What speech model to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSpec {
    Whisper {
        path: PathBuf,
        threads: Option<usize>,
    },
    /// flow-core's `MockAsr` (fixed transcript): tests and model-free dev.
    Mock { text: String },
}

impl ModelSpec {
    pub fn from_prefs(prefs: &crate::prefs::Prefs, appdata: Option<&str>) -> Self {
        ModelSpec::Whisper {
            path: prefs.resolved_model_path(appdata),
            threads: prefs.asr_threads,
        }
    }

    fn label(&self) -> String {
        match self {
            ModelSpec::Whisper { path, .. } => path.display().to_string(),
            ModelSpec::Mock { .. } => "mock".into(),
        }
    }

    /// The equivalent flow-core config (ASR only; the LLM is composed in
    /// separately, see the module docs).
    pub fn model_config(&self) -> fc::ModelConfig {
        match self {
            ModelSpec::Whisper { path, threads } => fc::ModelConfig {
                asr_backend: fc::AsrBackend::Whisper,
                asr_path: path.display().to_string(),
                llm_path: None,
                llm_lib: None,
                threads: threads.unwrap_or_else(fc::default_threads).max(1),
            },
            ModelSpec::Mock { text } => fc::ModelConfig {
                asr_backend: fc::AsrBackend::Mock { text: text.clone() },
                ..fc::ModelConfig::default()
            },
        }
    }
}

/// What LLM to load for cleanup; `None` in prefs terms = cleanup off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmSpec {
    pub model_path: PathBuf,
    /// Explicit `flow_llm` plugin; `None` = next to the exe (flow-core lookup).
    pub lib_path: Option<PathBuf>,
    pub threads: Option<usize>,
}

impl LlmSpec {
    pub fn from_prefs(prefs: &crate::prefs::Prefs, appdata: Option<&str>) -> Option<Self> {
        prefs.llm_enabled.then(|| LlmSpec {
            model_path: prefs.resolved_llm_path(appdata),
            lib_path: prefs
                .llm_lib_path
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
            threads: prefs.asr_threads,
        })
    }
}

/// Loads a cleaner; injectable so the two-phase logic is testable without
/// llama.cpp. `Ok` carries the plugin path for the status line.
pub type LlmLoader =
    Arc<dyn Fn(&LlmSpec) -> Result<(Arc<dyn Cleaner>, String), LlmStatus> + Send + Sync>;

/// The real loader: GGUF existence check, then `flow_core::llama`.
pub fn llama_loader() -> LlmLoader {
    Arc::new(|spec: &LlmSpec| {
        let path = spec.model_path.display().to_string();
        if !spec.model_path.is_file() {
            return Err(LlmStatus::Missing { path });
        }
        let threads = spec.threads.unwrap_or_else(fc::default_threads).max(1);
        let lib = spec.lib_path.as_ref().map(|p| p.display().to_string());
        match flow_core::llama::LlamaCleaner::load(&path, threads, lib.as_deref()) {
            Ok(c) => {
                let plugin = c.plugin_path.display().to_string();
                Ok((Arc::new(c) as Arc<dyn Cleaner>, plugin))
            }
            Err(e) => Err(classify_llm_error(path, e.to_string())),
        }
    })
}

/// flow-core reports every LLM failure as a string; tell "plugin not
/// found" apart so the settings page can say "ship flow_llm.dll".
fn classify_llm_error(path: String, message: String) -> LlmStatus {
    if message.starts_with("llm plugin not found") {
        LlmStatus::PluginMissing { path, message }
    } else {
        LlmStatus::Failed { path, message }
    }
}

// ------------------------------------------------------------------ adapter

type Listener<T> = Box<dyn Fn(&T) + Send + Sync>;

struct Slot {
    status: ModelStatus,
    asr: Option<Arc<dyn AsrEngine>>,
    llm_status: LlmStatus,
    cleaner: Option<Arc<dyn Cleaner>>,
    /// `from_engines(asr, cleaner)`; rebuilt whenever either changes.
    core: Option<Arc<flow_core::FlowCore>>,
    /// Bumped per load so a slow, superseded load can't win.
    generation: u64,
    llm_generation: u64,
}

impl Slot {
    fn rebuild(&mut self) {
        self.core = self.asr.as_ref().map(|asr| {
            Arc::new(flow_core::FlowCore::from_engines(
                asr.clone(),
                self.cleaner.clone(),
            ))
        });
    }
}

pub struct FlowCoreAdapter {
    slot: Mutex<Slot>,
    loaded: Condvar,
    /// Core of the open session (stays fixed even if a reload lands mid-session).
    active: RwLock<Option<Arc<flow_core::FlowCore>>>,
    listener: Mutex<Option<Listener<ModelStatus>>>,
    llm_listener: Mutex<Option<Listener<LlmStatus>>>,
    llm_loader: LlmLoader,
}

impl FlowCoreAdapter {
    pub fn new() -> Arc<Self> {
        Self::with_llm_loader(llama_loader())
    }

    /// Adapter whose LLM comes from `loader` (tests: a fake cleaner).
    pub fn with_llm_loader(loader: LlmLoader) -> Arc<Self> {
        Arc::new(FlowCoreAdapter {
            slot: Mutex::new(Slot {
                status: ModelStatus::Loading {
                    path: String::new(),
                },
                asr: None,
                llm_status: LlmStatus::Off,
                cleaner: None,
                core: None,
                generation: 0,
                llm_generation: 0,
            }),
            loaded: Condvar::new(),
            active: RwLock::new(None),
            listener: Mutex::new(None),
            llm_listener: Mutex::new(None),
            llm_loader: loader,
        })
    }

    /// Called on every speech-model status change (tray notice). Fires once
    /// immediately.
    pub fn on_status(&self, f: impl Fn(&ModelStatus) + Send + Sync + 'static) {
        *self.listener.lock().unwrap() = Some(Box::new(f));
        self.notify();
    }

    /// Called on every LLM status change. Fires once immediately.
    pub fn on_llm_status(&self, f: impl Fn(&LlmStatus) + Send + Sync + 'static) {
        *self.llm_listener.lock().unwrap() = Some(Box::new(f));
        self.notify_llm();
    }

    pub fn status(&self) -> ModelStatus {
        self.slot.lock().unwrap().status.clone()
    }

    pub fn llm_status(&self) -> LlmStatus {
        self.slot.lock().unwrap().llm_status.clone()
    }

    /// Start loading the speech model on a background thread.
    pub fn load(self: &Arc<Self>, spec: ModelSpec) {
        let generation = {
            let mut s = self.slot.lock().unwrap();
            s.generation += 1;
            s.status = ModelStatus::Loading { path: spec.label() };
            s.asr = None;
            s.core = None;
            s.generation
        };
        self.notify();
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-model-load".into())
            .spawn(move || {
                let (status, asr) = load_asr(&spec);
                me.finish(generation, status, asr);
            });
        if let Err(e) = spawned {
            self.finish(
                generation,
                ModelStatus::Failed {
                    path: String::new(),
                    message: e.to_string(),
                },
                None,
            );
        }
    }

    /// Synchronous speech-model load (tests, CLI).
    pub fn load_now(&self, spec: &ModelSpec) -> ModelStatus {
        let generation = {
            let mut s = self.slot.lock().unwrap();
            s.generation += 1;
            s.generation
        };
        let (status, asr) = load_asr(spec);
        self.finish(generation, status.clone(), asr);
        status
    }

    fn finish(&self, generation: u64, status: ModelStatus, asr: Option<Arc<dyn AsrEngine>>) {
        {
            let mut s = self.slot.lock().unwrap();
            if s.generation != generation {
                return; // superseded
            }
            s.status = status;
            s.asr = asr;
            s.rebuild();
        }
        self.loaded.notify_all();
        self.notify();
    }

    /// Turn cleanup off (`None`) or (re)load the LLM in the background.
    /// Dictation keeps running on the current core meanwhile.
    pub fn load_llm(self: &Arc<Self>, spec: Option<LlmSpec>) {
        let Some(spec) = spec else {
            self.set_llm_off();
            return;
        };
        let generation = self.begin_llm(&spec);
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-llm-load".into())
            .spawn(move || {
                let t0 = Instant::now();
                let r = (me.llm_loader)(&spec);
                me.finish_llm(generation, &spec, r, t0.elapsed().as_millis() as u64);
            });
        if let Err(e) = spawned {
            self.finish_llm_status(
                generation,
                LlmStatus::Failed {
                    path: String::new(),
                    message: e.to_string(),
                },
                None,
            );
        }
    }

    /// Synchronous LLM load (tests).
    pub fn load_llm_now(&self, spec: Option<LlmSpec>) -> LlmStatus {
        let Some(spec) = spec else {
            self.set_llm_off();
            return LlmStatus::Off;
        };
        let generation = self.begin_llm(&spec);
        let t0 = Instant::now();
        let r = (self.llm_loader)(&spec);
        self.finish_llm(generation, &spec, r, t0.elapsed().as_millis() as u64);
        self.llm_status()
    }

    fn set_llm_off(&self) {
        {
            let mut s = self.slot.lock().unwrap();
            s.llm_generation += 1;
            s.llm_status = LlmStatus::Off;
            if s.cleaner.take().is_some() {
                s.rebuild();
            }
        }
        self.notify_llm();
    }

    fn begin_llm(&self, spec: &LlmSpec) -> u64 {
        let generation = {
            let mut s = self.slot.lock().unwrap();
            s.llm_generation += 1;
            s.llm_status = LlmStatus::Loading {
                path: spec.model_path.display().to_string(),
            };
            s.llm_generation
        };
        self.notify_llm();
        generation
    }

    fn finish_llm(
        &self,
        generation: u64,
        spec: &LlmSpec,
        r: Result<(Arc<dyn Cleaner>, String), LlmStatus>,
        load_ms: u64,
    ) {
        match r {
            Ok((cleaner, plugin)) => self.finish_llm_status(
                generation,
                LlmStatus::Ready {
                    path: spec.model_path.display().to_string(),
                    plugin,
                    load_ms,
                },
                Some(cleaner),
            ),
            Err(st) => self.finish_llm_status(generation, st, None),
        }
    }

    fn finish_llm_status(
        &self,
        generation: u64,
        status: LlmStatus,
        cleaner: Option<Arc<dyn Cleaner>>,
    ) {
        {
            let mut s = self.slot.lock().unwrap();
            if s.llm_generation != generation {
                return; // superseded (toggled off, or another path chosen)
            }
            s.llm_status = status;
            s.cleaner = cleaner;
            s.rebuild();
        }
        self.notify_llm();
    }

    fn notify(&self) {
        let st = self.status();
        if let Some(f) = self.listener.lock().unwrap().as_ref() {
            f(&st);
        }
    }

    fn notify_llm(&self) {
        let st = self.llm_status();
        if let Some(f) = self.llm_listener.lock().unwrap().as_ref() {
            f(&st);
        }
    }

    /// The loaded core, waiting up to `wait` while a load is in progress.
    fn ready_core(&self, wait: Duration) -> Result<Arc<flow_core::FlowCore>, String> {
        let deadline = Instant::now() + wait;
        let mut s = self.slot.lock().unwrap();
        loop {
            if let Some(c) = &s.core {
                return Ok(c.clone());
            }
            let now = Instant::now();
            if !matches!(s.status, ModelStatus::Loading { .. }) || now >= deadline {
                return Err(s
                    .status
                    .problem()
                    .unwrap_or_else(|| "core unavailable".into()));
            }
            s = self.loaded.wait_timeout(s, deadline - now).unwrap().0;
        }
    }

    fn active(&self) -> Option<Arc<flow_core::FlowCore>> {
        self.active.read().unwrap().clone()
    }
}

fn load_asr(spec: &ModelSpec) -> (ModelStatus, Option<Arc<dyn AsrEngine>>) {
    let path = spec.label();
    let t0 = Instant::now();
    let engine: Result<Arc<dyn AsrEngine>, String> = match spec {
        ModelSpec::Mock { text } => Ok(Arc::new(flow_core::asr::MockAsr::new(text))),
        ModelSpec::Whisper { path: p, threads } => {
            if !p.is_file() {
                return (ModelStatus::Missing { path }, None);
            }
            whisper_engine(p, threads.unwrap_or_else(fc::default_threads).max(1))
        }
    };
    match engine {
        Ok(e) => (
            ModelStatus::Ready {
                path,
                load_ms: t0.elapsed().as_millis() as u64,
            },
            Some(e),
        ),
        Err(message) => (ModelStatus::Failed { path, message }, None),
    }
}

#[cfg(feature = "whisper")]
fn whisper_engine(p: &std::path::Path, threads: usize) -> Result<Arc<dyn AsrEngine>, String> {
    flow_core::whisper::WhisperEngine::load(&p.display().to_string(), threads)
        .map(|e| Arc::new(e) as Arc<dyn AsrEngine>)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "whisper"))]
fn whisper_engine(_p: &std::path::Path, _threads: usize) -> Result<Arc<dyn AsrEngine>, String> {
    Err("built without the `whisper` feature".into())
}

impl CoreApi for FlowCoreAdapter {
    fn start_session(&self, cfg: SessionConfig) -> Result<(), String> {
        let core = self.ready_core(LOAD_WAIT)?;
        core.start_session(to_core_session(&cfg))
            .map_err(|e| e.to_string())?;
        *self.active.write().unwrap() = Some(core);
        Ok(())
    }

    fn push_audio(&self, pcm: &[i16]) {
        if let Some(c) = self.active.read().unwrap().as_ref() {
            c.push_audio(pcm);
        }
    }

    fn set_context(&self, ctx: FieldContext) {
        if let Some(c) = self.active() {
            c.set_context(to_core_context(&ctx));
        }
    }

    fn partial(&self) -> Option<String> {
        self.active()?.partial().filter(|p| !p.trim().is_empty())
    }

    fn commit(&self) -> CommitResult {
        let core = self.active.write().unwrap().take();
        match core {
            Some(c) => from_core_result(c.commit()),
            None => CommitResult::failed("no active session"),
        }
    }

    fn cancel(&self) {
        if let Some(c) = self.active.write().unwrap().take() {
            c.cancel();
        }
    }

    fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<LearnCandidate> {
        let core = self
            .active()
            .or_else(|| self.slot.lock().unwrap().core.clone());
        let Some(c) = core else { return Vec::new() };
        c.learn_edit(pasted, edited)
            .into_iter()
            .map(|cand| LearnCandidate {
                word: cand.corrected,
                heard: cand.original,
                replacement: None,
            })
            .collect()
    }
}

/// Startup entry: an adapter already loading the models from prefs.
pub fn make_core(prefs: &crate::prefs::Prefs, appdata: Option<&str>) -> Arc<FlowCoreAdapter> {
    let a = FlowCoreAdapter::new();
    a.load(ModelSpec::from_prefs(prefs, appdata));
    a.load_llm(LlmSpec::from_prefs(prefs, appdata));
    a
}

/// True when the prefs change means a different speech model must be loaded.
pub fn needs_reload(old: &crate::prefs::Prefs, new: &crate::prefs::Prefs) -> bool {
    old.model_path != new.model_path || old.asr_threads != new.asr_threads
}

/// True when the prefs change means the LLM must be loaded or dropped.
pub fn needs_llm_reload(old: &crate::prefs::Prefs, new: &crate::prefs::Prefs) -> bool {
    LlmSpec::from_prefs(old, None) != LlmSpec::from_prefs(new, None)
}

// ------------------------------------------------------------------ mapping

fn parse_style(s: &str) -> fc::Style {
    let key = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    serde_json::from_value(serde_json::Value::String(key)).unwrap_or_default()
}

fn to_core_styles(s: &Styles) -> fc::Styles {
    fc::Styles {
        personal: parse_style(&s.personal),
        work: parse_style(&s.work),
        email: parse_style(&s.email),
        other: parse_style(&s.other),
    }
}

fn to_core_level(l: CleanupLevel) -> fc::CleanupLevel {
    match l {
        CleanupLevel::None => fc::CleanupLevel::None,
        CleanupLevel::Light => fc::CleanupLevel::Light,
        CleanupLevel::Medium => fc::CleanupLevel::Medium,
        CleanupLevel::High => fc::CleanupLevel::High,
    }
}

fn to_core_dict(d: &[DictEntry]) -> flow_core::dictionary::Dictionary {
    flow_core::dictionary::Dictionary {
        entries: d
            .iter()
            .map(|e| flow_core::dictionary::DictEntry {
                word: e.word.clone(),
                replacement: e.replacement.clone(),
                is_snippet: e.is_snippet && e.replacement.is_some(),
            })
            .collect(),
    }
}

pub fn to_core_session(c: &SessionConfig) -> fc::SessionConfig {
    fc::SessionConfig {
        languages: c.languages.clone(),
        dictionary: to_core_dict(&c.dictionary),
        styles: to_core_styles(&c.styles),
        cleanup_level: to_core_level(c.cleanup_level),
        raw_mode: c.raw_mode,
        max_samples: usize::try_from(c.max_samples).unwrap_or(usize::MAX).max(1),
        platform: fc::Platform::Windows,
        partial_interval_ms: c.partial_interval_ms,
        llm_timeout_ms: if c.llm_timeout_ms == 0 {
            fc::SessionConfig::default().llm_timeout_ms
        } else {
            c.llm_timeout_ms
        },
        ..fc::SessionConfig::default()
    }
}

/// Finer app kind first (terminal/AI change formatting); the style bucket
/// (which has a browser tab-title fallback) fills in when the kind is unknown.
pub fn to_core_category(kind: AppKind, bucket: StyleBucket) -> fc::AppCategory {
    match (kind, bucket) {
        (AppKind::Ai, _) => fc::AppCategory::AiPrompt,
        (AppKind::Email, _) => fc::AppCategory::Email,
        (AppKind::WorkMessaging, _) => fc::AppCategory::WorkChat,
        (AppKind::Messaging, _) => fc::AppCategory::PersonalChat,
        (AppKind::Document, _) => fc::AppCategory::Document,
        (AppKind::Terminal, _) => fc::AppCategory::Terminal,
        (AppKind::Other, StyleBucket::Email) => fc::AppCategory::Email,
        (AppKind::Other, StyleBucket::Work) => fc::AppCategory::WorkChat,
        (AppKind::Other, StyleBucket::Personal) => fc::AppCategory::PersonalChat,
        (AppKind::Other, StyleBucket::Other) => fc::AppCategory::Other,
    }
}

pub fn to_core_context(c: &FieldContext) -> fc::FieldContext {
    fc::FieldContext {
        // flow-core's Windows lists key on the exe stem.
        app_id: c.app_id.trim_end_matches(".exe").to_string(),
        url_host: c.url_host.clone().unwrap_or_default(),
        app_category: to_core_category(c.app_kind, c.app_category),
        before: c.before.clone(),
        selected: c.selected.clone(),
        after: c.after.clone(),
        screen_words: c.screen_words.clone(),
        file_names: Vec::new(),
        context_known: true,
    }
}

pub fn from_core_result(r: flow_core::CommitResult) -> CommitResult {
    use flow_core::CommitStatus as S;
    let status = match r.status {
        S::Formatted => CommitStatus::Formatted,
        S::RawTranscript => CommitStatus::RawTranscript,
        S::Empty => CommitStatus::Empty,
        S::Error => CommitStatus::Error,
    };
    CommitResult {
        status,
        asr_text: r.asr_text,
        final_text: r.final_text,
        language: r.language,
        timings: Timings {
            asr_ms: r.timings.asr_ms,
            llm_ms: r.timings.llm_ms,
            total_ms: r.timings.total_ms,
        },
        press_enter: r.press_enter,
        fallback_reason: r.fallback_reason,
        error: r.error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn default_model_path_is_under_appdata() {
        let p = default_model_path(Some(r"C:\Users\u\AppData\Roaming"));
        assert!(p.ends_with(Path::new("FlowClone").join("models").join("ggml-base.bin")));
    }

    #[test]
    fn styles_parse_leniently() {
        assert_eq!(parse_style("casual"), fc::Style::Casual);
        assert_eq!(parse_style("Very Casual"), fc::Style::VeryCasual);
        assert_eq!(parse_style("formal"), fc::Style::Formal);
        assert_eq!(parse_style("nonsense"), fc::Style::Default);
    }

    #[test]
    fn session_mapping_keeps_dictionary_and_sets_windows() {
        let cfg = SessionConfig {
            languages: vec!["en".into()],
            dictionary: vec![
                DictEntry {
                    word: "gh".into(),
                    replacement: Some("GitHub".into()),
                    is_snippet: false,
                },
                DictEntry {
                    word: "sig".into(),
                    replacement: None,
                    is_snippet: true, // snippet without a body = plain word
                },
            ],
            cleanup_level: CleanupLevel::None,
            max_samples: 16_000 * 60,
            partial_interval_ms: Some(1000),
            ..SessionConfig::default()
        };
        let c = to_core_session(&cfg);
        assert_eq!(c.platform, fc::Platform::Windows);
        assert_eq!(c.forced_language(), Some("en"));
        assert_eq!(c.dictionary.entries.len(), 2);
        assert!(!c.dictionary.entries[1].is_snippet);
        assert_eq!(c.cleanup_level, fc::CleanupLevel::None);
        assert_eq!(c.max_samples, 16_000 * 60);
        assert_eq!(c.partial_interval_ms, Some(1000));
        let json = serde_json::to_string(&c.dictionary).unwrap();
        assert!(json.contains(r#""isSnippet":false"#), "{json}");
    }

    #[test]
    fn context_mapping() {
        let ctx = FieldContext {
            app_id: "slack.exe".into(),
            url_host: None,
            app_category: StyleBucket::Work,
            app_kind: AppKind::WorkMessaging,
            before: "Hi ".into(),
            ..FieldContext::default()
        };
        let c = to_core_context(&ctx);
        assert_eq!(c.app_id, "slack");
        assert_eq!(c.app_category, fc::AppCategory::WorkChat);
        assert_eq!(c.before, "Hi ");
        assert!(c.context_known);
        assert_eq!(
            to_core_category(AppKind::Other, StyleBucket::Email),
            fc::AppCategory::Email,
            "browser tab-title fallback"
        );
        assert_eq!(
            to_core_category(AppKind::Terminal, StyleBucket::Other),
            fc::AppCategory::Terminal
        );
    }

    #[test]
    fn missing_model_is_a_clear_error_and_never_loads() {
        let a = FlowCoreAdapter::new();
        let st = a.load_now(&ModelSpec::Whisper {
            path: PathBuf::from("/nonexistent/ggml-base.bin"),
            threads: None,
        });
        assert!(matches!(st, ModelStatus::Missing { .. }));
        let err = a.start_session(SessionConfig::default()).unwrap_err();
        assert!(err.contains("not found"), "{err}");
        assert!(err.contains("Nothing is downloaded"), "{err}");
        assert_eq!(a.commit().status, CommitStatus::Error);
    }

    #[test]
    fn async_load_is_awaited_by_start_session() {
        let a = FlowCoreAdapter::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        a.on_status(move |st| s2.lock().unwrap().push(st.clone()));
        a.load(ModelSpec::Mock {
            text: "hello there".into(),
        });
        a.start_session(SessionConfig::default()).unwrap();
        a.push_audio(&[3000; 16_000]);
        let r = a.commit();
        assert_eq!(r.status, CommitStatus::RawTranscript);
        assert_eq!(r.final_text.trim(), "hello there");
        let seen = seen.lock().unwrap();
        assert!(seen
            .iter()
            .any(|s| matches!(s, ModelStatus::Loading { .. })));
        assert!(seen.last().unwrap().is_ready());
    }

    #[test]
    fn cancel_then_commit_is_error_not_stale_text() {
        let a = FlowCoreAdapter::new();
        a.load_now(&ModelSpec::Mock { text: "x".into() });
        a.start_session(SessionConfig::default()).unwrap();
        a.push_audio(&[3000; 16_000]);
        a.cancel();
        assert_eq!(a.commit().status, CommitStatus::Error);
    }

    /// Capitalise + full stop: small enough to pass the divergence guardrail.
    struct Sentence;
    impl Cleaner for Sentence {
        fn clean(
            &self,
            p: &flow_core::cleanup::CleanPrompt,
            _deadline: Instant,
        ) -> Result<String, flow_core::cleanup::CleanError> {
            let t = p.transcript.trim();
            let mut c = t.chars();
            let first = c.next().map(|f| f.to_uppercase().collect::<String>());
            Ok(format!("{}{}.", first.unwrap_or_default(), c.as_str()))
        }
    }

    fn fake_loader(delay_ms: u64, fail: Option<LlmStatus>) -> LlmLoader {
        Arc::new(move |_spec: &LlmSpec| {
            std::thread::sleep(Duration::from_millis(delay_ms));
            match &fail {
                Some(st) => Err(st.clone()),
                None => Ok((Arc::new(Sentence) as Arc<dyn Cleaner>, "fake-plugin".into())),
            }
        })
    }

    fn spec() -> LlmSpec {
        LlmSpec {
            model_path: PathBuf::from("/m/qwen.gguf"),
            lib_path: None,
            threads: Some(1),
        }
    }

    const SPEECH: &str = "please send the report today";

    fn dictate(a: &FlowCoreAdapter) -> CommitResult {
        a.start_session(SessionConfig::default()).unwrap();
        a.push_audio(&[3000; 16_000]);
        a.commit()
    }

    #[test]
    fn llm_off_means_raw_transcript() {
        let a = FlowCoreAdapter::with_llm_loader(fake_loader(0, None));
        a.load_now(&ModelSpec::Mock {
            text: SPEECH.into(),
        });
        assert_eq!(a.load_llm_now(None), LlmStatus::Off);
        assert_eq!(dictate(&a).status, CommitStatus::RawTranscript);
    }

    #[test]
    fn llm_joins_a_running_core_and_can_leave_again() {
        let a = FlowCoreAdapter::with_llm_loader(fake_loader(0, None));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        a.on_llm_status(move |st| s2.lock().unwrap().push(st.clone()));
        a.load_now(&ModelSpec::Mock {
            text: SPEECH.into(),
        });
        assert_eq!(dictate(&a).status, CommitStatus::RawTranscript);
        let st = a.load_llm_now(Some(spec()));
        assert!(st.is_ready(), "{st:?}");
        let r = dictate(&a);
        assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
        assert!(r.final_text.starts_with("Please send"), "{r:?}");
        assert_eq!(r.asr_text, SPEECH, "raw kept for undo-AI-edit");
        a.load_llm_now(None);
        assert_eq!(dictate(&a).status, CommitStatus::RawTranscript);
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|s| matches!(s, LlmStatus::Loading { .. })));
        assert_eq!(seen.last(), Some(&LlmStatus::Off));
    }

    #[test]
    fn llm_ready_before_asr_is_used_once_asr_lands() {
        let a = FlowCoreAdapter::with_llm_loader(fake_loader(0, None));
        a.load_llm_now(Some(spec()));
        a.load_now(&ModelSpec::Mock {
            text: SPEECH.into(),
        });
        assert_eq!(dictate(&a).status, CommitStatus::Formatted);
    }

    #[test]
    fn llm_failure_never_blocks_dictation() {
        let missing = LlmStatus::PluginMissing {
            path: "/m/qwen.gguf".into(),
            message: "llm plugin not found (x)".into(),
        };
        let a = FlowCoreAdapter::with_llm_loader(fake_loader(0, Some(missing.clone())));
        a.load_now(&ModelSpec::Mock {
            text: SPEECH.into(),
        });
        assert_eq!(a.load_llm_now(Some(spec())), missing);
        assert!(a
            .llm_status()
            .problem()
            .unwrap()
            .contains(llm_plugin_file()));
        assert_eq!(dictate(&a).status, CommitStatus::RawTranscript);
        assert!(a.status().is_ready());
    }

    #[test]
    fn superseded_llm_load_does_not_win() {
        let a = FlowCoreAdapter::with_llm_loader(fake_loader(150, None));
        a.load_now(&ModelSpec::Mock {
            text: SPEECH.into(),
        });
        a.load_llm(Some(spec()));
        assert!(matches!(a.llm_status(), LlmStatus::Loading { .. }));
        a.load_llm(None); // user unticks while it loads
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(a.llm_status(), LlmStatus::Off);
        assert_eq!(dictate(&a).status, CommitStatus::RawTranscript);
    }

    #[test]
    fn real_loader_reports_missing_model_and_missing_plugin() {
        let load = llama_loader();
        let Err(st) = load(&LlmSpec {
            model_path: PathBuf::from("/nonexistent/qwen.gguf"),
            ..spec()
        }) else {
            panic!("loaded a nonexistent model")
        };
        assert!(matches!(st, LlmStatus::Missing { .. }), "{st:?}");
        assert!(st.problem().unwrap().contains("Nothing is downloaded"));
        let Err(st) = load(&LlmSpec {
            model_path: PathBuf::from(env!("CARGO_MANIFEST_PATH")),
            lib_path: Some(PathBuf::from("/nonexistent/libflow_llm.so")),
            threads: Some(1),
        }) else {
            panic!("loaded a nonexistent plugin")
        };
        assert!(matches!(st, LlmStatus::PluginMissing { .. }), "{st:?}");
    }

    #[test]
    fn llm_spec_follows_prefs() {
        let mut p = crate::prefs::Prefs::default();
        assert_eq!(
            LlmSpec::from_prefs(&p, Some("/ad")),
            None,
            "D5: off by default"
        );
        p.llm_enabled = true;
        let s = LlmSpec::from_prefs(&p, Some("/ad")).unwrap();
        assert!(s.model_path.ends_with(DEFAULT_LLM_FILE));
        assert_eq!(s.lib_path, None);
        let mut q = p.clone();
        q.llm_lib_path = Some("  ".into());
        assert!(!needs_llm_reload(&p, &q), "blank lib path = default lookup");
        q.llm_model_path = Some("/m/qwen2.5-1.5b.gguf".into());
        assert!(needs_llm_reload(&p, &q));
        assert!(!needs_reload(&p, &q), "LLM change never reloads whisper");
        q.llm_enabled = false;
        q.llm_timeout_ms = 3000;
        assert!(needs_llm_reload(&p, &q));
    }

    #[test]
    fn llm_timeout_reaches_the_core() {
        let c = to_core_session(&SessionConfig {
            llm_timeout_ms: 2500,
            ..SessionConfig::default()
        });
        assert_eq!(c.llm_timeout_ms, 2500);
        let c = to_core_session(&SessionConfig::default());
        assert_eq!(c.llm_timeout_ms, 1500, "0 = core default");
    }

    #[test]
    fn reload_needed_only_for_model_prefs() {
        let a = crate::prefs::Prefs::default();
        let mut b = a.clone();
        b.enable_sounds = false;
        assert!(!needs_reload(&a, &b));
        b.model_path = Some("C:/m/ggml-small.bin".into());
        assert!(needs_reload(&a, &b));
    }
}
