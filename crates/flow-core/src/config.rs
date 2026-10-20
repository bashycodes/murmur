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
    #[default]
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
    /// Override the LLM divergence guardrail threshold (0..1).
    pub divergence_threshold: Option<f64>,
    /// Ring-buffer capacity between `push_audio` and the worker.
    pub ring_capacity_samples: usize,
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    Whisper,
    /// Returns a fixed transcript for any non-silent audio; for shell
    /// development and tests without a model.
    Mock {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub asr_backend: AsrBackend,
    pub asr_path: String,
    pub llm_path: Option<String>,
    pub threads: usize,
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
            threads: default_threads(),
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
        assert_eq!(c.styles.email, Style::Formal);
    }

    #[test]
    fn style_for_category_maps_buckets() {
        let s = Styles {
            personal: Style::VeryCasual,
            ..Default::default()
        };
        assert_eq!(s.for_category(AppCategory::PersonalChat), Style::VeryCasual);
        assert_eq!(s.for_category(AppCategory::Email), Style::Formal);
        assert_eq!(s.for_category(AppCategory::Terminal), Style::Formal);
    }

    #[test]
    fn field_context_partial_json() {
        let c: FieldContext =
            serde_json::from_str(r#"{"app_id":"com.slack","before":"Hi "}"#).unwrap();
        assert_eq!(c.app_category, AppCategory::Other);
        assert_eq!(c.after, "");
    }
}
