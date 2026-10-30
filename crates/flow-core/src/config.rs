//! Configuration and context types crossing the core API (§9 v2).
//!
//! All types are serde so the FFI/JNI layers can pass JSON verbatim.
//! Every field has a default so shells may send partial objects.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dictionary::Dictionary;

pub const SAMPLE_RATE: usize = 16_000;

/// Platform whose Wispr client behaviour the post-processor mirrors where
/// Windows and Android differ.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    #[default]
    Windows,
    Android,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupLevel {
    /// No LLM pass. Deterministic cleanup still follows
    /// `SessionConfig::spoken_cleanup` (D27).
    None,
    #[default]
    Light,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Wispr "default": period drop only in messaging apps (<= 2 sentences).
    #[default]
    Default,
    Formal,
    Casual,
    VeryCasual,
    Excited,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    Email,
    WorkChat,
    PersonalChat,
    AiPrompt,
    Terminal,
    Document,
    #[default]
    Other,
}

impl AppCategory {
    pub fn is_chat(self) -> bool {
        matches!(self, AppCategory::WorkChat | AppCategory::PersonalChat)
    }
}

/// Per-bucket style preference (Wispr: work / email / personal / other).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Styles {
    pub personal: Style,
    pub work: Style,
    pub email: Style,
    pub other: Style,
}

impl Styles {
    pub fn for_category(&self, c: AppCategory) -> Style {
        match c {
            AppCategory::PersonalChat => self.personal,
            AppCategory::WorkChat => self.work,
            AppCategory::Email => self.email,
            _ => self.other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Preferred languages (ISO 639-1). Empty or `["auto"]` = auto-detect;
    /// exactly one = forced.
    pub languages: Vec<String>,
    pub dictionary: Dictionary,
    pub styles: Styles,
    pub cleanup_level: CleanupLevel,
    /// Skip the LLM entirely (Wispr "No AI formatting"). The deterministic
    /// layer still runs unless `spoken_cleanup` is off (D27).
    pub raw_mode: bool,
    /// Hard safety bound; the shell owns the real session cap policy.
    pub max_samples: usize,
    pub platform: Platform,
    /// Emit live partials every N ms of new audio; `None` = off.
    pub partial_interval_ms: Option<u32>,
    /// LLM cleanup budget before falling back to RAW_TRANSCRIPT.
    pub llm_timeout_ms: u64,
    /// Optional extra guard: reject LLM output whose word edit distance /
    /// words exceeds this (0..1; the old 03 §7.1 check). `None` (default):
    /// only the support/coverage guard (`guard::check`) runs.
    pub divergence_threshold: Option<f64>,
    /// Ring-buffer capacity between `push_audio` and the worker.
    pub ring_capacity_samples: usize,
    /// Wispr "All lowercase" format mode.
    pub all_lowercase: bool,
    /// User's first/last name: proper nouns that keep their capital.
    pub user_names: Vec<String>,
    /// Spoken "press enter" strips the phrase and sets `press_enter`.
    pub press_enter_enabled: bool,
    /// Line appended to eligible emails, e.g. "— Sent by voice". Off by default.
    pub email_signature: Option<String>,
    /// Energy + zero-crossing VAD at commit: no speech → EMPTY without ASR;
    /// leading/trailing silence is trimmed before ASR. Default on.
    pub vad: bool,
    /// Drop Whisper's stock silence phrases ("Thank you.", "you", subtitle
    /// credits) when there was little or no speech. Default on.
    pub hallucination_guard: bool,
    /// Prefill the LLM's KV cache with the prompt (context + partial
    /// transcript) while the user is still speaking, so commit only decodes
    /// the difference. Only when the LLM would run. Default on.
    pub llm_prefill: bool,
    /// Skip the LLM when the deterministic result leaves it nothing to do
    /// (`cleanup::llm_can_help`: short, punctuated, capitalised, no fillers,
    /// command or number words left). `fallback_reason` is then
    /// `llm_skipped_clean`. Default on (D27).
    pub llm_skip_clean: bool,
    /// Deterministic spoken cleanup before the LLM and post-processing:
    /// spoken punctuation, fillers, stutters, "scratch that"/"no wait"
    /// backtracking, money/time/date formatting (`postprocess::spoken`),
    /// layout and the closing period. Independent of `cleanup_level` and
    /// `raw_mode`, which only gate the LLM (D27; before 2026-10-24 level
    /// `none` switched this off too). Off + level `none` = verbatim (the
    /// Wispr client port still runs). Default on.
    pub spoken_cleanup: bool,
    /// Hands-free endpointing (D19): after at least 300 ms of speech, this
    /// much trailing silence sets `SessionPoll.endpoint_reached` (see
    /// `FlowCore::poll`). The core never stops by itself; the shell commits
    /// when it sees the signal. `None` (default) = off. Values below 300
    /// are raised to 300; `endpoint::RECOMMENDED_SILENCE_MS` = 1500 (was 2000
    /// before D22).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_silence_ms: Option<u32>,
    /// Pause-segmented incremental decoding (D22): cut the dictation at
    /// pauses of at least this many ms (min 200; recommended
    /// `segment::RECOMMENDED_PAUSE_MS` = 800) and decode each finished
    /// piece while the user keeps talking, so `commit` decodes only the
    /// tail. Cuts depend only on the audio, so the text is the same on
    /// every route. Ignored by engines that must not see audio before
    /// commit (cloud STT, D18). `None` (default) = off: today's single
    /// decode at commit, byte-identical. Omitted from JSON when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_pause_ms: Option<u32>,
    /// D22: a piece shorter than this keeps growing past a pause. `None` =
    /// 4000.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_min_ms: Option<u32>,
    /// D22: a piece this long without a qualifying pause is cut at its
    /// longest gap of at least 200 ms in the last 5 s, if any. `None` =
    /// 20000.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_soft_max_ms: Option<u32>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            dictionary: Dictionary::default(),
            styles: Styles::default(),
            cleanup_level: CleanupLevel::Light,
            raw_mode: false,
            max_samples: SAMPLE_RATE * 60 * 20,
            platform: Platform::Windows,
            partial_interval_ms: None,
            llm_timeout_ms: 1500,
            divergence_threshold: None,
            ring_capacity_samples: SAMPLE_RATE * 30,
            all_lowercase: false,
            user_names: Vec::new(),
            press_enter_enabled: true,
            email_signature: None,
            vad: true,
            hallucination_guard: true,
            llm_prefill: true,
            llm_skip_clean: true,
            spoken_cleanup: true,
            endpoint_silence_ms: None,
            segment_pause_ms: None,
            segment_min_ms: None,
            segment_soft_max_ms: None,
        }
    }
}

impl SessionConfig {
    /// Forced language, if exactly one non-"auto" language is configured.
    pub fn forced_language(&self) -> Option<&str> {
        match self.languages.as_slice() {
            [one] if one != "auto" => Some(one.as_str()),
            _ => None,
        }
    }
}

/// Focused-field context; may arrive after audio started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FieldContext {
    pub app_id: String,
    pub url_host: String,
    pub app_category: AppCategory,
    /// Text before the cursor (<= 500 chars, shell truncates).
    pub before: String,
    pub selected: String,
    pub after: String,
    /// Words visible on screen (proper-noun hints, ASR bias).
    pub screen_words: Vec<String>,
    /// Open file names in an IDE (Cursor/Windsurf @file tagging).
    pub file_names: Vec<String>,
    /// False when the shell could not read the field at all.
    pub context_known: bool,
}

impl Default for FieldContext {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            url_host: String::new(),
            app_category: AppCategory::Other,
            before: String::new(),
            selected: String::new(),
            after: String::new(),
            screen_words: Vec::new(),
            file_names: Vec::new(),
            context_known: true,
        }
    }
}

/// Which ASR engine a model runs on.
///
/// Adding a backend (e.g. H2's sherpa online / zipformer engine) takes
/// three small changes: a variant here, its [`AsrBackend::name`], and one
/// arm in `FlowCore`'s engine loader (`engine::load_asr`). Routing,
/// language reporting and the per-language map need nothing new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    Whisper,
    /// NeMo transducer (Parakeet-TDT) through sherpa-onnx (feature
    /// `parakeet`); `asr_path` is the model directory. The model's
    /// language is the one it is configured for (D33): English as the
    /// default model, the map key in [`ModelConfig::languages`] (e.g.
    /// Parakeet-v3-he).
    Parakeet,
    /// Returns a fixed transcript for any non-silent audio; for shell
    /// development and tests without a model.
    Mock {
        text: String,
    },
}

impl AsrBackend {
    /// Stable short name (`"whisper"`, `"parakeet"`, `"mock"`) for logs
    /// and status reports.
    pub fn name(&self) -> &'static str {
        match self {
            AsrBackend::Whisper => "whisper",
            AsrBackend::Parakeet => "parakeet",
            AsrBackend::Mock { .. } => "mock",
        }
    }
}

/// Parakeet search mode (`ModelConfig.parakeet_decoding`, 2026-10-23).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParakeetDecoding {
    /// Beam search (hotwords) only when the session has dictionary
    /// hotwords, greedy otherwise; an empty or implausibly short beam
    /// result is re-decoded greedy. Loads two recognizers (≈ +0.6 GB RSS).
    #[default]
    Auto,
    /// Greedy only: one recognizer, no hotword biasing.
    Greedy,
    /// Beam only (the pre-2026-10-23 behaviour, one recognizer); an empty
    /// result on speech is still re-decoded in pieces.
    Beam,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// `"whisper"` (default), `"parakeet"` or `{"mock":{"text":…}}`; also
    /// accepted as `asr_engine`.
    #[serde(alias = "asr_engine")]
    pub asr_backend: AsrBackend,
    pub asr_path: String,
    /// GGUF instruct model for LLM cleanup (feature `llm`); `None` = no LLM.
    pub llm_path: Option<String>,
    /// Path of the `flow-llm` plugin library; `None` = default lookup
    /// (see `flow_core::llama`).
    pub llm_lib: Option<String>,
    pub threads: usize,
    /// Optional Silero VAD model for whisper.cpp's built-in VAD (feature
    /// `whisper`; e.g. `ggml-silero-v5.1.2.bin`). `None` = energy VAD only.
    pub vad_model_path: Option<String>,
    /// Whisper: size the encoder window to the clip (`audio_ctx` = clip +
    /// 1 s) instead of always encoding 30 s, re-decoding with the full
    /// window when the output looks degenerate (repetition / too many
    /// words). Default on (D20: 40 LibriSpeech clips, tiny.en 2.7× / base.en
    /// 1.8× / small 3.4× faster, WER −0.4 / +0.3 / +0.1 points).
    pub whisper_fit_ctx: bool,
    /// Parakeet search mode (`auto` default; `greedy` saves the second
    /// recognizer's RAM on a phone). Omitted from JSON when `auto`.
    #[serde(skip_serializing_if = "is_auto")]
    pub parakeet_decoding: ParakeetDecoding,
    /// Per-language models (D33), keyed by ISO 639-1 code (`"he"`). A
    /// dictation whose forced language (`SessionConfig::forced_language`)
    /// has an entry decodes on that model; everything else, auto-detect
    /// included, on the default model above, as before. Empty (default) =
    /// one model; omitted from JSON when empty, so older shells and
    /// servers see the same config. TOML: `[model.languages.he]`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub languages: BTreeMap<String, LanguageModel>,
}

fn is_auto(d: &ParakeetDecoding) -> bool {
    *d == ParakeetDecoding::Auto
}

/// One entry of [`ModelConfig::languages`]: the ASR model for a language.
/// Fields mirror the default model's; the rest (`vad_model_path`,
/// `whisper_fit_ctx`, the LLM) is shared with the default model.
///
/// JSON: `{"asr_backend":"parakeet","asr_path":"/models/parakeet-he"}`
/// (`asr_engine` and `dir` / `path` are accepted as aliases).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LanguageModel {
    #[serde(alias = "asr_engine")]
    pub asr_backend: AsrBackend,
    /// Model file (whisper) or directory (sherpa-onnx backends).
    #[serde(alias = "dir", alias = "path")]
    pub asr_path: String,
    /// `None` = `ModelConfig::threads`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<usize>,
    #[serde(skip_serializing_if = "is_auto")]
    pub parakeet_decoding: ParakeetDecoding,
}

impl Default for LanguageModel {
    fn default() -> Self {
        Self {
            asr_backend: AsrBackend::Whisper,
            asr_path: String::new(),
            threads: None,
            parakeet_decoding: ParakeetDecoding::Auto,
        }
    }
}

impl ModelConfig {
    /// The key of [`ModelConfig::languages`] that serves a dictation in
    /// `language` (normally `SessionConfig::forced_language()`), or `None`
    /// when the default model does (no entry, or auto-detect). Matching
    /// uses [`primary_language`], so `"he-IL"`, `"iw"` and `"Hebrew"` find
    /// a `"he"` entry. Shells that keep one model in memory (Android, D33)
    /// compare this across language changes to know when to reload.
    pub fn language_route(&self, language: Option<&str>) -> Option<&str> {
        let want = primary_language(language?)?;
        self.languages
            .keys()
            .find(|k| primary_language(k).as_deref() == Some(want.as_str()))
            .map(String::as_str)
    }

    /// The default model as a [`LanguageModel`] (same fields).
    pub fn default_model(&self) -> LanguageModel {
        LanguageModel {
            asr_backend: self.asr_backend.clone(),
            asr_path: self.asr_path.clone(),
            threads: Some(self.threads),
            parakeet_decoding: self.parakeet_decoding,
        }
    }
}

/// Language names some engines report instead of codes (cloud STT's
/// `verbose_json` says `"english"`), and the display name used in the
/// cleanup prompt.
const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("en", "English"),
    ("he", "Hebrew"),
    ("ar", "Arabic"),
    ("ru", "Russian"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("uk", "Ukrainian"),
    ("tr", "Turkish"),
    ("zh", "Chinese"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("hi", "Hindi"),
];

/// The ISO 639-1 primary code of a language tag or name: lowercased,
/// region dropped (`"he-IL"` / `"he_IL"` → `"he"`), the legacy `"iw"` →
/// `"he"`, English names mapped (`"English"` → `"en"`). `None` for empty
/// and `"auto"`.
pub fn primary_language(tag: &str) -> Option<String> {
    let t = tag.trim().to_ascii_lowercase();
    let p = t.split(['-', '_']).next().unwrap_or("");
    match p {
        "" | "auto" => None,
        "iw" => Some("he".into()),
        _ => Some(
            LANGUAGE_NAMES
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(p))
                .map_or(p, |(c, _)| c)
                .to_string(),
        ),
    }
}

/// English display name of a language code (`"he"` → `"Hebrew"`), or the
/// code itself when the table does not know it.
pub fn language_name(code: &str) -> String {
    let p = primary_language(code).unwrap_or_default();
    LANGUAGE_NAMES
        .iter()
        .find(|(c, _)| *c == p)
        .map_or(p, |(_, n)| n.to_string())
}

/// Whether the English-only text rules (spoken cleanup and ITN, layout,
/// segment capitalisation, the guard's English word lists) apply to a
/// dictation the engine reported in `language` (D33): English, or unknown
/// (the default engine is English unless it says otherwise).
pub fn english_rules(language: Option<&str>) -> bool {
    language
        .and_then(primary_language)
        .is_none_or(|l| l == "en")
}

/// Half the logical CPUs (≈ physical cores), capped at 8: ggml's spinning
/// workers collapse when oversubscribed onto hyperthreads (measured 8 → 4
/// threads on a loaded i7-6700: 16.7 s → 1.2 s for 11 s of audio, tiny.en).
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).clamp(1, 8))
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            asr_backend: AsrBackend::Whisper,
            asr_path: String::new(),
            llm_path: None,
            llm_lib: None,
            threads: default_threads(),
            vad_model_path: None,
            whisper_fit_ctx: true,
            parakeet_decoding: ParakeetDecoding::Auto,
            languages: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_defaults_from_empty_json() {
        let c: SessionConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.cleanup_level, CleanupLevel::Light);
        assert!(!c.raw_mode);
        assert_eq!(c.platform, Platform::Windows);
        assert_eq!(c.max_samples, 16_000 * 60 * 20);
        assert!(c.languages.is_empty());
        assert_eq!(c.endpoint_silence_ms, None);
    }

    #[test]
    fn endpoint_silence_is_opt_in_and_round_trips() {
        // Off: not serialised, so the JSON an old server sees is unchanged.
        let off = serde_json::to_value(SessionConfig::default()).unwrap();
        assert!(off.get("endpoint_silence_ms").is_none());
        let c: SessionConfig = serde_json::from_str(r#"{"endpoint_silence_ms":1500}"#).unwrap();
        assert_eq!(c.endpoint_silence_ms, Some(1500));
        let back: SessionConfig =
            serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn segmentation_is_opt_in_and_round_trips() {
        let off = serde_json::to_value(SessionConfig::default()).unwrap();
        for k in ["segment_pause_ms", "segment_min_ms", "segment_soft_max_ms"] {
            assert!(off.get(k).is_none(), "{k}");
        }
        let c: SessionConfig = serde_json::from_str(
            r#"{"segment_pause_ms":800,"segment_min_ms":3000,"segment_soft_max_ms":15000}"#,
        )
        .unwrap();
        assert_eq!(c.segment_pause_ms, Some(800));
        assert_eq!(c.segment_min_ms, Some(3000));
        assert_eq!(c.segment_soft_max_ms, Some(15000));
        let back: SessionConfig =
            serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn session_config_parses_shell_json() {
        let c: SessionConfig = serde_json::from_str(
            r#"{"languages":["en"],"dictionary":[{"word":"wispr","replacement":"Wispr","isSnippet":false}],
                "cleanup_level":"medium","raw_mode":true,"platform":"android","max_samples":4800000,
                "styles":{"work":"casual"}}"#,
        )
        .unwrap();
        assert_eq!(c.platform, Platform::Android);
        assert_eq!(c.cleanup_level, CleanupLevel::Medium);
        assert_eq!(c.dictionary.entries.len(), 1);
        assert_eq!(c.styles.work, Style::Casual);
        assert_eq!(c.styles.email, Style::Default);
    }

    #[test]
    fn style_for_category_maps_buckets() {
        let s = Styles {
            personal: Style::VeryCasual,
            ..Default::default()
        };
        assert_eq!(s.for_category(AppCategory::PersonalChat), Style::VeryCasual);
        assert_eq!(s.for_category(AppCategory::Email), Style::Default);
        assert_eq!(s.for_category(AppCategory::Terminal), Style::Default);
    }

    #[test]
    fn asr_engine_selector_defaults_to_whisper() {
        let c: ModelConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.asr_backend, AsrBackend::Whisper);
        assert!(c.whisper_fit_ctx);
        for j in [
            r#"{"asr_engine":"parakeet"}"#,
            r#"{"asr_backend":"parakeet"}"#,
        ] {
            let c: ModelConfig = serde_json::from_str(j).unwrap();
            assert_eq!(c.asr_backend, AsrBackend::Parakeet);
        }
    }

    #[test]
    fn model_config_without_languages_is_unchanged() {
        // Old prefs / server JSON parses, and serialises without the new key.
        let old =
            r#"{"asr_backend":"parakeet","asr_path":"/m/p","threads":4,"whisper_fit_ctx":true}"#;
        let c: ModelConfig = serde_json::from_str(old).unwrap();
        assert!(c.languages.is_empty());
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("languages").is_none(), "{v}");
        assert_eq!(c.language_route(Some("he")), None);
    }

    #[test]
    fn language_map_parses_and_routes() {
        let c: ModelConfig = serde_json::from_str(
            r#"{"asr_backend":"parakeet","asr_path":"/m/en",
                "languages":{"he":{"asr_engine":"parakeet","dir":"/m/he","threads":2}}}"#,
        )
        .unwrap();
        let he = &c.languages["he"];
        assert_eq!(he.asr_backend, AsrBackend::Parakeet);
        assert_eq!(he.asr_path, "/m/he");
        assert_eq!(he.threads, Some(2));
        for l in ["he", "he-IL", "iw", "Hebrew", " HE "] {
            assert_eq!(c.language_route(Some(l)), Some("he"), "{l}");
        }
        for l in [None, Some("auto"), Some("en"), Some("de"), Some("")] {
            assert_eq!(c.language_route(l), None, "{l:?}");
        }
        let back: ModelConfig = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn language_helpers() {
        assert_eq!(primary_language("en-US").as_deref(), Some("en"));
        assert_eq!(primary_language("english").as_deref(), Some("en"));
        assert_eq!(primary_language("auto"), None);
        assert_eq!(language_name("he"), "Hebrew");
        assert_eq!(language_name("xx"), "xx");
        assert!(english_rules(None));
        assert!(english_rules(Some("en")));
        assert!(english_rules(Some("English")));
        assert!(!english_rules(Some("he")));
        assert!(!english_rules(Some("de")));
    }

    #[test]
    fn field_context_partial_json() {
        let c: FieldContext =
            serde_json::from_str(r#"{"app_id":"com.slack","before":"Hi "}"#).unwrap();
        assert_eq!(c.app_category, AppCategory::Other);
        assert_eq!(c.after, "");
    }
}
