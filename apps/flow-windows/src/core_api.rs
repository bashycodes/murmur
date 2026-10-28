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
    /// Hands-free endpointing (D19): trailing silence after speech that
    /// ends the dictation. Set only for hands-free sessions; push-to-talk
    /// leaves it `None`.
    pub endpoint_silence_ms: Option<u32>,
    /// D22: decode each finished phrase while the user keeps talking; cuts
    /// at pauses at least this long (shell value 800). `None` = off.
    /// `segment_min_ms` / `segment_soft_max_ms` stay at the core defaults.
    pub segment_pause_ms: Option<u32>,
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
            endpoint_silence_ms: None,
            segment_pause_ms: None,
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
    /// Audio the engine received (after any fallback replay).
    pub audio_ms: u64,
    /// Samples lost because a buffer was full (0 = nothing lost).
    pub dropped_samples: u64,
    /// D22: `commit` waited this long for a decode already in flight.
    /// Included in `total_ms`.
    #[serde(default)]
    pub wait_ms: u64,
    /// D22: pieces that went to ASR (closed segments + tail); 0 = off.
    #[serde(default)]
    pub segments: u64,
    /// D22: ASR time while recording on pieces whose text was used (not
    /// part of `asr_ms`, which is now the tail decoded inside `commit`).
    #[serde(default)]
    pub pre_asr_ms: u64,
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
    /// D28: what the LLM step cost / which engine ran it (`None`: LLM off,
    /// skipped, or every cleaner refused up front).
    #[serde(default)]
    pub llm: Option<flow_core::cleanup::CleanUsage>,
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
            llm: None,
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

/// Where the live session runs (router `poll().route` /
/// `start_session_info().route`): `local` / `remote` / `cloud` + details.
pub use flow_router::SessionRoute;

/// One tick of the partial timer: preview text, hands-free endpoint (D19)
/// and the route in use right now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Poll {
    pub partial: Option<String>,
    /// Latched: the hands-free session heard speech followed by
    /// `endpoint_silence_ms` of silence; run the normal finish path.
    pub endpoint_reached: bool,
    /// `None` with no live session (or a bare core, which has no route).
    pub route: Option<SessionRoute>,
}

/// Recording-pill line for the live route: "Transcribing via this PC…",
/// "… via server…", "… via OpenRouter (openai/whisper-large-v3-turbo)…", plus a short
/// reason when the first route was skipped or failed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveRouteLabel {
    pub text: String,
    /// `local` / `remote` / `cloud` (styling).
    pub engine: String,
    /// Tooltip: server address, cloud provider host.
    pub title: Option<String>,
}

/// Short reason for a `SessionRoute.fallback` code (pill suffix).
pub fn fallback_reason(code: &str) -> &'static str {
    match code {
        "connect_failed" | "disconnected" | "timeout" => "server unreachable",
        "busy" => "server busy",
        "auth_failed" => "access rejected",
        "proto_mismatch" => "server version mismatch",
        "circuit_open" => "paused after recent failures",
        "server_not_configured" => "server not set up",
        "local_not_ready" => "local model not loaded",
        "cloud_not_configured" => "cloud not set up",
        "budget_exceeded" => "monthly budget reached",
        "quota_exceeded" => "out of credits",
        "provider_error" => "provider error",
        "no_private_route" => "no zero-retention provider",
        _ => "fell back",
    }
}

/// D27: what the LLM did for a result: `formatted` (its text was used), the
/// core `fallback_reason` (`llm_skipped_clean`, `llm_timeout`, …), or `None`
/// (LLM off, or no result). Debug log `llm`, Developer "Active engine".
pub fn llm_outcome(r: Option<&CommitResult>) -> Option<String> {
    let r = r?;
    if r.status == CommitStatus::Formatted {
        Some("formatted".into())
    } else {
        r.fallback_reason.clone()
    }
}

/// User-facing text for [`llm_outcome`]: `llm_skipped_clean` reads as a
/// normal result, not a failure (shared wording with Android).
pub fn llm_outcome_label(code: Option<&str>) -> Option<String> {
    Some(match code? {
        "formatted" => "AI cleaned".into(),
        "llm_skipped_clean" => "AI not needed (already clean)".into(),
        "llm_timeout" => "AI too slow, formatted text used".into(),
        other => format!("AI result not used ({other}), formatted text used"),
    })
}

/// `provider_name(id)` = the provider's display name and host (cloud).
pub fn live_route_label(
    r: &SessionRoute,
    provider_name: &dyn Fn(&str) -> Option<(String, String)>,
) -> LiveRouteLabel {
    let engine = r.engine.as_str().to_string();
    let (via, title) = match engine.as_str() {
        "remote" => (
            "server".to_string(),
            r.server.as_ref().map(|s| format!("Your flow-server {s}")),
        ),
        "cloud" => {
            let id = r.provider.clone().unwrap_or_default();
            let (name, host) = provider_name(&id).unwrap_or((id.clone(), String::new()));
            let via = match r.model.as_deref().filter(|m| !m.is_empty()) {
                Some(m) => format!("{name} ({m})"),
                None => name,
            };
            (
                via,
                (!host.is_empty()).then(|| format!("Audio is sent to {host}")),
            )
        }
        _ => ("this PC".to_string(), None),
    };
    let mut text = format!("Transcribing via {via}…");
    if let Some(code) = r.fallback.as_deref() {
        text.push_str(&format!(" ({})", fallback_reason(code)));
    }
    LiveRouteLabel {
        text,
        engine,
        title,
    }
}

pub trait CoreApi: Send + Sync {
    fn start_session(&self, cfg: SessionConfig) -> Result<(), String>;
    /// `start_session` that also reports the start route (router). Blocks
    /// (model-load wait, server connect): never call it on the UI thread.
    fn start_session_info(&self, cfg: SessionConfig) -> Result<Option<SessionRoute>, String> {
        self.start_session(cfg).map(|()| None)
    }
    /// Pill text for a live route (the router names cloud providers).
    fn describe_route(&self, r: &SessionRoute) -> LiveRouteLabel {
        live_route_label(r, &|_| None)
    }
    /// Partial + endpoint + live route in one call (the partial timer).
    fn poll(&self) -> Poll {
        Poll {
            partial: self.partial(),
            ..Poll::default()
        }
    }
    /// Non-blocking: copies into the core's ring buffer.
    fn push_audio(&self, pcm16_16k_mono: &[i16]);
    fn set_context(&self, ctx: FieldContext);
    fn partial(&self) -> Option<String>;
    /// Blocking (ASR + optional LLM); the shell calls it off the UI thread.
    fn commit(&self) -> CommitResult;
    fn cancel(&self);
    fn learn_edit(&self, pasted: &str, edited: &str) -> Vec<LearnCandidate>;
    /// Command Mode: run the spoken `instruction` on `selected` (`None` =
    /// write new text) with the cleanup LLM route (`flow_core::command`),
    /// blocking up to `timeout`. `Ok` = the text to paste, `Err` = pill
    /// text. Default: no LLM available.
    fn command(
        &self,
        instruction: &str,
        selected: Option<&str>,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        flow_core::command::run(None, instruction, selected, timeout).map_err(|e| e.to_string())
    }
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
    config: Option<SessionConfig>,
    /// What `poll()` answers while a session is active (tests).
    poll: Poll,
    start_route: Option<SessionRoute>,
    cancels: usize,
    starts: usize,
    /// Scripted Command Mode answer (`None` = the trait default, no LLM)
    /// and the (instruction, selection) pairs it was asked.
    command_reply: Option<Result<String, String>>,
    commands: Vec<(String, Option<String>)>,
}

impl StubCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_context(&self) -> Option<FieldContext> {
        self.inner.lock().unwrap().context.clone()
    }

    /// Config of the last `start_session`.
    pub fn last_config(&self) -> Option<SessionConfig> {
        self.inner.lock().unwrap().config.clone()
    }

    /// Script `poll()` (and the start route) for tests.
    pub fn set_poll(&self, poll: Poll) {
        self.inner.lock().unwrap().poll = poll;
    }

    pub fn set_start_route(&self, route: Option<SessionRoute>) {
        self.inner.lock().unwrap().start_route = route;
    }

    pub fn cancels(&self) -> usize {
        self.inner.lock().unwrap().cancels
    }

    pub fn starts(&self) -> usize {
        self.inner.lock().unwrap().starts
    }

    /// Script what `command()` answers.
    pub fn set_command_reply(&self, reply: Result<String, String>) {
        self.inner.lock().unwrap().command_reply = Some(reply);
    }

    /// (instruction, selection) of every `command()` call.
    pub fn commands(&self) -> Vec<(String, Option<String>)> {
        self.inner.lock().unwrap().commands.clone()
    }
}

impl CoreApi for StubCore {
    fn start_session(&self, cfg: SessionConfig) -> Result<(), String> {
        let mut s = self.inner.lock().unwrap();
        let old = std::mem::take(&mut *s);
        *s = StubState {
            active: true,
            config: Some(cfg),
            poll: old.poll,
            start_route: old.start_route,
            cancels: old.cancels,
            starts: old.starts + 1,
            command_reply: old.command_reply,
            commands: old.commands,
            ..StubState::default()
        };
        Ok(())
    }

    fn start_session_info(&self, cfg: SessionConfig) -> Result<Option<SessionRoute>, String> {
        self.start_session(cfg)?;
        Ok(self.inner.lock().unwrap().start_route.clone())
    }

    fn poll(&self) -> Poll {
        let s = self.inner.lock().unwrap();
        if s.active {
            s.poll.clone()
        } else {
            Poll::default()
        }
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
        s.cancels += 1;
    }

    fn learn_edit(&self, _pasted: &str, _edited: &str) -> Vec<LearnCandidate> {
        Vec::new()
    }

    fn command(
        &self,
        instruction: &str,
        selected: Option<&str>,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let reply = {
            let mut s = self.inner.lock().unwrap();
            s.commands
                .push((instruction.to_string(), selected.map(str::to_string)));
            s.command_reply.clone()
        };
        match reply {
            Some(r) => r,
            None => flow_core::command::run(None, instruction, selected, timeout)
                .map_err(|e| e.to_string()),
        }
    }
}

/// `CommitResult.error` of a commit aborted by `cancel()` (router C4).
pub const CANCELLED: &str = "cancelled";

/// Map a core result onto the dictation machine's outcome.
pub fn to_outcome(r: &CommitResult) -> crate::dictation::CommitOutcome {
    use crate::dictation::CommitOutcome;
    match r.status {
        CommitStatus::Formatted | CommitStatus::RawTranscript if !r.final_text.is_empty() => {
            CommitOutcome::Text(r.final_text.clone())
        }
        // Esc / the processing timeout aborted the commit (router C4):
        // dropped, nothing to paste, no error pill.
        CommitStatus::Error if r.error.as_deref() == Some(CANCELLED) => CommitOutcome::Empty,
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
    fn a_cancelled_commit_is_nothing_to_paste() {
        let r = CommitResult::failed("cancelled");
        assert_eq!(to_outcome(&r), CommitOutcome::Empty);
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

    #[test]
    fn live_route_labels() {
        use flow_router::EngineKind;
        let none = |_: &str| None;
        let local = SessionRoute::default();
        assert_eq!(
            live_route_label(&local, &none).text,
            "Transcribing via this PC…"
        );
        let remote = SessionRoute {
            engine: EngineKind::Remote,
            server: Some("100.64.0.2:7313".into()),
            ..Default::default()
        };
        let l = live_route_label(&remote, &none);
        assert_eq!(l.text, "Transcribing via server…");
        assert_eq!(l.engine, "remote");
        assert!(l.title.unwrap().contains("100.64.0.2:7313"));
        let cloud = SessionRoute {
            engine: EngineKind::Cloud,
            provider: Some("openrouter".into()),
            model: Some("openai/whisper-large-v3-turbo".into()),
            ..Default::default()
        };
        let named =
            |id: &str| (id == "openrouter").then(|| ("OpenRouter".into(), "openrouter.ai".into()));
        let l = live_route_label(&cloud, &named);
        assert_eq!(
            l.text,
            "Transcribing via OpenRouter (openai/whisper-large-v3-turbo)…"
        );
        assert_eq!(l.title.as_deref(), Some("Audio is sent to openrouter.ai"));
        let failed_over = SessionRoute {
            fallback: Some("disconnected".into()),
            ..Default::default()
        };
        assert_eq!(
            live_route_label(&failed_over, &none).text,
            "Transcribing via this PC… (server unreachable)"
        );
        let skipped = SessionRoute {
            fallback: Some("cloud_not_configured".into()),
            ..Default::default()
        };
        assert!(live_route_label(&skipped, &none)
            .text
            .ends_with("(cloud not set up)"));
    }
}
