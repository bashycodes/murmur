//! Seam to `flow-core` (spec 10 §9 v2 API).
//!
//! The shell codes against [`CoreApi`] only. `flow-core` is being built on
//! another branch; integration = add the path dependency and implement this
//! trait for its handle in `core_adapter.rs` (one file). Until then
//! [`StubCore`] lets the shell run end-to-end (it reports what it heard as a
//! duration string, so hotkeys → capture → paste can be exercised on a real
//! Windows box without models).
//!
//! `load_models`/`unload` are adapter-construction concerns (the adapter owns
//! the handle), so they are not trait methods.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupLevel {
    None,
    #[default]
    Light,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleBucket {
    Personal,
    Work,
    Email,
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Styles {
    pub personal: String,
    pub work: String,
    pub email: String,
    pub other: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    pub languages: Vec<String>,
    pub dictionary: Vec<String>,
    pub snippets: BTreeMap<String, String>,
    pub replacements: BTreeMap<String, String>,
    pub styles: Styles,
    pub cleanup_level: CleanupLevel,
    pub raw_mode: bool,
    pub max_samples: u64,
}

/// `set_context` payload (≤500 chars each for before/selected/after).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FieldContext {
    pub app_id: String,
    pub url_host: Option<String>,
    pub app_category: StyleBucket,
    pub before: String,
    pub selected: String,
    pub after: String,
    pub screen_words: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CommitStatus {
    Formatted,
    RawTranscript,
    Empty,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Timings {
    pub asr_ms: u64,
    pub llm_ms: u64,
    pub total_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitResult {
    pub status: CommitStatus,
    pub asr_text: String,
    pub final_text: String,
    pub language: Option<String>,
    pub timings: Timings,
    /// Set when `status == Error`.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnCandidate {
    pub word: String,
    pub replacement: Option<String>,
}

pub trait CoreApi: Send + Sync {
    fn start_session(&self, cfg: SessionConfig) -> Result<(), String>;
    /// Non-blocking: copies into the core's ring buffer.
    fn push_audio(&self, pcm16_16k_mono: &[i16]);
    fn set_context(&self, ctx: FieldContext);
    fn partial(&self) -> Option<String>;
    /// Blocking (ASR + optional LLM); the shell calls it off the UI thread.
    fn commit(&self) -> CommitResult;
    fn cancel(&self);
    fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<LearnCandidate>;
}

/// Model-free stand-in for `flow-core`: counts samples and returns
/// "[stub: N.N s of audio]" so the full shell path can be exercised.
#[derive(Default)]
pub struct StubCore {
    inner: Mutex<StubState>,
}

#[derive(Default)]
struct StubState {
    active: bool,
    samples: u64,
    context: Option<FieldContext>,
}

impl StubCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_context(&self) -> Option<FieldContext> {
        self.inner.lock().unwrap().context.clone()
    }
}

impl CoreApi for StubCore {
    fn start_session(&self, _cfg: SessionConfig) -> Result<(), String> {
        let mut s = self.inner.lock().unwrap();
        *s = StubState {
            active: true,
            ..StubState::default()
        };
        Ok(())
    }

    fn push_audio(&self, pcm: &[i16]) {
        let mut s = self.inner.lock().unwrap();
        if s.active {
            s.samples += pcm.len() as u64;
        }
    }

    fn set_context(&self, ctx: FieldContext) {
        self.inner.lock().unwrap().context = Some(ctx);
    }

    fn partial(&self) -> Option<String> {
        None
    }

    fn commit(&self) -> CommitResult {
        let mut s = self.inner.lock().unwrap();
        s.active = false;
        let secs = s.samples as f64 / 16_000.0;
        if s.samples == 0 {
            return CommitResult {
                status: CommitStatus::Empty,
                asr_text: String::new(),
                final_text: String::new(),
                language: None,
                timings: Timings::default(),
                error: None,
            };
        }
        let text = format!("[stub: {secs:.1} s of audio]");
        CommitResult {
            status: CommitStatus::RawTranscript,
            asr_text: text.clone(),
            final_text: text,
            language: None,
            timings: Timings::default(),
            error: None,
        }
    }

    fn cancel(&self) {
        let mut s = self.inner.lock().unwrap();
        s.active = false;
        s.samples = 0;
    }

    fn learn_edit(&self, _pasted: &str, _edited: &str) -> Vec<LearnCandidate> {
        Vec::new()
    }
}

/// Map a core result onto the dictation machine's outcome.
pub fn to_outcome(r: &CommitResult) -> crate::dictation::CommitOutcome {
    use crate::dictation::CommitOutcome;
    match r.status {
        CommitStatus::Formatted | CommitStatus::RawTranscript if !r.final_text.is_empty() => {
            CommitOutcome::Text(r.final_text.clone())
        }
        CommitStatus::Error => {
            CommitOutcome::Failed(r.error.clone().unwrap_or_else(|| "core error".into()))
        }
        _ => CommitOutcome::Empty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::CommitOutcome;

    #[test]
    fn stub_reports_audio_length() {
        let c = StubCore::new();
        c.start_session(SessionConfig::default()).unwrap();
        c.push_audio(&[0; 16_000]);
        c.push_audio(&[0; 8_000]);
        let r = c.commit();
        assert_eq!(r.final_text, "[stub: 1.5 s of audio]");
        assert_eq!(to_outcome(&r), CommitOutcome::Text(r.final_text.clone()));
    }

    #[test]
    fn stub_empty_without_audio_and_ignores_audio_when_inactive() {
        let c = StubCore::new();
        c.push_audio(&[0; 100]);
        c.start_session(SessionConfig::default()).unwrap();
        assert_eq!(c.commit().status, CommitStatus::Empty);
    }

    #[test]
    fn outcome_mapping() {
        let mut r = CommitResult {
            status: CommitStatus::Error,
            asr_text: String::new(),
            final_text: String::new(),
            language: None,
            timings: Timings::default(),
            error: Some("boom".into()),
        };
        assert_eq!(to_outcome(&r), CommitOutcome::Failed("boom".into()));
        r.status = CommitStatus::Formatted;
        assert_eq!(to_outcome(&r), CommitOutcome::Empty, "formatted but empty");
    }

    #[test]
    fn commit_status_serialises_like_spec() {
        assert_eq!(
            serde_json::to_string(&CommitStatus::RawTranscript).unwrap(),
            "\"RAW_TRANSCRIPT\""
        );
    }
}
