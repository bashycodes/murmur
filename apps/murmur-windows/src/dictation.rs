//! Dictation state machine:
//! `idle → listening(ptt|locked|command) → processing → pasted|error → idle`.
//!
//! Command Mode: a `command` session's transcript is an
//! instruction, not text to paste. Processing then asks the controller to
//! run it on the selection (`Effect::Command`) and pastes the answer that
//! comes back (`Input::CommandFinished`); a failure pastes nothing. An
//! instruction that asks a question about the selection comes back as
//! `Input::CommandAnswered`: the answer card shows it and nothing is pasted.
//!
//! Transforms (docs/proposals/transforms.md): a transform shortcut goes
//! straight from rest to processing (`Effect::Transform`; nothing is
//! recorded) and its answer comes back as `CommandFinished` like a
//! command's. With auto-apply on, a dictation's text first goes through
//! the chosen transform (`Effect::AutoTransform` →
//! `Input::AutoTransformFinished`); on failure the untransformed text is
//! pasted with a "Transform skipped" notice, so a dictation is never lost.
//!
//! Translation (D90): a `translate` session records like push-to-talk;
//! its transcript goes through `Effect::Translate` (one LLM call that
//! cleans and translates) → `Input::TranslateFinished`. On failure the
//! untranslated text is pasted with a notice that says why (wrong script,
//! timeout, no AI), like a skipped auto-apply transform.
//!
//! Send to webhook (D105): a `webhook` session records like push-to-talk
//! and is cleaned up as usual, but its text goes out as
//! `Effect::Webhook` (an HTTP POST) instead of a paste; the answer comes
//! back as `Input::WebhookFinished` and the pill says sent or failed. A
//! POST cannot be called back: Esc (or the processing timeout) while it is
//! in flight ends the dictation, and its real outcome, when it arrives,
//! still shows ("Already sent to webhook", or the failure).
//! Nothing is pasted either way.
//!
//! Pure: inputs are hotkey actions, core/paste completions and clock ticks
//! (`now_ms` is passed in); outputs are `Effect`s the shell executes. Session
//! ids let late completions from a cancelled session be dropped.

use crate::hotkeys::HotkeyAction;
use serde::Serialize;

pub type SessionId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ptt,
    Locked,
    /// Command chord held: release = run the spoken instruction.
    Command,
    /// Translate chord held (D90): release = paste the dictation translated.
    Translate,
    /// Webhook chord held (D105): release = POST the dictation to the
    /// webhook URL instead of pasting it.
    Webhook,
}

impl Mode {
    /// Command and Translate: one AI call on the raw transcript, not a cleanup.
    pub fn is_one_call(self) -> bool {
        matches!(self, Mode::Command | Mode::Translate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listening {
    pub mode: Mode,
    pub session: SessionId,
    pub since_ms: u64,
    pub warned: bool,
    /// D92: when the push-to-talk key was released as a tap (held less
    /// than `debounce_ms`). Dismissed at `released_at + double_tap_gap_ms`
    /// unless pressed again sooner (→ double-tap lock).
    pub released_at: Option<u64>,
    /// When the session became locked (triple-press detection).
    pub locked_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Listening(Listening),
    Processing {
        session: SessionId,
        since_ms: u64,
        /// A Command Mode session (the transcript is an instruction).
        command: bool,
    },
    Pasted {
        since_ms: u64,
    },
    /// Error pill, or a notice (EMPTY with a reason), shown for `linger_ms`
    /// ([`Policy::notice_linger_ms`]) or until Esc or a new dictation.
    Error {
        since_ms: u64,
        linger_ms: u64,
    },
    /// Command Mode answered a question: the answer card shows for
    /// `linger_ms` or until Esc. Nothing was pasted.
    Answer {
        since_ms: u64,
        linger_ms: u64,
    },
}

/// Visual state of the overlay pill (the five states that matter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OverlayState {
    Resting,
    ActivePtt,
    ActiveLocked,
    /// Command chord held (listening to an instruction).
    ActiveCommand,
    Processing,
    /// Transcribing / running a Command Mode instruction.
    CommandProcessing,
    Error {
        message: String,
    },
    /// Informational pill text (no error sound), e.g. "No speech heard".
    Notice {
        message: String,
    },
    /// Command Mode's answer to a question about the selection, in a card
    /// above the pill (the selection is untouched).
    Answer {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    Start,
    Stop,
    Lock,
    Paste,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    Text(String),
    Empty,
    /// Nothing to paste, with a reason worth showing (EMPTY `no_speech`).
    Notice(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Hotkey(HotkeyAction),
    /// D92: a hotkey stamped `at_ms` (the controller's clock) when the
    /// keyboard hook delivered it. The controller turns it into `Hotkey`
    /// timed at that moment, so a busy controller (a mic opening) doesn't
    /// stretch a tap into a hold. The machine itself ignores it.
    HotkeyAt {
        action: HotkeyAction,
        at_ms: u64,
    },
    /// Periodic clock tick (drives max-duration and linger timeouts).
    Tick,
    Committed {
        session: SessionId,
        outcome: CommitOutcome,
    },
    PasteFinished {
        session: SessionId,
        result: Result<(), String>,
    },
    /// Overlay ✓ clicked: finish and paste (any listening mode).
    Finish,
    /// The background session start returned with a route to show (the
    /// controller handles it; the machine ignores it).
    Started {
        session: SessionId,
        route: crate::core_api::SessionRoute,
    },
    /// Mic couldn't be opened / core refused the session.
    StartFailed {
        session: SessionId,
        message: String,
    },
    /// A Command Mode instruction ran: the text to paste over the
    /// selection, or why there is none (pill text).
    CommandFinished {
        session: SessionId,
        result: Result<String, String>,
    },
    /// A Command Mode instruction asked a question: its answer, shown
    /// and never pasted.
    CommandAnswered {
        session: SessionId,
        answer: String,
    },
    /// D90: a translation ran on a dictation's `original` text: the
    /// translated text, or the notice that says why there is none
    /// (`TranslateError::notice`).
    TranslateFinished {
        session: SessionId,
        original: String,
        result: Result<String, String>,
    },
    /// D105: the webhook POST of a dictation finished (or was dropped
    /// before it went out).
    WebhookFinished {
        session: SessionId,
        outcome: crate::webhook::Outcome,
    },
    /// The auto-apply transform ran on a dictation's `original` text:
    /// the transformed text, or why it was skipped.
    AutoTransformFinished {
        session: SessionId,
        original: String,
        result: Result<String, String>,
    },
    /// History "Retry": re-transcribe row `row`'s kept recording with the
    /// settings active now (the controller handles it; the machine
    /// ignores it). Never pastes.
    Retry {
        row: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Sound(Sound),
    /// Mute/duck other apps' audio sessions.
    Duck,
    Unduck,
    /// Snapshot UIA context, start core session, start mic capture.
    BeginSession(SessionId),
    /// Stop mic capture (audio already pushed stays in the core).
    EndCapture,
    /// Ask the core for the final text (runs off-thread; reply = `Committed`).
    Commit(SessionId),
    CancelSession(SessionId),
    Paste {
        session: SessionId,
        text: String,
    },
    /// Run a Command Mode `instruction` on the selection (off-thread;
    /// reply = `CommandFinished`).
    Command {
        session: SessionId,
        instruction: String,
    },
    /// Run the transform at `index` on the selection once the shortcut is
    /// up (off-thread; reply = `CommandFinished`).
    Transform {
        session: SessionId,
        index: u8,
    },
    /// Run the auto-apply transform on a dictation's `text` before it is
    /// pasted (off-thread; reply = `AutoTransformFinished`).
    AutoTransform {
        session: SessionId,
        text: String,
    },
    /// D90: translate a dictation's `text` before it is pasted
    /// (off-thread; reply = `TranslateFinished`).
    Translate {
        session: SessionId,
        text: String,
    },
    /// D105: POST a dictation's cleaned `text` to the webhook URL instead
    /// of pasting it (off-thread; reply = `WebhookFinished`).
    Webhook {
        session: SessionId,
        text: String,
    },
    /// Max-duration about to be reached (overlay hint).
    MaxDurationWarning {
        remaining_ms: u64,
    },
    Overlay(OverlayState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Hard stop: the dictation is committed (not discarded) at this length.
    pub max_ms: u64,
    pub warn_ms: u64,
    /// Hold threshold: PTT released sooner = tap (dismissed unless a
    /// second press locks it); press sooner after locking = triple-press
    /// dismiss.
    pub debounce_ms: u64,
    /// D92: a second press this soon after a tap's *release* locks
    /// (hands-free). Measured release → press, not press → press, so a
    /// slow first tap still counts.
    /// A double tap made while the last dictation was still processing
    /// starts hands-free only if that processing ends within this gap of
    /// the second press too.
    pub double_tap_gap_ms: u64,
    /// Processing longer than this is an error (35 s).
    pub processing_timeout_ms: u64,
    /// D58: the budget instead when the stop language differs and the
    /// whole recording is transcribed again (as Android's retry budget).
    pub rerun_timeout_ms: u64,
    pub pasted_linger_ms: u64,
    /// An error or notice pill ("No speech heard", "… Typed as said.")
    /// shows at least this long (4 s), longer for long text, see
    /// [`Policy::notice_linger_ms`].
    pub notice_min_ms: u64,
    /// ... and at most this long (8 s).
    pub notice_max_ms: u64,
    /// A Command Mode answer card shows at least this long (longer
    /// answers get reading time, see [`Policy::answer_linger_ms`]).
    pub answer_min_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            max_ms: 20 * 60 * 1000,
            warn_ms: 19 * 60 * 1000,
            debounce_ms: 500,
            double_tap_gap_ms: 500,
            processing_timeout_ms: 35_000,
            rerun_timeout_ms: 90_000,
            pasted_linger_ms: 0,
            notice_min_ms: 4000,
            notice_max_ms: 8000,
            answer_min_ms: 8000,
        }
    }
}

impl Policy {
    /// `SessionConfig.max_samples` for the core (16 kHz mono).
    pub fn max_samples(&self) -> u64 {
        self.max_ms * 16
    }

    /// How long the card for `answer` stays up: reading time at
    /// [`READ_CHARS_PER_SEC`], between [`Policy::answer_min_ms`] and
    /// [`ANSWER_MAX_MS`].
    pub fn answer_linger_ms(&self, answer: &str) -> u64 {
        let read = answer.chars().count() as u64 * 1000 / READ_CHARS_PER_SEC;
        read.clamp(self.answer_min_ms, ANSWER_MAX_MS.max(self.answer_min_ms))
    }

    /// How long an error or notice pill with `message` stays up before it
    /// rests: [`Policy::notice_min_ms`], plus [`NOTICE_MS_PER_CHAR`] for
    /// every character past [`NOTICE_SHORT_CHARS`], at most
    /// [`Policy::notice_max_ms`]. Esc or a new dictation ends it sooner.
    pub fn notice_linger_ms(&self, message: &str) -> u64 {
        let extra = (message.chars().count() as u64).saturating_sub(NOTICE_SHORT_CHARS);
        (self.notice_min_ms + extra * NOTICE_MS_PER_CHAR)
            .min(self.notice_max_ms.max(self.notice_min_ms))
    }
}

/// An error / notice this long reads in [`Policy::notice_min_ms`].
pub const NOTICE_SHORT_CHARS: u64 = 40;

/// Extra time an error / notice stays up per character past
/// [`NOTICE_SHORT_CHARS`].
pub const NOTICE_MS_PER_CHAR: u64 = 50;

/// Longest an answer card stays up on its own (Esc closes it sooner).
pub const ANSWER_MAX_MS: u64 = 60_000;

/// Reading speed an answer card allows for (characters a second).
pub const READ_CHARS_PER_SEC: u64 = 15;

/// Pill text of a transform shortcut pressed with nothing selected.
pub const TRANSFORM_NO_SELECTION: &str = "Select text to transform";

/// Notice after an auto-apply transform failed and the untransformed text
/// was pasted.
pub const TRANSFORM_SKIPPED: &str = "Transform skipped";

/// D90: notice after a translation failed for no more specific reason
/// and the untranslated text was pasted (the core's
/// `TranslateError::notice` says why when it knows).
pub const TRANSLATE_SKIPPED: &str = murmur_core::translate::NOT_TRANSLATED_NOTICE;
/// D92: push-to-talk presses seen while no dictation could start
/// (processing), so a double tap begun then still locks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BusyTap {
    /// A press still held.
    down_at: Option<u64>,
    /// That press was released as a tap, at this time.
    released_at: Option<u64>,
    /// A second press came in time: lock once processing ends.
    lock_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Dictation {
    phase: Phase,
    busy_tap: BusyTap,
    policy: Policy,
    next_session: SessionId,
    /// Dictations go through the auto-apply transform before the paste.
    auto_transform: bool,
    /// A notice to show once this session's paste is done.
    after_paste: Option<(SessionId, String)>,
    /// D58: this session is transcribed again in its stop language
    /// (processing gets `rerun_timeout_ms`).
    rerun: Option<SessionId>,
    /// D90: this session's text is translated before the paste.
    translate: Option<SessionId>,
    /// D105: this session's text goes to the webhook, not the paste.
    webhook: Option<SessionId>,
    /// D105: this session's `Effect::Webhook` went out and its
    /// `WebhookFinished` has not come back yet.
    webhook_sending: Option<SessionId>,
}

impl Dictation {
    pub fn new(policy: Policy) -> Self {
        Dictation {
            phase: Phase::Idle,
            busy_tap: BusyTap::default(),
            policy,
            next_session: 1,
            auto_transform: false,
            after_paste: None,
            rerun: None,
            translate: None,
            webhook: None,
            webhook_sending: None,
        }
    }

    /// D58: `session` is transcribed again in its stop language: its
    /// processing runs on `rerun_timeout_ms` instead.
    pub fn rerun(&mut self, session: SessionId) {
        self.rerun = Some(session);
    }

    /// Auto-apply on or off (Transforms on and a transform chosen); read
    /// when a dictation's text arrives.
    pub fn set_auto_transform(&mut self, on: bool) {
        self.auto_transform = on;
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Listening or processing: Esc should be swallowed.
    pub fn is_active(&self) -> bool {
        matches!(self.phase, Phase::Listening(_) | Phase::Processing { .. })
    }

    pub fn handle(&mut self, input: Input, now_ms: u64) -> Vec<Effect> {
        // Esc or another key during push-to-talk: no pending tap survives
        // it, whatever the phase (a press right after must not lock).
        if matches!(
            input,
            Input::Hotkey(HotkeyAction::Cancel | HotkeyAction::PttAborted)
        ) {
            self.busy_tap = BusyTap::default();
        }
        match self.phase.clone() {
            Phase::Idle | Phase::Pasted { .. } | Phase::Error { .. } | Phase::Answer { .. } => {
                self.handle_resting(input, now_ms)
            }
            Phase::Listening(l) => self.handle_listening(l, input, now_ms),
            Phase::Processing {
                session,
                since_ms,
                command,
            } => {
                if let Input::Hotkey(a) = &input {
                    self.note_busy_tap(*a, now_ms);
                }
                let mut fx = self.handle_processing(session, since_ms, command, input, now_ms);
                fx.extend(self.start_queued_lock(now_ms));
                fx
            }
        }
    }

    /// D92: a push-to-talk press or release while processing (it can't
    /// start a dictation): remembered so a double tap still locks.
    fn note_busy_tap(&mut self, action: HotkeyAction, now_ms: u64) {
        let p = self.policy;
        let second = self.within_gap(self.busy_tap.released_at, now_ms);
        let t = &mut self.busy_tap;
        match action {
            // A third press dismisses the queued lock, as a triple press
            // dismisses a locked dictation.
            HotkeyAction::PttDown if t.lock_at.is_some() => *t = BusyTap::default(),
            HotkeyAction::PttDown => {
                *t = if second {
                    BusyTap {
                        lock_at: Some(now_ms),
                        ..BusyTap::default()
                    }
                } else {
                    BusyTap {
                        down_at: Some(now_ms),
                        ..BusyTap::default()
                    }
                };
            }
            HotkeyAction::PttUp => {
                if let Some(down) = t.down_at.take() {
                    if now_ms.saturating_sub(down) < p.debounce_ms {
                        t.released_at = Some(now_ms);
                    }
                }
            }
            // Another shortcut (Translate, Command, hands-free, a
            // transform; with Ctrl+Win push-to-talk their chords start
            // with a PttDown): the user meant that, not a double tap.
            HotkeyAction::TranslateDown
            | HotkeyAction::WebhookDown
            | HotkeyAction::CommandDown
            | HotkeyAction::HandsFreeToggle
            | HotkeyAction::Transform(_) => *t = BusyTap::default(),
            _ => {}
        }
    }

    /// D92: processing just ended in a paste (or nothing to paste) and a
    /// double tap came meanwhile: start hands-free now, if it was recent.
    /// An error or notice drops it (its pill stays readable).
    fn start_queued_lock(&mut self, now_ms: u64) -> Vec<Effect> {
        if !matches!(self.phase, Phase::Idle | Phase::Pasted { .. }) {
            if !matches!(self.phase, Phase::Processing { .. }) {
                self.busy_tap.lock_at = None;
            }
            return Vec::new();
        }
        match self.busy_tap.lock_at.take() {
            Some(at) if self.within_gap(Some(at), now_ms) => self.begin(Mode::Locked, now_ms),
            _ => Vec::new(),
        }
    }

    /// D92: `now_ms` is less than `double_tap_gap_ms` after `since`
    /// (a tap's release, or a queued lock's second press).
    fn within_gap(&self, since: Option<u64>, now_ms: u64) -> bool {
        since.is_some_and(|t| now_ms.saturating_sub(t) < self.policy.double_tap_gap_ms)
    }

    fn handle_resting(&mut self, input: Input, now_ms: u64) -> Vec<Effect> {
        let p = self.policy;
        match (&self.phase, input) {
            // D92: the second press of a double tap whose first tap came
            // while processing.
            (_, Input::Hotkey(HotkeyAction::PttDown))
                if self.within_gap(self.busy_tap.released_at, now_ms) =>
            {
                self.begin(Mode::Locked, now_ms)
            }
            (_, Input::Hotkey(HotkeyAction::PttDown)) => self.begin(Mode::Ptt, now_ms),
            // The release of a press made while processing.
            (_, Input::Hotkey(a @ HotkeyAction::PttUp)) => {
                self.note_busy_tap(a, now_ms);
                Vec::new()
            }
            (_, Input::Hotkey(HotkeyAction::HandsFreeToggle)) => self.begin(Mode::Locked, now_ms),
            (_, Input::Hotkey(HotkeyAction::CommandDown)) => self.begin(Mode::Command, now_ms),
            (_, Input::Hotkey(HotkeyAction::TranslateDown)) => self.begin(Mode::Translate, now_ms),
            (_, Input::Hotkey(HotkeyAction::WebhookDown)) => self.begin(Mode::Webhook, now_ms),
            (_, Input::Hotkey(HotkeyAction::Transform(index))) => {
                self.begin_transform(index, now_ms)
            }
            (Phase::Pasted { since_ms }, Input::Tick)
                if now_ms.saturating_sub(*since_ms) >= p.pasted_linger_ms =>
            {
                self.phase = Phase::Idle;
                Vec::new()
            }
            (
                Phase::Error {
                    since_ms,
                    linger_ms,
                },
                Input::Tick,
            ) if now_ms.saturating_sub(*since_ms) >= *linger_ms => {
                self.phase = Phase::Idle;
                vec![Effect::Overlay(OverlayState::Resting)]
            }
            (
                Phase::Answer {
                    since_ms,
                    linger_ms,
                },
                Input::Tick,
            ) if now_ms.saturating_sub(*since_ms) >= *linger_ms => {
                self.phase = Phase::Idle;
                vec![Effect::Overlay(OverlayState::Resting)]
            }
            // D105: a POST that was in flight when Esc (or the timeout)
            // ended its dictation: say what really happened.
            (_, Input::WebhookFinished { session, outcome })
                if self.webhook_sending == Some(session) =>
            {
                self.webhook_finished(outcome, crate::webhook::ALREADY_SENT, now_ms)
            }
            // Esc closes the answer card or an error / notice (the key
            // still reaches the app).
            (Phase::Answer { .. } | Phase::Error { .. }, Input::Hotkey(HotkeyAction::Cancel)) => {
                self.phase = Phase::Idle;
                vec![Effect::Overlay(OverlayState::Resting)]
            }
            _ => Vec::new(),
        }
    }

    fn handle_listening(&mut self, mut l: Listening, input: Input, now_ms: u64) -> Vec<Effect> {
        use HotkeyAction::*;
        let p = self.policy;
        let elapsed = now_ms.saturating_sub(l.since_ms);
        match (l.mode, input) {
            (_, Input::Hotkey(Cancel)) => self.cancel_listening(l.session),
            (_, Input::Finish) => self.stop_listening(&l, now_ms),
            (_, Input::StartFailed { session, message }) if session == l.session => {
                let mut fx = self.cancel_listening(l.session);
                fx.pop(); // replace the "resting" overlay with the error
                fx.extend(self.fail(message, now_ms));
                fx
            }

            // ---- push-to-talk ----
            // D92: the gap counts from the tap's release, not its press.
            (Mode::Ptt, Input::Hotkey(PttDown)) if l.released_at.is_some() => {
                if self.within_gap(l.released_at, now_ms) {
                    self.lock(l, now_ms)
                } else {
                    // Too late for a double tap: the tap is dismissed and
                    // this is a fresh press.
                    let mut fx = self.cancel_listening(l.session);
                    fx.extend(self.begin(Mode::Ptt, now_ms));
                    fx
                }
            }
            (Mode::Ptt, Input::Hotkey(PttUp)) if l.released_at.is_none() => {
                if elapsed < p.debounce_ms {
                    l.released_at = Some(now_ms);
                    self.phase = Phase::Listening(l);
                    Vec::new()
                } else {
                    self.stop(l.session, false, now_ms)
                }
            }
            (Mode::Ptt, Input::Hotkey(PttAborted)) => self.cancel_listening(l.session),
            (Mode::Ptt, Input::Hotkey(HandsFreeToggle)) => self.lock(l, now_ms),
            // Ctrl+Win, then Alt: the push-to-talk becomes a command.
            (Mode::Ptt, Input::Hotkey(CommandDown)) => {
                l.mode = Mode::Command;
                l.released_at = None;
                self.phase = Phase::Listening(l);
                vec![Effect::Overlay(OverlayState::ActiveCommand)]
            }

            // ---- command (held like push-to-talk) ----
            (Mode::Command, Input::Hotkey(CommandUp)) => {
                if elapsed < p.debounce_ms {
                    self.cancel_listening(l.session) // a tap: nothing said
                } else {
                    self.stop(l.session, true, now_ms)
                }
            }
            (Mode::Command, Input::Hotkey(PttAborted)) => self.cancel_listening(l.session),

            // ---- translate (held like push-to-talk, D90) ----
            // Ctrl+Win, then Shift: the push-to-talk becomes a translation.
            (Mode::Ptt, Input::Hotkey(TranslateDown)) => {
                l.mode = Mode::Translate;
                l.released_at = None;
                self.phase = Phase::Listening(l);
                vec![Effect::Overlay(OverlayState::ActiveCommand)]
            }
            (Mode::Translate, Input::Hotkey(TranslateUp)) => {
                if elapsed < p.debounce_ms {
                    self.cancel_listening(l.session) // a tap: nothing said
                } else {
                    self.stop_listening(&l, now_ms)
                }
            }
            (Mode::Translate, Input::Hotkey(PttAborted)) => self.cancel_listening(l.session),

            // ---- webhook (held like push-to-talk, D105) ----
            // Push-to-talk, then the rest of the webhook chord.
            (Mode::Ptt, Input::Hotkey(WebhookDown)) => {
                l.mode = Mode::Webhook;
                l.released_at = None;
                self.phase = Phase::Listening(l);
                Vec::new()
            }
            (Mode::Webhook, Input::Hotkey(WebhookUp)) => {
                if elapsed < p.debounce_ms {
                    self.cancel_listening(l.session) // a tap: nothing said
                } else {
                    self.stop_listening(&l, now_ms)
                }
            }
            (Mode::Webhook, Input::Hotkey(PttAborted)) => self.cancel_listening(l.session),

            // ---- locked (hands-free) ----
            (Mode::Locked, Input::Hotkey(PttDown)) => {
                let since_lock = now_ms.saturating_sub(l.locked_at.unwrap_or(l.since_ms));
                if since_lock < p.debounce_ms {
                    self.cancel_listening(l.session) // triple press
                } else {
                    self.stop(l.session, false, now_ms)
                }
            }
            (Mode::Locked, Input::Hotkey(HandsFreeToggle)) => {
                if elapsed < p.debounce_ms {
                    self.cancel_listening(l.session) // quick double
                } else {
                    self.stop(l.session, false, now_ms)
                }
            }

            // ---- clock ----
            (_, Input::Tick) => {
                if l.released_at.is_some() && !self.within_gap(l.released_at, now_ms) {
                    self.cancel_listening(l.session)
                } else if elapsed >= p.max_ms {
                    self.stop_listening(&l, now_ms)
                } else if elapsed >= p.warn_ms && !l.warned {
                    l.warned = true;
                    self.phase = Phase::Listening(l);
                    vec![Effect::MaxDurationWarning {
                        remaining_ms: p.max_ms - elapsed,
                    }]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn processing_limit(&self, session: SessionId) -> u64 {
        if self.rerun == Some(session) {
            self.policy.rerun_timeout_ms
        } else {
            self.policy.processing_timeout_ms
        }
    }

    fn handle_processing(
        &mut self,
        session: SessionId,
        since_ms: u64,
        command: bool,
        input: Input,
        now_ms: u64,
    ) -> Vec<Effect> {
        match input {
            Input::Hotkey(HotkeyAction::Cancel) => {
                self.phase = Phase::Idle;
                vec![
                    Effect::CancelSession(session),
                    Effect::Overlay(OverlayState::Resting),
                ]
            }
            Input::Committed {
                session: s,
                outcome,
            } if s == session && command => match outcome {
                CommitOutcome::Text(instruction) => vec![Effect::Command {
                    session,
                    instruction,
                }],
                CommitOutcome::Empty => self.notice("No instruction heard".into(), now_ms),
                CommitOutcome::Notice(message) => self.notice(message, now_ms),
                CommitOutcome::Failed(message) => self.fail(message, now_ms),
            },
            Input::CommandFinished { session: s, result } if s == session && command => {
                match result {
                    Ok(text) => vec![Effect::Paste { session, text }],
                    Err(message) if message == TRANSFORM_NO_SELECTION => {
                        self.notice(message, now_ms)
                    }
                    Err(message) => self.fail(message, now_ms),
                }
            }
            Input::CommandAnswered { session: s, answer } if s == session && command => {
                self.phase = Phase::Answer {
                    since_ms: now_ms,
                    linger_ms: self.policy.answer_linger_ms(&answer),
                };
                vec![Effect::Overlay(OverlayState::Answer { message: answer })]
            }
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text(text),
            } if s == session && self.translate == Some(session) => {
                vec![Effect::Translate { session, text }]
            }
            Input::TranslateFinished {
                session: s,
                original,
                result,
            } if s == session && self.translate == Some(session) => match result {
                Ok(text) => vec![Effect::Paste { session, text }],
                Err(notice) => {
                    self.after_paste = Some((session, notice));
                    vec![Effect::Paste {
                        session,
                        text: original,
                    }]
                }
            },
            // D105: the cleaned text goes to the webhook (no auto-apply
            // transform), never to the paste.
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text(text),
            } if s == session && self.webhook == Some(session) => {
                self.webhook_sending = Some(session);
                vec![Effect::Webhook { session, text }]
            }
            Input::WebhookFinished {
                session: s,
                outcome,
            } if s == session && self.webhook_sending == Some(session) => {
                self.webhook_finished(outcome, crate::webhook::SENT, now_ms)
            }
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text(text),
            } if s == session && self.auto_transform => {
                vec![Effect::AutoTransform { session, text }]
            }
            Input::AutoTransformFinished {
                session: s,
                original,
                result,
            } if s == session && !command => match result {
                Ok(text) => vec![Effect::Paste { session, text }],
                Err(_) => {
                    self.after_paste = Some((session, TRANSFORM_SKIPPED.into()));
                    vec![Effect::Paste {
                        session,
                        text: original,
                    }]
                }
            },
            Input::Committed {
                session: s,
                outcome,
            } if s == session => match outcome {
                CommitOutcome::Text(text) => vec![Effect::Paste { session, text }],
                CommitOutcome::Empty => {
                    self.phase = Phase::Idle;
                    vec![Effect::Overlay(OverlayState::Resting)]
                }
                CommitOutcome::Notice(message) => self.notice(message, now_ms),
                CommitOutcome::Failed(message) => self.fail(message, now_ms),
            },
            Input::PasteFinished { session: s, result } if s == session => match result {
                // No success pill: back to rest, the paste sound is the feedback.
                Ok(()) => {
                    if let Some((_, message)) =
                        self.after_paste.take().filter(|(s, _)| *s == session)
                    {
                        let mut fx = vec![Effect::Sound(Sound::Paste)];
                        fx.extend(self.notice(message, now_ms));
                        return fx;
                    }
                    self.phase = Phase::Pasted { since_ms: now_ms };
                    vec![
                        Effect::Sound(Sound::Paste),
                        Effect::Overlay(OverlayState::Resting),
                    ]
                }
                Err(message) => self.fail(message, now_ms),
            },
            Input::Tick if now_ms.saturating_sub(since_ms) >= self.processing_limit(session) => {
                let mut fx = vec![Effect::CancelSession(session)];
                let why = if command {
                    "Command timed out"
                } else if self.translate == Some(session) {
                    "Translation timed out"
                } else if self.webhook_sending == Some(session) {
                    "Webhook timed out"
                } else {
                    "Transcription timed out"
                };
                fx.extend(self.fail(why.into(), now_ms));
                fx
            }
            _ => Vec::new(),
        }
    }

    fn begin(&mut self, mode: Mode, now_ms: u64) -> Vec<Effect> {
        self.busy_tap = BusyTap::default();
        let session = self.next_session;
        self.next_session += 1;
        self.phase = Phase::Listening(Listening {
            mode,
            session,
            since_ms: now_ms,
            warned: false,
            released_at: None,
            locked_at: (mode == Mode::Locked).then_some(now_ms),
        });
        let (sound, overlay) = match mode {
            Mode::Ptt => (Sound::Start, OverlayState::ActivePtt),
            Mode::Locked => (Sound::Lock, OverlayState::ActiveLocked),
            // D105: dictated as usual; only where the text goes differs.
            Mode::Webhook => (Sound::Start, OverlayState::ActivePtt),
            _ => (Sound::Start, OverlayState::ActiveCommand),
        };
        vec![
            Effect::Sound(sound),
            Effect::Duck,
            Effect::BeginSession(session),
            Effect::Overlay(overlay),
        ]
    }

    /// A transform shortcut: no recording, straight to processing.
    fn begin_transform(&mut self, index: u8, now_ms: u64) -> Vec<Effect> {
        let session = self.next_session;
        self.next_session += 1;
        self.phase = Phase::Processing {
            session,
            since_ms: now_ms,
            command: true,
        };
        vec![
            Effect::Overlay(OverlayState::CommandProcessing),
            Effect::Transform { session, index },
        ]
    }

    fn lock(&mut self, mut l: Listening, now_ms: u64) -> Vec<Effect> {
        l.mode = Mode::Locked;
        l.released_at = None;
        l.locked_at = Some(now_ms);
        self.phase = Phase::Listening(l);
        vec![
            Effect::Sound(Sound::Lock),
            Effect::Overlay(OverlayState::ActiveLocked),
        ]
    }

    /// Stop listening and commit, as the session's mode wants it.
    fn stop_listening(&mut self, l: &Listening, now_ms: u64) -> Vec<Effect> {
        if l.mode == Mode::Translate {
            self.translate = Some(l.session);
            let mut fx = self.stop(l.session, false, now_ms);
            if let Some(Effect::Overlay(o)) = fx.last_mut() {
                *o = OverlayState::CommandProcessing;
            }
            return fx;
        }
        if l.mode == Mode::Webhook {
            self.webhook = Some(l.session);
        }
        self.stop(l.session, l.mode == Mode::Command, now_ms)
    }

    fn stop(&mut self, session: SessionId, command: bool, now_ms: u64) -> Vec<Effect> {
        self.phase = Phase::Processing {
            session,
            since_ms: now_ms,
            command,
        };
        vec![
            Effect::EndCapture,
            Effect::Unduck,
            Effect::Sound(Sound::Stop),
            Effect::Commit(session),
            Effect::Overlay(if command {
                OverlayState::CommandProcessing
            } else {
                OverlayState::Processing
            }),
        ]
    }

    /// D105: the pill for a webhook outcome (`sent` = its text on a 2xx).
    fn webhook_finished(
        &mut self,
        outcome: crate::webhook::Outcome,
        sent: &str,
        now_ms: u64,
    ) -> Vec<Effect> {
        self.webhook_sending = None;
        if outcome == crate::webhook::Outcome::Sent {
            let mut fx = vec![Effect::Sound(Sound::Paste)];
            fx.extend(self.notice(sent.into(), now_ms));
            return fx;
        }
        let Some(message) = outcome.failure() else {
            // Dropped before it went out: nothing to say.
            self.phase = Phase::Idle;
            return vec![Effect::Overlay(OverlayState::Resting)];
        };
        self.fail(message, now_ms)
    }

    /// Informational pill (no error sound) for
    /// [`Policy::notice_linger_ms`].
    fn notice(&mut self, message: String, now_ms: u64) -> Vec<Effect> {
        self.phase = Phase::Error {
            since_ms: now_ms,
            linger_ms: self.policy.notice_linger_ms(&message),
        };
        vec![Effect::Overlay(OverlayState::Notice { message })]
    }

    fn cancel_listening(&mut self, session: SessionId) -> Vec<Effect> {
        self.phase = Phase::Idle;
        vec![
            Effect::EndCapture,
            Effect::Unduck,
            Effect::CancelSession(session),
            Effect::Overlay(OverlayState::Resting),
        ]
    }

    /// Error pill (with the error sound) for [`Policy::notice_linger_ms`].
    fn fail(&mut self, message: String, now_ms: u64) -> Vec<Effect> {
        self.phase = Phase::Error {
            since_ms: now_ms,
            linger_ms: self.policy.notice_linger_ms(&message),
        };
        vec![
            Effect::Sound(Sound::Error),
            Effect::Overlay(OverlayState::Error { message }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyAction::*;

    fn d() -> Dictation {
        Dictation::new(Policy::default())
    }

    fn hk(d: &mut Dictation, a: HotkeyAction, t: u64) -> Vec<Effect> {
        d.handle(Input::Hotkey(a), t)
    }

    fn session_of(fx: &[Effect]) -> SessionId {
        fx.iter()
            .find_map(|e| match e {
                Effect::BeginSession(s) => Some(*s),
                _ => None,
            })
            .expect("BeginSession")
    }

    fn listening(d: &Dictation) -> &Listening {
        match d.phase() {
            Phase::Listening(l) => l,
            other => panic!("not listening: {other:?}"),
        }
    }

    fn cancel_fx(s: SessionId) -> Vec<Effect> {
        vec![
            Effect::EndCapture,
            Effect::Unduck,
            Effect::CancelSession(s),
            Effect::Overlay(OverlayState::Resting),
        ]
    }

    fn lock_fx() -> Vec<Effect> {
        vec![
            Effect::Sound(Sound::Lock),
            Effect::Overlay(OverlayState::ActiveLocked),
        ]
    }

    fn processing(d: &Dictation) -> SessionId {
        match d.phase() {
            Phase::Processing { session, .. } => *session,
            other => panic!("not processing: {other:?}"),
        }
    }

    #[test]
    fn ptt_down_starts_listening_with_sound_before_duck() {
        let mut d = d();
        let fx = hk(&mut d, PttDown, 0);
        let s = session_of(&fx);
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Start),
                Effect::Duck,
                Effect::BeginSession(s),
                Effect::Overlay(OverlayState::ActivePtt),
            ]
        );
        assert_eq!(listening(&d).mode, Mode::Ptt);
        assert!(d.is_active());
    }

    #[test]
    fn ptt_hold_past_debounce_then_release_commits() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        let fx = hk(&mut d, PttUp, 500);
        assert_eq!(
            fx,
            vec![
                Effect::EndCapture,
                Effect::Unduck,
                Effect::Sound(Sound::Stop),
                Effect::Commit(s),
                Effect::Overlay(OverlayState::Processing),
            ]
        );
        assert_eq!(
            d.phase(),
            &Phase::Processing {
                session: s,
                since_ms: 500,
                command: false,
            }
        );
    }

    #[test]
    fn short_tap_waits_out_debounce_then_dismisses() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 1000));
        assert!(
            hk(&mut d, PttUp, 1100).is_empty(),
            "keeps listening for a 2nd tap"
        );
        assert_eq!(listening(&d).released_at, Some(1100));
        // D92: the double-tap gap runs from the release (1100 + 500).
        assert!(d.handle(Input::Tick, 1599).is_empty());
        assert_eq!(d.handle(Input::Tick, 1600), cancel_fx(s));
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn double_tap_locks_same_session() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        assert_eq!(hk(&mut d, PttDown, 300), lock_fx());
        let l = listening(&d);
        assert_eq!(
            (l.mode, l.session, l.locked_at),
            (Mode::Locked, s, Some(300))
        );
        assert!(
            hk(&mut d, PttUp, 350).is_empty(),
            "release ignored when locked"
        );
        assert!(
            d.handle(Input::Tick, 2000).is_empty(),
            "no dismiss once locked"
        );
    }

    #[test]
    fn triple_press_dismisses() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        hk(&mut d, PttDown, 300);
        hk(&mut d, PttUp, 350);
        assert_eq!(hk(&mut d, PttDown, 700), cancel_fx(s));
    }

    #[test]
    fn locked_ptt_press_after_debounce_commits() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        hk(&mut d, PttDown, 300);
        let fx = hk(&mut d, PttDown, 5000);
        assert!(fx.contains(&Effect::Commit(s)));
    }

    #[test]
    fn late_second_tap_dismisses_and_starts_fresh_press() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        // No tick arrived yet: 500 ms after the release is too late.
        let fx = hk(&mut d, PttDown, 600);
        assert_eq!(fx[..4], cancel_fx(s)[..]);
        let fresh = session_of(&fx);
        assert_ne!(fresh, s);
        let l = listening(&d);
        assert_eq!((l.mode, l.session), (Mode::Ptt, fresh));
    }

    #[test]
    fn hands_free_chord_on_top_of_ptt_locks() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert_eq!(hk(&mut d, HandsFreeToggle, 40), lock_fx());
        assert_eq!(listening(&d).session, s);
        assert!(hk(&mut d, PttUp, 200).is_empty());
        // Stopping with the same chord: Ctrl+Win (PttDown) arrives first.
        let fx = hk(&mut d, PttDown, 4000);
        assert!(fx.contains(&Effect::Commit(s)));
        assert!(
            hk(&mut d, HandsFreeToggle, 4010).is_empty(),
            "ignored while processing"
        );
    }

    #[test]
    fn hands_free_from_idle() {
        let mut d = d();
        let fx = hk(&mut d, HandsFreeToggle, 0);
        let s = session_of(&fx);
        assert_eq!(fx[0], Effect::Sound(Sound::Lock));
        assert_eq!(fx[1], Effect::Duck);
        assert_eq!(fx[3], Effect::Overlay(OverlayState::ActiveLocked));
        assert!(hk(&mut d, PttAborted, 70).is_empty());
        let fx = hk(&mut d, HandsFreeToggle, 3000);
        assert!(fx.contains(&Effect::Commit(s)));
    }

    #[test]
    fn hands_free_quick_double_dismisses() {
        let mut d = d();
        let s = session_of(&hk(&mut d, HandsFreeToggle, 0));
        assert_eq!(hk(&mut d, HandsFreeToggle, 300), cancel_fx(s));
    }

    #[test]
    fn cancel_while_listening() {
        let mut d = d();
        let s = session_of(&hk(&mut d, HandsFreeToggle, 0));
        assert_eq!(hk(&mut d, Cancel, 5000), cancel_fx(s));
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn ptt_aborted_cancels() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert_eq!(hk(&mut d, PttAborted, 3000), cancel_fx(s));
    }

    #[test]
    fn commit_text_pastes_then_rests_without_success_state() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("hi".into()),
            },
            1500,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "hi".into()
            }]
        );
        assert!(d.is_active(), "still processing while inserting");
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            1700,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
        assert!(!d.is_active());
        d.handle(Input::Tick, 1700);
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn empty_commit_returns_to_idle_without_paste() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Empty,
            },
            1200,
        );
        assert_eq!(fx, vec![Effect::Overlay(OverlayState::Resting)]);
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn failed_commit_shows_error_for_500ms() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Failed("asr".into()),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Error),
                Effect::Overlay(OverlayState::Error {
                    message: "asr".into()
                })
            ]
        );
        assert!(d.handle(Input::Tick, 5199).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 5200),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
    }

    // --- Error / notice hold: readable before the bar rests (and hides). ---

    /// Dictate from `t`, then fail the start with `message`; returns the
    /// time of the error.
    fn start_fails(d: &mut Dictation, message: &str, t: u64) -> u64 {
        let s = session_of(&hk(d, PttDown, t));
        let fx = d.handle(
            Input::StartFailed {
                session: s,
                message: message.into(),
            },
            t + 10,
        );
        assert!(fx.contains(&Effect::Overlay(OverlayState::Error {
            message: message.into()
        })));
        t + 10
    }

    #[test]
    fn notice_hold_is_4s_plus_50ms_a_char_past_40_capped_at_8s() {
        let p = Policy::default();
        assert_eq!(p.notice_linger_ms("No speech heard"), 4000);
        assert_eq!(p.notice_linger_ms(&"x".repeat(40)), 4000);
        assert_eq!(p.notice_linger_ms(&"x".repeat(41)), 4050);
        assert_eq!(p.notice_linger_ms(&"x".repeat(60)), 5000);
        assert_eq!(p.notice_linger_ms(&"x".repeat(120)), 8000);
        assert_eq!(p.notice_linger_ms(&"x".repeat(5000)), 8000);
        // Characters, not bytes.
        assert_eq!(p.notice_linger_ms(&"é".repeat(40)), 4000);
    }

    #[test]
    fn an_error_then_idle_waits_for_the_hold() {
        let mut d = d();
        let t = start_fails(&mut d, "Microphone unavailable", 0);
        // The ticks of the old 500 ms linger no longer rest the pill.
        for dt in [500, 1500, 3999] {
            assert!(d.handle(Input::Tick, t + dt).is_empty(), "{dt}");
        }
        assert!(matches!(d.phase(), Phase::Error { .. }));
        assert_eq!(
            d.handle(Input::Tick, t + 4000),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn a_long_error_stays_longer() {
        let mut d = d();
        let msg = "The server refused the request; check the token in Settings";
        let hold = 4000 + (msg.chars().count() as u64 - 40) * 50;
        let t = start_fails(&mut d, msg, 0);
        assert!(d.handle(Input::Tick, t + hold - 1).is_empty());
        assert_eq!(
            d.handle(Input::Tick, t + hold),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn a_new_dictation_replaces_an_error_at_once() {
        let mut d = d();
        let t = start_fails(&mut d, "Microphone unavailable", 0);
        let fx = hk(&mut d, PttDown, t + 1000);
        assert!(
            fx.contains(&Effect::Overlay(OverlayState::ActivePtt)),
            "{fx:?}"
        );
        assert!(matches!(d.phase(), Phase::Listening(_)));
        // The old error's hold ending does not rest the new dictation.
        assert!(d.handle(Input::Tick, t + 4000).is_empty());
    }

    #[test]
    fn a_second_error_gets_its_own_hold() {
        let mut d = d();
        let t1 = start_fails(&mut d, "first", 0);
        let t2 = start_fails(&mut d, "second", t1 + 2000);
        // The first error's hold would have ended here.
        assert!(d.handle(Input::Tick, t1 + 4000).is_empty());
        assert!(d.handle(Input::Tick, t2 + 3999).is_empty());
        assert_eq!(
            d.handle(Input::Tick, t2 + 4000),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn esc_dismisses_an_error() {
        let mut d = d();
        let t = start_fails(&mut d, "Microphone unavailable", 0);
        assert_eq!(
            hk(&mut d, Cancel, t + 500),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
        assert!(d.handle(Input::Tick, t + 4000).is_empty());
    }

    #[test]
    fn a_typed_as_said_notice_gets_the_same_hold() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let msg = "Not translated. Typed as said.";
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Notice(msg.into()),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![Effect::Overlay(OverlayState::Notice {
                message: msg.into()
            })]
        );
        assert!(d.handle(Input::Tick, 1200 + 3999).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 1200 + 4000),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn the_hold_is_the_same_with_the_always_shown_bar() {
        // The machine owns the hold; "always show" only keeps the window up
        // after it rests (crate::overlay::bar_wanted).
        let prefs = crate::prefs::Prefs {
            show_murmur_bar_always: true,
            ..Default::default()
        };
        let mut d = Dictation::new(prefs.policy());
        let t = start_fails(&mut d, "Microphone unavailable", 0);
        assert!(d.handle(Input::Tick, t + 3999).is_empty());
        assert_eq!(
            d.handle(Input::Tick, t + 4000),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn paste_failure_is_error() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("x".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Err("uipi".into()),
            },
            1200,
        );
        assert_eq!(fx[0], Effect::Sound(Sound::Error));
        assert!(matches!(d.phase(), Phase::Error { .. }));
    }

    #[test]
    fn processing_times_out_after_35s() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        assert!(d.handle(Input::Tick, 35_999).is_empty());
        let fx = d.handle(Input::Tick, 36_000);
        assert_eq!(fx[0], Effect::CancelSession(s));
        assert!(matches!(d.phase(), Phase::Error { .. }));
    }

    #[test]
    fn cancel_while_processing_drops_late_result() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        assert_eq!(
            hk(&mut d, Cancel, 1100),
            vec![
                Effect::CancelSession(s),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
        let late = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("late".into()),
            },
            1500,
        );
        assert!(late.is_empty());
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn stale_session_results_are_ignored_by_new_session() {
        let mut d = d();
        let s1 = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, Cancel, 500);
        let s2 = session_of(&hk(&mut d, PttDown, 1000));
        assert_ne!(s1, s2);
        hk(&mut d, PttUp, 3000);
        let fx = d.handle(
            Input::Committed {
                session: s1,
                outcome: CommitOutcome::Text("old".into()),
            },
            3100,
        );
        assert!(fx.is_empty());
        assert_eq!(processing(&d), s2);
    }

    #[test]
    fn cancel_when_idle_is_noop() {
        let mut d = d();
        assert!(hk(&mut d, Cancel, 0).is_empty());
        assert!(hk(&mut d, PttUp, 0).is_empty());
        assert!(!d.is_active());
    }

    #[test]
    fn hotkeys_ignored_while_processing() {
        let mut d = d();
        session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        assert!(hk(&mut d, PttDown, 1100).is_empty());
        assert!(hk(&mut d, HandsFreeToggle, 1200).is_empty());
    }

    #[test]
    fn no_speech_notice_lingers_without_error_sound() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Notice("No speech heard".into()),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![Effect::Overlay(OverlayState::Notice {
                message: "No speech heard".into()
            })]
        );
        assert!(!d.is_active());
        assert!(d.handle(Input::Tick, 5199).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 5200),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn new_dictation_can_start_during_error_linger() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Failed("e".into()),
            },
            1100,
        );
        let fx = hk(&mut d, PttDown, 1200);
        assert_ne!(session_of(&fx), s);
    }

    #[test]
    fn max_duration_warns_once_then_commits() {
        let p = Policy::default();
        let mut d = d();
        let s = session_of(&hk(&mut d, HandsFreeToggle, 0));
        assert!(d.handle(Input::Tick, p.warn_ms - 1).is_empty());
        assert_eq!(
            d.handle(Input::Tick, p.warn_ms),
            vec![Effect::MaxDurationWarning {
                remaining_ms: p.max_ms - p.warn_ms
            }]
        );
        assert!(
            d.handle(Input::Tick, p.warn_ms + 10).is_empty(),
            "warn once"
        );
        let fx = d.handle(Input::Tick, p.max_ms);
        assert!(fx.contains(&Effect::Commit(s)), "hard stop keeps the audio");
        assert_eq!(processing(&d), s);
    }

    #[test]
    fn start_failure_while_listening_tears_down_and_errors() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        let fx = d.handle(
            Input::StartFailed {
                session: s,
                message: "no mic".into(),
            },
            10,
        );
        assert_eq!(
            fx,
            vec![
                Effect::EndCapture,
                Effect::Unduck,
                Effect::CancelSession(s),
                Effect::Sound(Sound::Error),
                Effect::Overlay(OverlayState::Error {
                    message: "no mic".into()
                }),
            ]
        );
        assert_eq!(
            d.phase(),
            &Phase::Error {
                since_ms: 10,
                linger_ms: 4000
            }
        );
        assert!(hk(&mut d, PttUp, 500).is_empty());
    }

    #[test]
    fn stale_start_failure_is_ignored() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, Cancel, 10);
        let s2 = session_of(&hk(&mut d, PttDown, 20));
        let fx = d.handle(
            Input::StartFailed {
                session: s,
                message: "x".into(),
            },
            30,
        );
        assert!(fx.is_empty());
        assert_eq!(listening(&d).session, s2);
    }

    #[test]
    fn overlay_finish_commits_even_right_after_lock() {
        let mut d = d();
        let s = session_of(&hk(&mut d, HandsFreeToggle, 0));
        // A HandsFreeToggle this soon would be a "quick double" cancel;
        // the ✓ button always means finish.
        let fx = d.handle(Input::Finish, 100);
        assert!(fx.contains(&Effect::Commit(s)));
        assert!(matches!(d.phase(), Phase::Processing { .. }));
        assert!(
            d.handle(Input::Finish, 200).is_empty(),
            "ignored while processing"
        );
    }

    #[test]
    fn paste_last_hotkey_does_not_disturb_the_machine() {
        let mut d = d();
        assert!(hk(&mut d, PasteLast, 0).is_empty());
        assert_eq!(d.phase(), &Phase::Idle);
        let s = session_of(&hk(&mut d, PttDown, 10));
        assert!(hk(&mut d, PasteLast, 20).is_empty());
        assert_eq!(listening(&d).session, s);
    }

    // ---- Command Mode ----

    fn command_session(d: &mut Dictation) -> SessionId {
        let fx = hk(d, CommandDown, 0);
        let s = session_of(&fx);
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Start),
                Effect::Duck,
                Effect::BeginSession(s),
                Effect::Overlay(OverlayState::ActiveCommand),
            ]
        );
        assert_eq!(listening(d).mode, Mode::Command);
        s
    }

    #[test]
    fn command_release_commits_and_the_instruction_runs_instead_of_pasting() {
        let mut d = d();
        let s = command_session(&mut d);
        let fx = hk(&mut d, CommandUp, 1200);
        assert!(fx.contains(&Effect::Commit(s)));
        assert!(fx.contains(&Effect::Overlay(OverlayState::CommandProcessing)));
        assert_eq!(
            d.phase(),
            &Phase::Processing {
                session: s,
                since_ms: 1200,
                command: true
            }
        );
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("make it formal".into()),
            },
            1500,
        );
        assert_eq!(
            fx,
            vec![Effect::Command {
                session: s,
                instruction: "make it formal".into()
            }],
            "the instruction is never pasted"
        );
        let fx = d.handle(
            Input::CommandFinished {
                session: s,
                result: Ok("Formal text.".into()),
            },
            3000,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "Formal text.".into()
            }]
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            3100,
        );
        assert!(fx.contains(&Effect::Sound(Sound::Paste)));
        assert!(matches!(d.phase(), Phase::Pasted { .. }));
    }

    #[test]
    fn command_failure_pastes_nothing_and_plays_the_error_sound() {
        let mut d = d();
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 1000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("translate".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::CommandFinished {
                session: s,
                result: Err("no LLM".into()),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Error),
                Effect::Overlay(OverlayState::Error {
                    message: "no LLM".into()
                }),
            ]
        );
        assert!(matches!(d.phase(), Phase::Error { .. }));
    }

    #[test]
    fn a_command_answer_shows_the_card_and_pastes_nothing() {
        let mut d = d();
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 1000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("what does this mean".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::CommandAnswered {
                session: s,
                answer: "It asks to move the meeting.".into(),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![Effect::Overlay(OverlayState::Answer {
                message: "It asks to move the meeting.".into()
            })],
            "no paste, no error sound"
        );
        let linger = Policy::default().answer_min_ms;
        assert_eq!(
            d.phase(),
            &Phase::Answer {
                since_ms: 1200,
                linger_ms: linger
            }
        );
        assert!(!d.is_active(), "Esc is not swallowed");
        assert!(d.handle(Input::Tick, 1200 + linger - 1).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 1200 + linger),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn esc_or_a_new_dictation_replaces_the_answer_card() {
        let mut d = d();
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 1000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("explain this".into()),
            },
            1100,
        );
        d.handle(
            Input::CommandAnswered {
                session: s,
                answer: "An answer.".into(),
            },
            1200,
        );
        assert_eq!(
            hk(&mut d, Cancel, 1300),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
        // A late answer of a finished session is dropped.
        assert!(d
            .handle(
                Input::CommandAnswered {
                    session: s,
                    answer: "late".into()
                },
                1400
            )
            .is_empty());
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 3000);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("explain this".into()),
            },
            3100,
        );
        d.handle(
            Input::CommandAnswered {
                session: s,
                answer: "An answer.".into(),
            },
            3200,
        );
        let fx = hk(&mut d, PttDown, 3300);
        assert!(fx.contains(&Effect::Overlay(OverlayState::ActivePtt)));
    }

    #[test]
    fn long_answers_stay_up_long_enough_to_read() {
        let p = Policy::default();
        assert_eq!(p.answer_linger_ms("Short."), p.answer_min_ms);
        assert_eq!(p.answer_linger_ms(&"a".repeat(300)), 20_000);
        assert_eq!(p.answer_linger_ms(&"a".repeat(5000)), ANSWER_MAX_MS);
    }

    #[test]
    fn command_without_an_instruction_is_a_notice() {
        let mut d = d();
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Empty,
            },
            1100,
        );
        assert_eq!(
            fx,
            vec![Effect::Overlay(OverlayState::Notice {
                message: "No instruction heard".into()
            })]
        );
    }

    #[test]
    fn command_tap_esc_abort_and_timeout_never_paste() {
        let mut d = d();
        let s = command_session(&mut d);
        assert_eq!(hk(&mut d, CommandUp, 200), cancel_fx(s), "tap");
        let s = session_of(&hk(&mut d, CommandDown, 1000));
        assert_eq!(hk(&mut d, Cancel, 1500), cancel_fx(s), "Esc");
        let s = session_of(&hk(&mut d, CommandDown, 2000));
        assert_eq!(hk(&mut d, PttAborted, 2600), cancel_fx(s), "foreign key");
        let s = session_of(&hk(&mut d, CommandDown, 3000));
        hk(&mut d, CommandUp, 4000);
        let fx = d.handle(Input::Tick, 4000 + 35_000);
        assert_eq!(fx[0], Effect::CancelSession(s));
        assert!(fx.contains(&Effect::Overlay(OverlayState::Error {
            message: "Command timed out".into()
        })));
        // A late answer of the timed-out command is dropped.
        let late = d.handle(
            Input::CommandFinished {
                session: s,
                result: Ok("late".into()),
            },
            40_000,
        );
        assert!(late.is_empty());
    }

    #[test]
    fn esc_while_a_command_runs_cancels_it() {
        let mut d = d();
        let s = command_session(&mut d);
        hk(&mut d, CommandUp, 1000);
        assert_eq!(
            hk(&mut d, Cancel, 1100),
            vec![
                Effect::CancelSession(s),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
    }

    #[test]
    fn alt_during_push_to_talk_turns_it_into_a_command() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert_eq!(
            hk(&mut d, CommandDown, 100),
            vec![Effect::Overlay(OverlayState::ActiveCommand)]
        );
        assert_eq!(listening(&d).session, s, "same session, same audio");
        assert_eq!(listening(&d).mode, Mode::Command);
        let fx = hk(&mut d, CommandUp, 900);
        assert!(fx.contains(&Effect::Commit(s)));
        assert!(matches!(d.phase(), Phase::Processing { command: true, .. }));
    }

    #[test]
    fn dictation_commits_still_paste_directly() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("hello".into()),
            },
            1100,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "hello".into()
            }]
        );
        assert!(
            d.handle(
                Input::CommandFinished {
                    session: s,
                    result: Ok("x".into())
                },
                1200
            )
            .is_empty(),
            "a dictation ignores command results"
        );
    }

    fn transform_session(fx: &[Effect]) -> SessionId {
        fx.iter()
            .find_map(|e| match e {
                Effect::Transform { session, .. } => Some(*session),
                _ => None,
            })
            .expect("Effect::Transform")
    }

    #[test]
    fn transform_shortcut_goes_straight_to_processing_and_pastes_the_answer() {
        let mut d = d();
        let fx = hk(&mut d, Transform(1), 0);
        let s = transform_session(&fx);
        assert_eq!(
            fx,
            vec![
                Effect::Overlay(OverlayState::CommandProcessing),
                Effect::Transform {
                    session: s,
                    index: 1
                },
            ],
            "no sound, no mic, no ducking"
        );
        assert!(d.is_active(), "Esc is swallowed while it runs");
        assert!(hk(&mut d, Transform(0), 10).is_empty(), "one at a time");
        let fx = d.handle(
            Input::CommandFinished {
                session: s,
                result: Ok("better".into()),
            },
            500,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "better".into()
            }]
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            600,
        );
        assert_eq!(fx[0], Effect::Sound(Sound::Paste));
        assert!(!d.is_active());
    }

    #[test]
    fn transform_with_nothing_selected_is_a_notice_and_esc_cancels() {
        let mut d = d();
        let s = transform_session(&hk(&mut d, Transform(0), 0));
        let fx = d.handle(
            Input::CommandFinished {
                session: s,
                result: Err(TRANSFORM_NO_SELECTION.into()),
            },
            100,
        );
        assert_eq!(
            fx,
            vec![Effect::Overlay(OverlayState::Notice {
                message: TRANSFORM_NO_SELECTION.into()
            })]
        );
        let s = transform_session(&hk(&mut d, Transform(0), 5000));
        assert_eq!(
            hk(&mut d, Cancel, 5100),
            vec![
                Effect::CancelSession(s),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
        let s = transform_session(&hk(&mut d, Transform(0), 9000));
        let fx = d.handle(
            Input::CommandFinished {
                session: s,
                result: Err("AI failed".into()),
            },
            9100,
        );
        assert!(fx.contains(&Effect::Sound(Sound::Error)));
    }

    fn dictated(d: &mut Dictation) -> SessionId {
        let s = session_of(&hk(d, PttDown, 0));
        hk(d, PttUp, 1000);
        s
    }

    #[test]
    fn auto_apply_transforms_a_dictation_before_the_paste() {
        let mut d = d();
        d.set_auto_transform(true);
        let s = dictated(&mut d);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("hello there".into()),
            },
            1100,
        );
        assert_eq!(
            fx,
            vec![Effect::AutoTransform {
                session: s,
                text: "hello there".into()
            }]
        );
        let fx = d.handle(
            Input::AutoTransformFinished {
                session: s,
                original: "hello there".into(),
                result: Ok("Hello there.".into()),
            },
            1500,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "Hello there.".into()
            }]
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            1600,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
    }

    #[test]
    fn a_failed_auto_apply_pastes_the_original_then_says_so() {
        let mut d = d();
        d.set_auto_transform(true);
        let s = dictated(&mut d);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("keep me".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::AutoTransformFinished {
                session: s,
                original: "keep me".into(),
                result: Err("AI didn't answer within 8 s".into()),
            },
            9100,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "keep me".into()
            }],
            "never lose a dictation"
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            9200,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Notice {
                    message: TRANSFORM_SKIPPED.into()
                })
            ]
        );
    }

    #[test]
    fn auto_apply_off_pastes_directly_and_commands_are_never_auto_transformed() {
        let mut d = d();
        let s = dictated(&mut d);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("x".into()),
            },
            1100,
        );
        assert!(matches!(fx[0], Effect::Paste { .. }));
        let mut d = Dictation::new(Policy::default());
        d.set_auto_transform(true);
        let s = session_of(&hk(&mut d, CommandDown, 0));
        hk(&mut d, CommandUp, 1000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("make it formal".into()),
            },
            1100,
        );
        assert!(matches!(fx[0], Effect::Command { .. }));
    }

    #[test]
    fn policy_default_timings() {
        let p = Policy::default();
        assert_eq!(p.max_ms, 1_200_000);
        assert_eq!(p.warn_ms, 1_140_000);
        assert_eq!(p.debounce_ms, 500);
        assert_eq!(p.double_tap_gap_ms, 500);
        assert_eq!(p.processing_timeout_ms, 35_000);
        assert_eq!(p.notice_min_ms, 4000);
        assert_eq!(p.notice_max_ms, 8000);
        assert_eq!(p.max_samples(), 20 * 60 * 16_000);
    }

    // --- Right Alt push-to-talk end to end (D57): the real chord matcher
    // with the default shortcuts feeding the real machine. ---

    const RALT_VK: u16 = 0xA5;
    const ALT_SCAN: u32 = 0x38;

    /// Right Alt down/up at the given times (ms) on an AltGr layout (the
    /// fake Left Ctrl rides along; at the release it goes up first if
    /// `fake_up_first`), with a tick every 50 ms up to `until`.
    fn right_alt_presses(
        d: &mut Dictation,
        presses: &[(u64, u64)],
        until: u64,
        fake_up_first: bool,
    ) -> Vec<Effect> {
        right_alt_timeline(d, presses, &[], until, fake_up_first)
    }

    /// [`right_alt_presses`] plus an auto-repeat (both AltGr halves
    /// again, as key-downs) at each time in `repeats`.
    fn right_alt_timeline(
        d: &mut Dictation,
        presses: &[(u64, u64)],
        repeats: &[u64],
        until: u64,
        fake_up_first: bool,
    ) -> Vec<Effect> {
        let mut m = crate::hotkeys::ChordMatcher::new(crate::hotkeys::Bindings::default());
        let fake = |down| (0xA2, 0x21D, down);
        let ralt = |down| (RALT_VK, ALT_SCAN, down);
        let mut fx = Vec::new();
        for t in 0..=until {
            if repeats.contains(&t) {
                for (vk, scan, is_down) in [fake(true), ralt(true)] {
                    let dec = m.handle_raw(vk, scan, is_down, d.is_active());
                    assert!(dec.suppress, "a repeat never reaches the app");
                    for a in dec.actions {
                        fx.extend(d.handle(Input::Hotkey(a), t));
                    }
                }
            }
            for &(down, up) in presses {
                let evs = if t == down {
                    vec![fake(true), ralt(true)]
                } else if t == up && fake_up_first {
                    vec![fake(false), ralt(false)]
                } else if t == up {
                    vec![ralt(false), fake(false)]
                } else {
                    Vec::new()
                };
                for (vk, scan, is_down) in evs {
                    let dec = m.handle_raw(vk, scan, is_down, d.is_active());
                    assert!(dec.suppress, "Right Alt and AltGr never reach the app");
                    for a in dec.actions {
                        fx.extend(d.handle(Input::Hotkey(a), t));
                    }
                }
            }
            if t % 50 == 0 {
                fx.extend(d.handle(Input::Tick, t));
            }
        }
        fx
    }

    fn default_policy() -> Dictation {
        Dictation::new(crate::prefs::Prefs::default().policy())
    }

    #[test]
    fn right_alt_double_tap_locks_hands_free() {
        for fake_up_first in [false, true] {
            let mut d = default_policy();
            let fx = right_alt_presses(&mut d, &[(0, 80), (200, 280)], 3000, fake_up_first);
            assert!(fx.contains(&Effect::Sound(Sound::Lock)));
            assert_eq!(listening(&d).mode, Mode::Locked);
            assert!(!fx.iter().any(|e| matches!(e, Effect::Commit(_))));
        }
    }

    #[test]
    fn right_alt_hold_commits() {
        for fake_up_first in [false, true] {
            let mut d = default_policy();
            let fx = right_alt_presses(&mut d, &[(0, 1000)], 1100, fake_up_first);
            let s = session_of(&fx);
            assert!(fx.contains(&Effect::Commit(s)));
            assert_eq!(processing(&d), s);
        }
    }

    #[test]
    fn right_alt_tap_is_dismissed_with_nothing_committed() {
        for fake_up_first in [false, true] {
            let mut d = default_policy();
            let fx = right_alt_presses(&mut d, &[(0, 80)], 1000, fake_up_first);
            let s = session_of(&fx);
            assert!(fx.contains(&Effect::CancelSession(s)));
            assert!(!fx.iter().any(|e| matches!(e, Effect::Commit(_))));
            assert_eq!(d.phase(), &Phase::Idle);
        }
    }

    // ---- D90: translation ----

    /// A translate session held past the debounce and released.
    fn translated(d: &mut Dictation) -> SessionId {
        let s = session_of(&hk(d, TranslateDown, 0));
        assert_eq!(listening(d).mode, Mode::Translate);
        let fx = hk(d, TranslateUp, 1000);
        assert!(fx.contains(&Effect::Commit(s)));
        assert_eq!(
            fx.last(),
            Some(&Effect::Overlay(OverlayState::CommandProcessing))
        );
        s
    }

    #[test]
    fn a_translation_pastes_the_translated_text() {
        let mut d = d();
        d.set_auto_transform(true); // a translation is never auto-transformed
        let s = translated(&mut d);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("the demo is on thursday".into()),
            },
            1100,
        );
        assert_eq!(
            fx,
            vec![Effect::Translate {
                session: s,
                text: "the demo is on thursday".into()
            }]
        );
        let fx = d.handle(
            Input::TranslateFinished {
                session: s,
                original: "the demo is on thursday".into(),
                result: Ok("translated".into()),
            },
            3000,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "translated".into()
            }]
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            3100,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Resting)
            ]
        );
    }

    #[test]
    fn a_failed_translation_pastes_the_dictation_then_says_so() {
        let mut d = d();
        let s = translated(&mut d);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("keep me".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::TranslateFinished {
                session: s,
                original: "keep me".into(),
                result: Err(murmur_core::translate::NO_LLM_NOTICE.into()),
            },
            2000,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "keep me".into()
            }],
            "never lose a dictation"
        );
        let fx = d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            2100,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Notice {
                    message: murmur_core::translate::NO_LLM_NOTICE.into()
                })
            ]
        );
    }

    #[test]
    fn translate_extends_a_push_to_talk_and_a_tap_or_abort_cancels() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert_eq!(
            hk(&mut d, TranslateDown, 100),
            vec![Effect::Overlay(OverlayState::ActiveCommand)]
        );
        assert_eq!(listening(&d).session, s, "same recording");
        assert!(hk(&mut d, TranslateUp, 900).contains(&Effect::Commit(s)));

        let mut d = self::d();
        let s = session_of(&hk(&mut d, TranslateDown, 0));
        assert_eq!(hk(&mut d, TranslateUp, 200), cancel_fx(s), "tap");
        let s = session_of(&hk(&mut d, TranslateDown, 1000));
        assert_eq!(hk(&mut d, PttAborted, 1100), cancel_fx(s));
    }

    #[test]
    fn a_plain_dictation_after_a_translation_is_not_translated() {
        let mut d = d();
        let s = translated(&mut d);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Empty,
            },
            1100,
        );
        assert_eq!(d.phase(), &Phase::Idle);
        let s = session_of(&hk(&mut d, PttDown, 5000));
        hk(&mut d, PttUp, 6000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("plain".into()),
            },
            6100,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "plain".into()
            }]
        );
    }

    /// A webhook session held past the debounce and released.
    fn sent_to_webhook(d: &mut Dictation) -> SessionId {
        let fx = hk(d, WebhookDown, 0);
        let s = session_of(&fx);
        assert!(fx.contains(&Effect::Overlay(OverlayState::ActivePtt)));
        assert_eq!(listening(d).mode, Mode::Webhook);
        let fx = hk(d, WebhookUp, 1000);
        assert!(fx.contains(&Effect::Commit(s)));
        s
    }

    #[test]
    fn a_webhook_dictation_is_sent_not_pasted() {
        let mut d = d();
        d.set_auto_transform(true); // never applied to a webhook dictation
        let s = sent_to_webhook(&mut d);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("Clean words.".into()),
            },
            1100,
        );
        assert_eq!(
            fx,
            vec![Effect::Webhook {
                session: s,
                text: "Clean words.".into()
            }]
        );
        let fx = d.handle(
            Input::WebhookFinished {
                session: s,
                outcome: crate::webhook::Outcome::Sent,
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Notice {
                    message: crate::webhook::SENT.into()
                })
            ]
        );
        assert!(!fx.iter().any(|e| matches!(e, Effect::Paste { .. })));
    }

    #[test]
    fn a_failed_webhook_shows_its_status_long_enough_to_read() {
        let mut d = d();
        let s = sent_to_webhook(&mut d);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("words".into()),
            },
            1100,
        );
        let fx = d.handle(
            Input::WebhookFinished {
                session: s,
                outcome: crate::webhook::Outcome::Status(500),
            },
            1200,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Error),
                Effect::Overlay(OverlayState::Error {
                    message: "Webhook failed (HTTP 500)".into()
                })
            ]
        );
        let linger = d.policy().notice_linger_ms("Webhook failed (HTTP 500)");
        assert!(d.handle(Input::Tick, 1200 + linger - 1).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 1200 + linger),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
    }

    #[test]
    fn webhook_extends_a_push_to_talk_and_a_tap_or_abort_cancels() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert!(hk(&mut d, WebhookDown, 100).is_empty());
        assert_eq!(listening(&d).mode, Mode::Webhook);
        assert!(hk(&mut d, WebhookUp, 900).contains(&Effect::Commit(s)));
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("t".into()),
            },
            1000,
        );
        assert!(matches!(fx[..], [Effect::Webhook { .. }]));
        d.handle(
            Input::WebhookFinished {
                session: s,
                outcome: crate::webhook::Outcome::Sent,
            },
            1100,
        );
        d.handle(Input::Tick, 5000);
        let s = session_of(&hk(&mut d, WebhookDown, 6000));
        assert_eq!(hk(&mut d, WebhookUp, 6200), cancel_fx(s), "tap");
        let s = session_of(&hk(&mut d, WebhookDown, 7000));
        assert_eq!(hk(&mut d, PttAborted, 7100), cancel_fx(s), "abort");
    }

    /// A webhook dictation whose POST is in flight (`Effect::Webhook` out).
    fn webhook_in_flight(d: &mut Dictation) -> SessionId {
        let s = sent_to_webhook(d);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("words".into()),
            },
            1100,
        );
        assert!(matches!(fx[..], [Effect::Webhook { .. }]));
        s
    }

    #[test]
    fn esc_during_the_post_still_shows_what_really_happened() {
        use crate::webhook::Outcome;
        // Sent anyway: the pill says so, after Esc rested it.
        let mut d = d();
        let s = webhook_in_flight(&mut d);
        assert!(hk(&mut d, Cancel, 1200).contains(&Effect::CancelSession(s)));
        assert_eq!(d.phase(), &Phase::Idle);
        let fx = d.handle(
            Input::WebhookFinished {
                session: s,
                outcome: Outcome::Sent,
            },
            1300,
        );
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Paste),
                Effect::Overlay(OverlayState::Notice {
                    message: crate::webhook::ALREADY_SENT.into()
                })
            ]
        );
        // Failed anyway: the failure shows.
        let mut d = d_at_rest_after(Outcome::Status(502));
        assert!(matches!(d.phase(), Phase::Error { .. }));
        // Dropped before it went out: nothing more to say.
        d = d_at_rest_after(Outcome::Cancelled);
        assert_eq!(d.phase(), &Phase::Idle);
        // Only once: a second (stale) result is ignored.
        let s2 = webhook_in_flight(&mut d);
        hk(&mut d, Cancel, 1200);
        let done = Input::WebhookFinished {
            session: s2,
            outcome: Outcome::Sent,
        };
        assert!(!d.handle(done.clone(), 1300).is_empty());
        d.handle(Input::Tick, 9000);
        assert!(d.handle(done, 9100).is_empty());
    }

    /// Esc during the POST, then its `outcome` arrives.
    fn d_at_rest_after(outcome: crate::webhook::Outcome) -> Dictation {
        let mut d = d();
        let s = webhook_in_flight(&mut d);
        hk(&mut d, Cancel, 1200);
        d.handle(
            Input::WebhookFinished {
                session: s,
                outcome,
            },
            1300,
        );
        d
    }

    #[test]
    fn webhook_timed_out_only_once_the_post_went_out() {
        let overlay = |fx: &[Effect]| {
            fx.iter()
                .find_map(|e| match e {
                    Effect::Overlay(OverlayState::Error { message }) => Some(message.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let limit = Policy::default().processing_timeout_ms;
        let mut early = d();
        sent_to_webhook(&mut early); // still transcribing
        assert_eq!(
            overlay(&early.handle(Input::Tick, 1000 + limit)),
            "Transcription timed out"
        );
        let mut late = d();
        webhook_in_flight(&mut late);
        assert_eq!(
            overlay(&late.handle(Input::Tick, 1000 + limit)),
            "Webhook timed out"
        );
    }

    #[test]
    fn a_plain_dictation_after_a_webhook_one_is_pasted() {
        let mut d = d();
        let s = sent_to_webhook(&mut d);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Empty,
            },
            1100,
        );
        assert_eq!(d.phase(), &Phase::Idle);
        let s = session_of(&hk(&mut d, PttDown, 5000));
        hk(&mut d, PttUp, 6000);
        let fx = d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("plain".into()),
            },
            6100,
        );
        assert_eq!(
            fx,
            vec![Effect::Paste {
                session: s,
                text: "plain".into()
            }]
        );
    }

    #[test]
    fn a_stuck_translation_times_out_with_its_own_message() {
        let mut d = d();
        let s = translated(&mut d);
        let fx = d.handle(Input::Tick, 1000 + Policy::default().processing_timeout_ms);
        assert_eq!(fx[0], Effect::CancelSession(s));
        assert!(fx.contains(&Effect::Overlay(OverlayState::Error {
            message: "Translation timed out".into()
        })));
    }

    // --- D92: the double-tap detector, as timestamps. ---

    /// Push-to-talk presses `(down, up)` (ms) with a tick every 50 ms up
    /// to `until`, as the controller feeds them.
    fn taps(d: &mut Dictation, presses: &[(u64, u64)], until: u64) -> Vec<Effect> {
        let mut fx = Vec::new();
        for t in 0..=until {
            for &(down, up) in presses {
                if t == down {
                    fx.extend(hk(d, PttDown, t));
                }
                if t == up {
                    fx.extend(hk(d, PttUp, t));
                }
            }
            if t % 50 == 0 {
                fx.extend(d.handle(Input::Tick, t));
            }
        }
        fx
    }

    fn is_locked(d: &Dictation) -> bool {
        matches!(d.phase(), Phase::Listening(l) if l.mode == Mode::Locked)
    }

    fn assert_locked_once(d: &Dictation, fx: &[Effect], why: &str) {
        assert!(is_locked(d), "{why}: {:?}", d.phase());
        let begins = fx
            .iter()
            .filter(|e| matches!(e, Effect::BeginSession(_)))
            .count();
        assert_eq!(begins, 1, "{why}: one session, locked in place");
        assert!(!fx.iter().any(|e| matches!(e, Effect::Commit(_))), "{why}");
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::CancelSession(_))),
            "{why}"
        );
    }

    #[test]
    fn double_tap_locks_across_human_tap_and_gap_lengths() {
        // Taps of 80–200 ms, gaps of 120–450 ms: press to press runs up to
        // 650 ms, past the old press-measured 500 ms window.
        for tap in [80, 120, 160, 200] {
            for gap in [120, 200, 300, 400, 450] {
                let mut d = d();
                let (a, b) = (1000, 1000 + tap);
                let (c, e) = (b + gap, b + gap + tap);
                let fx = taps(&mut d, &[(a, b), (c, e)], e + 2000);
                assert_locked_once(&d, &fx, &format!("tap {tap} gap {gap}"));
            }
        }
    }

    #[test]
    fn harness_cells_that_failed_on_1_3_0_lock() {
        // Tap 80/120/160 + gap 400, +20 ms jitter on each (press to press
        // 520–600 ms).
        for tap in [80, 120, 160] {
            let mut d = d();
            let up = tap + 20;
            let fx = taps(&mut d, &[(0, up), (up + 420, up + 420 + tap)], 3000);
            assert_locked_once(&d, &fx, &format!("tap {tap}"));
        }
    }

    #[test]
    fn long_first_tap_still_locks() {
        // Held 300–490 ms (still a tap), then a 400 ms gap.
        for tap in [300, 400, 490] {
            let mut d = d();
            let fx = taps(&mut d, &[(0, tap), (tap + 400, tap + 480)], 3000);
            assert_locked_once(&d, &fx, &format!("first tap {tap}"));
        }
    }

    #[test]
    fn hold_past_threshold_is_still_push_to_talk() {
        let mut d = d();
        let fx = taps(&mut d, &[(0, 500)], 1000);
        let s = session_of(&fx);
        assert!(fx.contains(&Effect::Commit(s)));
        assert_eq!(processing(&d), s);
    }

    #[test]
    fn single_tap_is_dismissed_one_gap_after_its_release() {
        let mut d = d();
        let fx = taps(&mut d, &[(0, 200)], 650);
        let s = session_of(&fx);
        assert_eq!(listening(&d).released_at, Some(200), "waits for a 2nd tap");
        assert!(!fx.contains(&Effect::CancelSession(s)));
        assert_eq!(d.handle(Input::Tick, 700), cancel_fx(s));
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn second_press_at_the_gap_is_a_fresh_press_not_a_lock() {
        let mut d = d();
        hk(&mut d, PttDown, 0);
        hk(&mut d, PttUp, 100);
        hk(&mut d, PttDown, 599);
        assert!(is_locked(&d), "499 ms after the release: locks");
        let mut d = Dictation::new(Policy::default());
        hk(&mut d, PttDown, 0);
        hk(&mut d, PttUp, 100);
        hk(&mut d, PttDown, 600);
        assert_eq!(listening(&d).mode, Mode::Ptt, "500 ms: a new press");
        // ...that is push-to-talk when held.
        let s = listening(&d).session;
        assert!(hk(&mut d, PttUp, 1600).contains(&Effect::Commit(s)));
    }

    #[test]
    fn triple_press_still_dismisses_with_slow_taps() {
        let mut d = d();
        let fx = taps(&mut d, &[(0, 200), (600, 750), (900, 950)], 1000);
        let s = session_of(&fx);
        assert!(fx.contains(&Effect::CancelSession(s)));
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn repeated_key_events_change_nothing() {
        // A duplicate release after the tap and duplicate presses once
        // locked (the matcher drops auto-repeat; the machine must not care).
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        assert!(hk(&mut d, PttUp, 150).is_empty());
        assert!(hk(&mut d, PttUp, 160).is_empty(), "2nd release ignored");
        assert_eq!(listening(&d).released_at, Some(150));
        assert_eq!(hk(&mut d, PttDown, 550), lock_fx());
        assert_eq!(listening(&d).session, s);
        // A press held with repeats: no early release, a hold.
        let mut d = Dictation::new(Policy::default());
        let s = session_of(&hk(&mut d, PttDown, 0));
        for t in (300..1000).step_by(33) {
            assert!(hk(&mut d, PttDown, t).is_empty(), "repeat at {t}");
        }
        assert!(hk(&mut d, PttUp, 1000).contains(&Effect::Commit(s)));
    }

    /// Dictate and stop: processing session `s`.
    fn processing_one(d: &mut Dictation) -> SessionId {
        let s = session_of(&hk(d, PttDown, 0));
        hk(d, PttUp, 1000);
        assert_eq!(processing(d), s);
        s
    }

    fn paste_done(d: &mut Dictation, s: SessionId, t: u64) -> Vec<Effect> {
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Text("hi".into()),
            },
            t,
        );
        d.handle(
            Input::PasteFinished {
                session: s,
                result: Ok(()),
            },
            t,
        )
    }

    #[test]
    fn first_tap_during_processing_second_after_paste_locks() {
        let mut d = d();
        let s = processing_one(&mut d);
        assert!(hk(&mut d, PttDown, 1200).is_empty());
        assert!(hk(&mut d, PttUp, 1300).is_empty());
        paste_done(&mut d, s, 1400);
        d.handle(Input::Tick, 1450);
        let fx = hk(&mut d, PttDown, 1700);
        assert_eq!(fx[0], Effect::Sound(Sound::Lock));
        assert!(is_locked(&d));
        assert_ne!(listening(&d).session, s);
    }

    #[test]
    fn double_tap_during_processing_locks_when_the_paste_lands() {
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1600);
        hk(&mut d, PttUp, 1700);
        assert_eq!(processing(&d), s, "no second recording meanwhile");
        // Within the gap (499 ms) of the second press.
        let fx = paste_done(&mut d, s, 2099);
        assert_eq!(fx[0], Effect::Sound(Sound::Paste));
        assert!(fx.contains(&Effect::Sound(Sound::Lock)));
        assert!(is_locked(&d));
        assert_eq!(listening(&d).since_ms, 2099);
        assert!(hk(&mut d, PttUp, 2150).is_empty(), "stays locked");
    }

    #[test]
    fn queued_lock_is_dropped_when_stale_failed_or_cancelled() {
        // Processing ended a gap (500 ms) after the second press.
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1600);
        let fx = paste_done(&mut d, s, 2100);
        assert!(!fx.contains(&Effect::Sound(Sound::Lock)));
        assert_eq!(d.phase(), &Phase::Pasted { since_ms: 2100 });
        // Processing failed: the error stays readable.
        let mut d = Dictation::new(Policy::default());
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1600);
        d.handle(
            Input::Committed {
                session: s,
                outcome: CommitOutcome::Failed("boom".into()),
            },
            1700,
        );
        assert!(matches!(d.phase(), Phase::Error { .. }));
        d.handle(Input::Tick, 5700);
        assert_eq!(d.phase(), &Phase::Idle);
        // Esc while processing.
        let mut d = Dictation::new(Policy::default());
        processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1600);
        hk(&mut d, Cancel, 1650);
        assert_eq!(d.phase(), &Phase::Idle);
        let fx = hk(&mut d, PttDown, 1700);
        assert_eq!(fx[0], Effect::Sound(Sound::Start), "plain push-to-talk");
    }

    #[test]
    fn single_or_slow_taps_during_processing_lock_nothing() {
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1800); // 500 ms after the release
        hk(&mut d, PttUp, 1850);
        paste_done(&mut d, s, 1900);
        d.handle(Input::Tick, 1950);
        assert_eq!(d.phase(), &Phase::Idle);
        // A held press during processing is not a tap.
        let mut d = Dictation::new(Policy::default());
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1800);
        paste_done(&mut d, s, 1850);
        let fx = hk(&mut d, PttDown, 1900);
        assert_eq!(fx[0], Effect::Sound(Sound::Start));
        assert_eq!(listening(&d).mode, Mode::Ptt);
    }

    #[test]
    fn right_alt_altgr_slow_double_tap_locks() {
        // AltGr's fake Left Ctrl rides along with each Right Alt edge.
        for fake_up_first in [false, true] {
            for (tap, gap) in [(80, 120), (160, 400), (200, 450), (400, 300)] {
                let mut d = default_policy();
                let presses = [(100, 100 + tap), (100 + tap + gap, 100 + 2 * tap + gap)];
                let fx = right_alt_presses(&mut d, &presses, 3000, fake_up_first);
                assert_locked_once(&d, &fx, &format!("AltGr tap {tap} gap {gap}"));
            }
        }
    }

    #[test]
    fn right_alt_auto_repeat_while_held_is_still_a_hold() {
        // Auto-repeat re-sends both AltGr halves while Right Alt is held.
        for fake_up_first in [false, true] {
            let mut d = default_policy();
            let repeats: Vec<u64> = (250..1200).step_by(33).collect();
            let fx = right_alt_timeline(&mut d, &[(0, 1200)], &repeats, 1300, fake_up_first);
            let s = session_of(&fx);
            assert!(fx.contains(&Effect::Commit(s)));
            assert_eq!(processing(&d), s);
        }
    }

    #[test]
    fn right_alt_auto_repeat_during_double_tap_still_locks() {
        // A first tap long enough to auto-repeat (300 ms), gap 400.
        for fake_up_first in [false, true] {
            let mut d = default_policy();
            let presses = [(0, 300), (700, 780)];
            let fx = right_alt_timeline(&mut d, &presses, &[250, 283], 2000, fake_up_first);
            assert_locked_once(&d, &fx, "repeat in the first tap");
        }
    }

    // --- D92 review: Esc and other keys clear a pending tap. ---

    /// A first tap during processing, then the paste lands: the next
    /// press within the gap would lock.
    fn tap_pending_after_paste() -> Dictation {
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        paste_done(&mut d, s, 1350);
        assert!(matches!(d.phase(), Phase::Pasted { .. }));
        d
    }

    #[test]
    fn esc_after_a_pending_tap_means_the_next_press_does_not_lock() {
        // Esc in Pasted.
        let mut d = tap_pending_after_paste();
        hk(&mut d, Cancel, 1360);
        let fx = hk(&mut d, PttDown, 1500);
        assert_eq!(fx[0], Effect::Sound(Sound::Start), "push-to-talk, no lock");
        assert_eq!(listening(&d).mode, Mode::Ptt);
        // Esc in Idle.
        let mut d = tap_pending_after_paste();
        d.handle(Input::Tick, 1400);
        assert_eq!(d.phase(), &Phase::Idle);
        hk(&mut d, Cancel, 1450);
        hk(&mut d, PttDown, 1500);
        assert_eq!(listening(&d).mode, Mode::Ptt);
    }

    #[test]
    fn ptt_aborted_at_rest_clears_a_pending_tap() {
        let mut d = tap_pending_after_paste();
        hk(&mut d, PttAborted, 1400);
        hk(&mut d, PttDown, 1500);
        assert_eq!(listening(&d).mode, Mode::Ptt);
    }

    #[test]
    fn esc_or_another_key_during_processing_clears_a_first_tap() {
        for abort in [Cancel, PttAborted] {
            let mut d = d();
            let s = processing_one(&mut d);
            hk(&mut d, PttDown, 1200);
            hk(&mut d, PttUp, 1300);
            hk(&mut d, abort, 1350);
            if abort == PttAborted {
                paste_done(&mut d, s, 1400);
            }
            hk(&mut d, PttDown, 1500);
            assert_eq!(listening(&d).mode, Mode::Ptt, "{abort:?}");
        }
    }

    #[test]
    fn third_press_during_processing_clears_the_queued_lock() {
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, PttDown, 1500); // second: lock queued
        hk(&mut d, PttUp, 1550);
        hk(&mut d, PttDown, 1650); // third: dismissed
        hk(&mut d, PttUp, 1700);
        let fx = paste_done(&mut d, s, 1750);
        assert!(!fx.contains(&Effect::Sound(Sound::Lock)), "{fx:?}");
        assert_eq!(d.phase(), &Phase::Pasted { since_ms: 1750 });
        // ...and starts no new tap either.
        hk(&mut d, PttDown, 1800);
        assert_eq!(listening(&d).mode, Mode::Ptt);
    }

    #[test]
    fn translate_or_command_during_processing_clears_a_pending_double_tap() {
        // A queued lock, then the Translate (or Command) chord: no lock.
        for other in [TranslateDown, CommandDown] {
            let mut d = d();
            let s = processing_one(&mut d);
            hk(&mut d, PttDown, 1200);
            hk(&mut d, PttUp, 1300);
            hk(&mut d, PttDown, 1500);
            hk(&mut d, other, 1510);
            let fx = paste_done(&mut d, s, 1600);
            assert!(!fx.contains(&Effect::Sound(Sound::Lock)), "{other:?}");
            assert_eq!(d.phase(), &Phase::Pasted { since_ms: 1600 });
        }
        // A first tap, then Translate: the next press is push-to-talk.
        let mut d = d();
        let s = processing_one(&mut d);
        hk(&mut d, PttDown, 1200);
        hk(&mut d, PttUp, 1300);
        hk(&mut d, TranslateDown, 1350);
        paste_done(&mut d, s, 1400);
        hk(&mut d, PttDown, 1500);
        assert_eq!(listening(&d).mode, Mode::Ptt);
    }

    #[test]
    fn translate_after_a_tap_at_rest_starts_translate_not_a_lock() {
        // A push-to-talk tap is pending; the Translate chord turns that
        // listening session into a translation, which a release ends.
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        hk(&mut d, TranslateDown, 300);
        let l = listening(&d);
        assert_eq!(
            (l.mode, l.session, l.released_at),
            (Mode::Translate, s, None)
        );
        assert!(d.handle(Input::Tick, 900).is_empty(), "no tap dismissal");
        assert!(hk(&mut d, TranslateUp, 1300).contains(&Effect::Commit(s)));
    }
}
