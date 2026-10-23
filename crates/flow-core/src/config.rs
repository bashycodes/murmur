//! Configuration and context types crossing the core API (§9 v2).
//!
//! All types are serde so the FFI/JNI layers can pass JSON verbatim.
//! Every field has a default so shells may send partial objects.

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
    /// Verbatim: no LLM pass.
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
    /// Skip the LLM entirely (Wispr "No AI formatting").
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
    /// Deterministic spoken cleanup before the LLM and post-processing:
    /// spoken punctuation, fillers, stutters, "scratch that"/"no wait"
    /// backtracking, money/time/date formatting (`postprocess::spoken`).
    /// Off at cleanup level `none`. Default on.
    pub spoken_cleanup: bool,
    /// Hands-free endpointing (D19): after at least 300 ms of speech, this
    /// much trailing silence sets `SessionPoll.endpoint_reached` (see
    /// `FlowCore::poll`). The core never stops by itself; the shell commits
    /// when it sees the signal. `None` (default) = off. Values below 300
    /// are raised to 300; `endpoint::RECOMMENDED_SILENCE_MS` = 2000.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    Whisper,
    /// Parakeet-TDT through sherpa-onnx (feature `parakeet`); `asr_path` is
    /// the model directory. English only.
    Parakeet,
    /// Returns a fixed transcript for any non-silent audio; for shell
    /// development and tests without a model.
    Mock {
        text: String,
    },
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
}

fn is_auto(d: &ParakeetDecoding) -> bool {
    *d == ParakeetDecoding::Auto
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
    fn field_context_partial_json() {
        let c: FieldContext =
            serde_json::from_str(r#"{"app_id":"com.slack","before":"Hi "}"#).unwrap();
        assert_eq!(c.app_category, AppCategory::Other);
        assert_eq!(c.after, "");
    }
}
