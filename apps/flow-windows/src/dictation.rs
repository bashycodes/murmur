//! Dictation state machine (spec 10 §3):
//! `idle → listening(ptt|locked) → processing → pasted|error → idle`.
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listening {
    pub mode: Mode,
    pub session: SessionId,
    pub since_ms: u64,
    pub warned: bool,
    /// PTT released before the debounce window ended: dismissed at
    /// `since + debounce` unless pressed again in time (→ double-tap lock).
    pub early_release: bool,
    /// When the session became locked (triple-press detection).
    pub locked_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Listening(Listening),
    Processing { session: SessionId, since_ms: u64 },
    Pasted { since_ms: u64 },
    Error { since_ms: u64 },
}

/// Visual state of the overlay pill (05 §7.3: the five states that matter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OverlayState {
    Resting,
    ActivePtt,
    ActiveLocked,
    Processing,
    Error { message: String },
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
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Hotkey(HotkeyAction),
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
    /// Mic couldn't be opened / core refused the session.
    StartFailed {
        session: SessionId,
        message: String,
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
    /// Max-duration about to be reached (overlay hint).
    MaxDurationWarning {
        remaining_ms: u64,
    },
    Overlay(OverlayState),
}

/// Timings from Wispr 1.6.1167 main process (windows-research
/// `findings/data/overlay-spec.json` `main_side_dictation_timings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Hard stop: the dictation is committed (not discarded) at this length.
    pub max_ms: u64,
    pub warn_ms: u64,
    /// `DICTATION_DEBOUNCE_DELAY`: PTT released sooner = tap (dismissed
    /// unless a second press locks it); second press sooner = double-tap
    /// lock; press sooner after locking = triple-press dismiss.
    pub debounce_ms: u64,
    /// Processing longer than this is an error (Wispr: 35 s).
    pub processing_timeout_ms: u64,
    pub pasted_linger_ms: u64,
    /// Error pill shows this long, then rests (Wispr: 500 ms).
    pub error_linger_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            max_ms: 20 * 60 * 1000,
            warn_ms: 19 * 60 * 1000,
            debounce_ms: 500,
            processing_timeout_ms: 35_000,
            pasted_linger_ms: 0,
            error_linger_ms: 500,
        }
    }
}

impl Policy {
    /// `SessionConfig.max_samples` for the core (16 kHz mono).
    pub fn max_samples(&self) -> u64 {
        self.max_ms * 16
    }
}

#[derive(Debug, Clone)]
pub struct Dictation {
    phase: Phase,
    policy: Policy,
    next_session: SessionId,
}

impl Dictation {
    pub fn new(policy: Policy) -> Self {
        Dictation {
            phase: Phase::Idle,
            policy,
            next_session: 1,
        }
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
        match self.phase.clone() {
            Phase::Idle | Phase::Pasted { .. } | Phase::Error { .. } => {
                self.handle_resting(input, now_ms)
            }
            Phase::Listening(l) => self.handle_listening(l, input, now_ms),
            Phase::Processing { session, since_ms } => {
                self.handle_processing(session, since_ms, input, now_ms)
            }
        }
    }

    fn handle_resting(&mut self, input: Input, now_ms: u64) -> Vec<Effect> {
        let p = self.policy;
        match (&self.phase, input) {
            (_, Input::Hotkey(HotkeyAction::PttDown)) => self.begin(Mode::Ptt, now_ms),
            (_, Input::Hotkey(HotkeyAction::HandsFreeToggle)) => self.begin(Mode::Locked, now_ms),
            (Phase::Pasted { since_ms }, Input::Tick)
                if now_ms.saturating_sub(*since_ms) >= p.pasted_linger_ms =>
            {
                self.phase = Phase::Idle;
                Vec::new()
            }
            (Phase::Error { since_ms }, Input::Tick)
                if now_ms.saturating_sub(*since_ms) >= p.error_linger_ms =>
            {
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
            (_, Input::Finish) => self.stop(l.session, now_ms),
            (_, Input::StartFailed { session, message }) if session == l.session => {
                let mut fx = self.cancel_listening(l.session);
                fx.pop(); // replace the "resting" overlay with the error
                fx.extend(self.fail(message, now_ms));
                fx
            }

            // ---- push-to-talk ----
            (Mode::Ptt, Input::Hotkey(PttDown)) if l.early_release => {
                if elapsed < p.debounce_ms {
                    self.lock(l, now_ms)
                } else {
                    self.cancel_listening(l.session)
                }
            }
            (Mode::Ptt, Input::Hotkey(PttUp)) if !l.early_release => {
                if elapsed < p.debounce_ms {
                    l.early_release = true;
                    self.phase = Phase::Listening(l);
                    Vec::new()
                } else {
                    self.stop(l.session, now_ms)
                }
            }
            (Mode::Ptt, Input::Hotkey(PttAborted)) => self.cancel_listening(l.session),
            (Mode::Ptt, Input::Hotkey(HandsFreeToggle)) => self.lock(l, now_ms),

            // ---- locked (hands-free) ----
            (Mode::Locked, Input::Hotkey(PttDown)) => {
                let since_lock = now_ms.saturating_sub(l.locked_at.unwrap_or(l.since_ms));
                if since_lock < p.debounce_ms {
                    self.cancel_listening(l.session) // triple press
                } else {
                    self.stop(l.session, now_ms)
                }
            }
            (Mode::Locked, Input::Hotkey(HandsFreeToggle)) => {
                if elapsed < p.debounce_ms {
                    self.cancel_listening(l.session) // quick double
                } else {
                    self.stop(l.session, now_ms)
                }
            }

            // ---- clock ----
            (_, Input::Tick) => {
                if l.early_release && elapsed >= p.debounce_ms {
                    self.cancel_listening(l.session)
                } else if elapsed >= p.max_ms {
                    self.stop(l.session, now_ms)
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

    fn handle_processing(
        &mut self,
        session: SessionId,
        since_ms: u64,
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
            } if s == session => match outcome {
                CommitOutcome::Text(text) => vec![Effect::Paste { session, text }],
                CommitOutcome::Empty => {
                    self.phase = Phase::Idle;
                    vec![Effect::Overlay(OverlayState::Resting)]
                }
                CommitOutcome::Failed(message) => self.fail(message, now_ms),
            },
            Input::PasteFinished { session: s, result } if s == session => match result {
                // No success pill: back to rest, the paste sound is the feedback.
                Ok(()) => {
                    self.phase = Phase::Pasted { since_ms: now_ms };
                    vec![
                        Effect::Sound(Sound::Paste),
                        Effect::Overlay(OverlayState::Resting),
                    ]
                }
                Err(message) => self.fail(message, now_ms),
            },
            Input::Tick if now_ms.saturating_sub(since_ms) >= self.policy.processing_timeout_ms => {
                let mut fx = vec![Effect::CancelSession(session)];
                fx.extend(self.fail("Transcription timed out".into(), now_ms));
                fx
            }
            _ => Vec::new(),
        }
    }

    fn begin(&mut self, mode: Mode, now_ms: u64) -> Vec<Effect> {
        let session = self.next_session;
        self.next_session += 1;
        self.phase = Phase::Listening(Listening {
            mode,
            session,
            since_ms: now_ms,
            warned: false,
            early_release: false,
            locked_at: (mode == Mode::Locked).then_some(now_ms),
        });
        let (sound, overlay) = match mode {
            Mode::Ptt => (Sound::Start, OverlayState::ActivePtt),
            Mode::Locked => (Sound::Lock, OverlayState::ActiveLocked),
        };
        vec![
            Effect::Sound(sound),
            Effect::Duck,
            Effect::BeginSession(session),
            Effect::Overlay(overlay),
        ]
    }

    fn lock(&mut self, mut l: Listening, now_ms: u64) -> Vec<Effect> {
        l.mode = Mode::Locked;
        l.early_release = false;
        l.locked_at = Some(now_ms);
        self.phase = Phase::Listening(l);
        vec![
            Effect::Sound(Sound::Lock),
            Effect::Overlay(OverlayState::ActiveLocked),
        ]
    }

    fn stop(&mut self, session: SessionId, now_ms: u64) -> Vec<Effect> {
        self.phase = Phase::Processing {
            session,
            since_ms: now_ms,
        };
        vec![
            Effect::EndCapture,
            Effect::Unduck,
            Effect::Sound(Sound::Stop),
            Effect::Commit(session),
            Effect::Overlay(OverlayState::Processing),
        ]
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

    fn fail(&mut self, message: String, now_ms: u64) -> Vec<Effect> {
        self.phase = Phase::Error { since_ms: now_ms };
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
                since_ms: 500
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
        assert!(listening(&d).early_release);
        assert!(d.handle(Input::Tick, 1499).is_empty());
        assert_eq!(d.handle(Input::Tick, 1500), cancel_fx(s));
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
    fn late_second_tap_after_debounce_dismisses() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 100);
        assert_eq!(
            hk(&mut d, PttDown, 600),
            cancel_fx(s),
            "no tick arrived yet"
        );
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
        assert!(d.handle(Input::Tick, 1699).is_empty());
        assert_eq!(
            d.handle(Input::Tick, 1700),
            vec![Effect::Overlay(OverlayState::Resting)]
        );
        assert_eq!(d.phase(), &Phase::Idle);
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
        assert_eq!(d.phase(), &Phase::Error { since_ms: 10 });
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

    #[test]
    fn policy_matches_wispr_timings() {
        let p = Policy::default();
        assert_eq!(p.max_ms, 1_200_000);
        assert_eq!(p.warn_ms, 1_140_000);
        assert_eq!(p.debounce_ms, 500);
        assert_eq!(p.processing_timeout_ms, 35_000);
        assert_eq!(p.error_linger_ms, 500);
        assert_eq!(p.max_samples(), 20 * 60 * 16_000);
    }
}
