//! The integration point with `flow-core`: implements [`CoreApi`] on top of
//! `flow_core::FlowCore` and owns model loading.
//!
//! * Models load on a background thread at startup (`load`), so the tray
//!   comes up at once. A dictation started while loading waits up to
//!   [`LOAD_WAIT`] for it; a missing model file is a clear error state
//!   ([`ModelStatus::Missing`]) and is never downloaded (DECISIONS D9).
//! * The shell's §9 types are mapped onto flow-core's here (pure functions,
//!   unit-tested), so the rest of the shell never names a flow-core type.
//! * Default model: `%APPDATA%\FlowClone\models\ggml-base.bin`, multilingual,
//!   CPU (D4: the CPU tier; `small` q5 is the quality step up, set it in
//!   prefs `model_path`). Threads = flow-core's default (half the logical
//!   CPUs) unless prefs `asr_threads` says otherwise.

use crate::core_api::{
    AppKind, CleanupLevel, CommitResult, CommitStatus, CoreApi, DictEntry, FieldContext,
    LearnCandidate, SessionConfig, StyleBucket, Styles, Timings,
};
use flow_core::config as fc;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const DEFAULT_MODEL_FILE: &str = "ggml-base.bin";
/// How long a dictation started during model load waits for it.
pub const LOAD_WAIT: Duration = Duration::from_secs(15);

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

/// What to load.
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

    fn model_config(&self) -> fc::ModelConfig {
        match self {
            ModelSpec::Whisper { path, threads } => fc::ModelConfig {
                asr_backend: fc::AsrBackend::Whisper,
                asr_path: path.display().to_string(),
                llm_path: None,
                threads: threads.unwrap_or_else(fc::default_threads).max(1),
            },
            ModelSpec::Mock { text } => fc::ModelConfig {
                asr_backend: fc::AsrBackend::Mock { text: text.clone() },
                ..fc::ModelConfig::default()
            },
        }
    }
}

// ------------------------------------------------------------------ adapter

type StatusListener = Box<dyn Fn(&ModelStatus) + Send + Sync>;

struct Slot {
    status: ModelStatus,
    core: Option<Arc<flow_core::FlowCore>>,
    /// Bumped per `load` so a slow, superseded load can't win.
    generation: u64,
}

pub struct FlowCoreAdapter {
    slot: Mutex<Slot>,
    loaded: Condvar,
    /// Core of the open session (stays fixed even if a reload lands mid-session).
    active: RwLock<Option<Arc<flow_core::FlowCore>>>,
    listener: Mutex<Option<StatusListener>>,
}

impl FlowCoreAdapter {
    pub fn new() -> Arc<Self> {
        Arc::new(FlowCoreAdapter {
            slot: Mutex::new(Slot {
                status: ModelStatus::Loading {
                    path: String::new(),
                },
                core: None,
                generation: 0,
            }),
            loaded: Condvar::new(),
            active: RwLock::new(None),
            listener: Mutex::new(None),
        })
    }

    /// Called on every status change (tray notice). Fires once immediately.
    pub fn on_status(&self, f: impl Fn(&ModelStatus) + Send + Sync + 'static) {
        *self.listener.lock().unwrap() = Some(Box::new(f));
        self.notify();
    }

    pub fn status(&self) -> ModelStatus {
        self.slot.lock().unwrap().status.clone()
    }

    /// Start loading `spec` on a background thread (startup / model change).
    pub fn load(self: &Arc<Self>, spec: ModelSpec) {
        let generation = {
            let mut s = self.slot.lock().unwrap();
            s.generation += 1;
            s.status = ModelStatus::Loading { path: spec.label() };
            s.core = None;
            s.generation
        };
        self.notify();
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-model-load".into())
            .spawn(move || {
                let (status, core) = load_blocking(&spec);
                me.finish(generation, status, core);
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

    /// Synchronous load (tests, CLI).
    pub fn load_now(&self, spec: &ModelSpec) -> ModelStatus {
        let generation = {
            let mut s = self.slot.lock().unwrap();
            s.generation += 1;
            s.generation
        };
        let (status, core) = load_blocking(spec);
        self.finish(generation, status.clone(), core);
        status
    }

    fn finish(&self, generation: u64, status: ModelStatus, core: Option<flow_core::FlowCore>) {
        {
            let mut s = self.slot.lock().unwrap();
            if s.generation != generation {
                return; // superseded
            }
            s.status = status;
            s.core = core.map(Arc::new);
        }
        self.loaded.notify_all();
        self.notify();
    }

    fn notify(&self) {
        let st = self.status();
        if let Some(f) = self.listener.lock().unwrap().as_ref() {
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

fn load_blocking(spec: &ModelSpec) -> (ModelStatus, Option<flow_core::FlowCore>) {
    let path = spec.label();
    if let ModelSpec::Whisper { path: p, .. } = spec {
        if !p.is_file() {
            return (ModelStatus::Missing { path }, None);
        }
    }
    let t0 = Instant::now();
    match flow_core::FlowCore::load_models(&spec.model_config()) {
        Ok(core) => (
            ModelStatus::Ready {
                path,
                load_ms: t0.elapsed().as_millis() as u64,
            },
            Some(core),
        ),
        Err(e) => (
            ModelStatus::Failed {
                path,
                message: e.to_string(),
            },
            None,
        ),
    }
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
                replacement: None,
            })
            .collect()
    }
}

/// Startup entry: an adapter already loading the model from prefs.
pub fn make_core(prefs: &crate::prefs::Prefs, appdata: Option<&str>) -> Arc<FlowCoreAdapter> {
    let a = FlowCoreAdapter::new();
    a.load(ModelSpec::from_prefs(prefs, appdata));
    a
}

/// True when the prefs change means a different model must be loaded.
pub fn needs_reload(old: &crate::prefs::Prefs, new: &crate::prefs::Prefs) -> bool {
    old.model_path != new.model_path || old.asr_threads != new.asr_threads
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
