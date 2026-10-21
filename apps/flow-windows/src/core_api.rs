//! Seam to `flow-core` (spec 10 §9 v2 API).
//!
//! The shell codes against [`CoreApi`] only; `core_adapter.rs` implements it
//! on top of `flow_core::FlowCore` (the shell's types are mapped there, so
//! controller tests never need a model). [`StubCore`] stays as a model-free
//! stand-in for tests: it reports what it heard as a duration string.
//!
//! `load_models`/`unload` are adapter-construction concerns (the adapter owns
//! the handle), so they are not trait methods.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// The paste plan (core-p1 CR-2) is plain data, so the shell reuses the
/// core's type rather than mirroring it: `{"text": ".."}` / `{"key": "Enter"|"Tab"|"Break"}`.
pub use flow_core::paste::{PasteKey, PasteSegment};
/// Which engine produced a result (D17), filled by the router.
pub use flow_core::RouteInfo;

/// Shared dictionary/snippet export shape (spec §9): `{word, replacement, isSnippet}`.
/// No replacement = vocabulary word (ASR bias); replacement = deterministic
/// rewrite; `isSnippet` = the replacement is an expansion body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictEntry {
    pub word: String,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub is_snippet: bool,
}

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

/// Finer app category than the style bucket (AI apps / terminals change
/// formatting, e.g. no trailing-period drop or raw paste in terminals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    Ai,
    Email,
    WorkMessaging,
    Messaging,
    Document,
    Terminal,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub languages: Vec<String>,
    /// Kept as one ordered list (the core's dictionary regex honours order).
    pub dictionary: Vec<DictEntry>,
    pub styles: Styles,
    pub cleanup_level: CleanupLevel,
    pub raw_mode: bool,
    pub max_samples: u64,
    /// Live preview: re-decode every N ms of new audio; `None` = off.
    pub partial_interval_ms: Option<u32>,
    /// LLM cleanup budget; 0 = core default (1.5 s).
    pub llm_timeout_ms: u64,
    /// core-p1 session flags (all default on, like the core).
    /// Energy VAD at commit: no speech → EMPTY `no_speech` without ASR.
    pub vad: bool,
    /// Drop Whisper's stock silence phrases.
    pub hallucination_guard: bool,
    /// Deterministic spoken cleanup (punctuation words, fillers,
    /// backtracking, ITN); user pref "Auto-format punctuation, fillers and
    /// corrections".
    pub spoken_cleanup: bool,
    /// LLM KV prefill while the user speaks.
    pub llm_prefill: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            languages: Vec::new(),
            dictionary: Vec::new(),
            styles: Styles::default(),
            cleanup_level: CleanupLevel::default(),
            raw_mode: false,
            max_samples: 0,
            partial_interval_ms: None,
            llm_timeout_ms: 0,
            vad: true,
            hallucination_guard: true,
            spoken_cleanup: true,
            llm_prefill: true,
        }
    }
}

/// `set_context` payload (≤500 chars each for before/selected/after).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FieldContext {
    pub app_id: String,
    pub url_host: Option<String>,
    pub app_category: StyleBucket,
    pub app_kind: AppKind,
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
    /// Spoken "press enter". Informational: the Enter is already the last
    /// entry of `segments`, so the shell never acts on this flag itself.
    pub press_enter: bool,
    /// Why the LLM output was not used (RAW_TRANSCRIPT fallbacks).
    pub fallback_reason: Option<String>,
    /// Set when `status == Error`.
    pub error: Option<String>,
    /// `final_text` as a paste plan (text runs, Tab/Break keys, trailing
    /// Enter). Empty when there is nothing to paste.
    pub segments: Vec<PasteSegment>,
    /// `final_text` without paste markers: history, paste/copy last.
    pub plain_text: String,
    /// Why an EMPTY result is empty (`no_audio`, `no_speech`, `blank_asr`,
    /// `hallucination`, `only_filler`).
    pub empty_reason: Option<String>,
    /// The session hit `max_samples`.
    pub truncated: bool,
    /// Engine that served the dictation and any fallback (router only).
    pub route: Option<RouteInfo>,
}

impl CommitResult {
    pub fn with_status(status: CommitStatus) -> Self {
        CommitResult {
            status,
            asr_text: String::new(),
            final_text: String::new(),
            language: None,
            timings: Timings::default(),
            press_enter: false,
            fallback_reason: None,
            error: None,
            segments: Vec::new(),
            plain_text: String::new(),
            empty_reason: None,
            truncated: false,
            route: None,
        }
    }

    /// A text result with its paste plan derived the way the core does it.
    pub fn text(status: CommitStatus, text: impl Into<String>, press_enter: bool) -> Self {
        let text = text.into();
        CommitResult {
            asr_text: text.clone(),
            segments: flow_core::paste::segments(&text, press_enter),
            plain_text: flow_core::paste::plain_text(&text),
            final_text: text,
            press_enter,
            ..Self::with_status(status)
        }
    }

    /// The paste plan; derived from `final_text` + `press_enter` for results
    /// that carry none (a pre-core-p1 producer).
    pub fn paste_segments(&self) -> Vec<PasteSegment> {
        if self.segments.is_empty() && !self.final_text.is_empty() {
            flow_core::paste::segments(&self.final_text, self.press_enter)
        } else {
            self.segments.clone()
        }
    }

    /// What the user ended up with, without markers.
    pub fn plain(&self) -> String {
        if self.plain_text.is_empty() {
            flow_core::paste::plain_text(&self.final_text)
        } else {
            self.plain_text.clone()
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        CommitResult {
            error: Some(message.into()),
            ..Self::with_status(CommitStatus::Error)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnCandidate {
    /// The word(s) the user typed: the proposed dictionary entry.
    pub word: String,
    /// What the dictation had produced in their place.
    pub heard: String,
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
            return CommitResult::with_status(CommitStatus::Empty);
        }
        CommitResult::text(
            CommitStatus::RawTranscript,
            format!("[stub: {secs:.1} s of audio]"),
            false,
        )
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
        CommitStatus::Empty => match empty_notice(r.empty_reason.as_deref()) {
            Some(n) => CommitOutcome::Notice(n.into()),
            None => CommitOutcome::Empty,
        },
        _ => CommitOutcome::Empty,
    }
}

/// Pill text for an EMPTY result, when the reason is worth telling: only a
/// press where the VAD heard no speech at all ("is the mic muted?"). Blank
/// ASR, hallucinations and filler-only presses rest silently.
pub fn empty_notice(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some("no_speech") => Some("No speech heard"),
        _ => None,
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
        let mut r = CommitResult::failed("boom");
        assert_eq!(to_outcome(&r), CommitOutcome::Failed("boom".into()));
        r.status = CommitStatus::Formatted;
        assert_eq!(to_outcome(&r), CommitOutcome::Empty, "formatted but empty");
    }

    #[test]
    fn no_speech_is_a_notice_other_empties_are_silent() {
        let mut r = CommitResult::with_status(CommitStatus::Empty);
        assert_eq!(to_outcome(&r), CommitOutcome::Empty);
        r.empty_reason = Some("no_speech".into());
        assert_eq!(
            to_outcome(&r),
            CommitOutcome::Notice("No speech heard".into())
        );
        for quiet in ["no_audio", "blank_asr", "hallucination", "only_filler"] {
            r.empty_reason = Some(quiet.into());
            assert_eq!(to_outcome(&r), CommitOutcome::Empty, "{quiet}");
        }
    }

    #[test]
    fn segments_fall_back_to_final_text_and_press_enter() {
        let mut r = CommitResult::with_status(CommitStatus::Formatted);
        r.final_text = "see @{{BREAK}}main.rs{{TAB}}".into();
        r.press_enter = true;
        let segs = r.paste_segments();
        assert_eq!(
            segs.last(),
            Some(&PasteSegment::Key {
                key: PasteKey::Enter
            })
        );
        assert!(segs.contains(&PasteSegment::Key { key: PasteKey::Tab }));
        assert_eq!(r.plain(), "see @main.rs");
        let t = CommitResult::text(CommitStatus::Formatted, "Hi.", false);
        assert_eq!(t.segments, vec![PasteSegment::Text { text: "Hi.".into() }]);
        assert_eq!(t.plain_text, "Hi.");
    }

    #[test]
    fn session_flags_default_on() {
        let c = SessionConfig::default();
        assert!(c.vad && c.hallucination_guard && c.spoken_cleanup && c.llm_prefill);
    }

    #[test]
    fn commit_status_serialises_like_spec() {
        assert_eq!(
            serde_json::to_string(&CommitStatus::RawTranscript).unwrap(),
            "\"RAW_TRANSCRIPT\""
        );
    }
}
