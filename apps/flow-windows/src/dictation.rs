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
pub enum Phase {
    Idle,
    Listening {
        mode: Mode,
        session: SessionId,
        since_ms: u64,
        warned: bool,
    },
    Processing {
        session: SessionId,
    },
    Pasted {
        since_ms: u64,
    },
    Error {
        since_ms: u64,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Hard stop: the dictation is committed (not discarded) at this length.
    pub max_ms: u64,
    pub warn_ms: u64,
    /// PTT taps shorter than this are treated as accidental and discarded.
    pub min_ptt_ms: u64,
    pub pasted_linger_ms: u64,
    pub error_linger_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            max_ms: 20 * 60 * 1000,
            warn_ms: 19 * 60 * 1000,
            min_ptt_ms: 250,
            pasted_linger_ms: 800,
            error_linger_ms: 2500,
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
        matches!(
            self.phase,
            Phase::Listening { .. } | Phase::Processing { .. }
        )
    }

    pub fn handle(&mut self, input: Input, now_ms: u64) -> Vec<Effect> {
        use HotkeyAction::*;
        match (self.phase.clone(), input) {
            // ---- start -------------------------------------------------
            (Phase::Idle | Phase::Pasted { .. } | Phase::Error { .. }, Input::Hotkey(PttDown)) => {
                self.begin(Mode::Ptt, now_ms)
            }
            (
                Phase::Idle | Phase::Pasted { .. } | Phase::Error { .. },
                Input::Hotkey(HandsFreeToggle),
            ) => self.begin(Mode::Locked, now_ms),

            // ---- listening ---------------------------------------------
            (
                Phase::Listening {
                    mode: Mode::Ptt,
                    session,
                    since_ms,
                    warned,
                },
                Input::Hotkey(HandsFreeToggle),
            ) => {
                self.phase = Phase::Listening {
                    mode: Mode::Locked,
                    session,
                    since_ms,
                    warned,
                };
                vec![
                    Effect::Sound(Sound::Lock),
                    Effect::Overlay(OverlayState::ActiveLocked),
                ]
            }
            (
                Phase::Listening {
                    mode: Mode::Ptt,
                    session,
                    since_ms,
                    ..
                },
                Input::Hotkey(PttUp),
            ) => {
                if now_ms.saturating_sub(since_ms) < self.policy.min_ptt_ms {
                    self.cancel_listening(session)
                } else {
                    self.stop(session)
                }
            }
            (
                Phase::Listening {
                    mode: Mode::Ptt,
                    session,
                    ..
                },
                Input::Hotkey(PttAborted),
            ) => self.cancel_listening(session),
            (
                Phase::Listening {
                    mode: Mode::Locked,
                    session,
                    ..
                },
                Input::Hotkey(HandsFreeToggle),
            ) => self.stop(session),
            (Phase::Listening { session, .. }, Input::Hotkey(Cancel)) => {
                self.cancel_listening(session)
            }
            (
                Phase::Listening { session, .. },
                Input::StartFailed {
                    session: s,
                    message,
                },
            ) if s == session => {
                let mut fx = self.cancel_listening(session);
                fx.pop(); // replace the "resting" overlay with the error
                fx.extend(self.fail(message, now_ms));
                fx
            }
            (
                Phase::Listening {
                    mode,
                    session,
                    since_ms,
                    warned,
                },
                Input::Tick,
            ) => {
                let elapsed = now_ms.saturating_sub(since_ms);
                if elapsed >= self.policy.max_ms {
                    self.stop(session)
                } else if elapsed >= self.policy.warn_ms && !warned {
                    self.phase = Phase::Listening {
                        mode,
                        session,
                        since_ms,
                        warned: true,
                    };
                    vec![Effect::MaxDurationWarning {
                        remaining_ms: self.policy.max_ms - elapsed,
                    }]
                } else {
                    Vec::new()
                }
            }

            // ---- processing --------------------------------------------
            (Phase::Processing { session }, Input::Hotkey(Cancel)) => {
                self.phase = Phase::Idle;
                vec![
                    Effect::CancelSession(session),
                    Effect::Overlay(OverlayState::Resting),
                ]
            }
            (
                Phase::Processing { session },
                Input::Committed {
                    session: s,
                    outcome,
                },
            ) if s == session => match outcome {
                CommitOutcome::Text(text) => vec![Effect::Paste { session, text }],
                CommitOutcome::Empty => {
                    self.phase = Phase::Idle;
                    vec![Effect::Overlay(OverlayState::Resting)]
                }
                CommitOutcome::Failed(message) => self.fail(message, now_ms),
            },
            (Phase::Processing { session }, Input::PasteFinished { session: s, result })
                if s == session =>
            {
                match result {
                    Ok(()) => {
                        self.phase = Phase::Pasted { since_ms: now_ms };
                        vec![
                            Effect::Sound(Sound::Paste),
                            Effect::Overlay(OverlayState::Resting),
                        ]
                    }
                    Err(message) => self.fail(message, now_ms),
                }
            }

            // ---- linger ------------------------------------------------
            (Phase::Pasted { since_ms }, Input::Tick)
                if now_ms.saturating_sub(since_ms) >= self.policy.pasted_linger_ms =>
            {
                self.phase = Phase::Idle;
                Vec::new()
            }
            (Phase::Error { since_ms }, Input::Tick)
                if now_ms.saturating_sub(since_ms) >= self.policy.error_linger_ms =>
            {
                self.phase = Phase::Idle;
                vec![Effect::Overlay(OverlayState::Resting)]
            }

            _ => Vec::new(),
        }
    }

    fn begin(&mut self, mode: Mode, now_ms: u64) -> Vec<Effect> {
        let session = self.next_session;
        self.next_session += 1;
        self.phase = Phase::Listening {
            mode,
            session,
            since_ms: now_ms,
            warned: false,
        };
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

    fn stop(&mut self, session: SessionId) -> Vec<Effect> {
        self.phase = Phase::Processing { session };
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
        assert!(matches!(
            d.phase(),
            Phase::Listening {
                mode: Mode::Ptt,
                ..
            }
        ));
        assert!(d.is_active());
    }

    #[test]
    fn ptt_up_commits() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        let fx = hk(&mut d, PttUp, 2000);
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
        assert_eq!(d.phase(), &Phase::Processing { session: s });
    }

    #[test]
    fn short_ptt_tap_is_discarded() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 1000));
        let fx = hk(&mut d, PttUp, 1100);
        assert_eq!(
            fx,
            vec![
                Effect::EndCapture,
                Effect::Unduck,
                Effect::CancelSession(s),
                Effect::Overlay(OverlayState::Resting),
            ]
        );
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn commit_text_pastes_then_pasted_then_idle_after_linger() {
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
        assert_eq!(d.phase(), &Phase::Pasted { since_ms: 1700 });
        assert!(!d.is_active());
        assert!(d.handle(Input::Tick, 2000).is_empty());
        d.handle(Input::Tick, 2600);
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
    fn failed_commit_shows_error_then_idle() {
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
        assert_eq!(d.phase(), &Phase::Error { since_ms: 1200 });
        d.handle(Input::Tick, 1200 + 2500);
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
    fn ptt_then_space_locks_without_new_session() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        let fx = hk(&mut d, HandsFreeToggle, 100);
        assert_eq!(
            fx,
            vec![
                Effect::Sound(Sound::Lock),
                Effect::Overlay(OverlayState::ActiveLocked)
            ]
        );
        assert!(matches!(
            d.phase(),
            Phase::Listening { mode: Mode::Locked, session, since_ms: 0, .. } if *session == s
        ));
    }

    #[test]
    fn hands_free_from_idle_and_second_toggle_commits() {
        let mut d = d();
        let fx = hk(&mut d, HandsFreeToggle, 0);
        let s = session_of(&fx);
        assert_eq!(fx[0], Effect::Sound(Sound::Lock));
        assert_eq!(fx[1], Effect::Duck);
        assert_eq!(fx[3], Effect::Overlay(OverlayState::ActiveLocked));
        // PTT chord edges are ignored while locked.
        assert!(hk(&mut d, PttDown, 50).is_empty());
        assert!(hk(&mut d, PttUp, 60).is_empty());
        assert!(hk(&mut d, PttAborted, 70).is_empty());
        let fx = hk(&mut d, HandsFreeToggle, 100);
        assert!(
            fx.contains(&Effect::Commit(s)),
            "no min-duration for locked"
        );
    }

    #[test]
    fn cancel_while_listening() {
        let mut d = d();
        let s = session_of(&hk(&mut d, HandsFreeToggle, 0));
        let fx = hk(&mut d, Cancel, 5000);
        assert_eq!(
            fx,
            vec![
                Effect::EndCapture,
                Effect::Unduck,
                Effect::CancelSession(s),
                Effect::Overlay(OverlayState::Resting),
            ]
        );
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn ptt_aborted_cancels() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        let fx = hk(&mut d, PttAborted, 3000);
        assert!(fx.contains(&Effect::CancelSession(s)));
        assert_eq!(d.phase(), &Phase::Idle);
    }

    #[test]
    fn cancel_while_processing_drops_late_result() {
        let mut d = d();
        let s = session_of(&hk(&mut d, PttDown, 0));
        hk(&mut d, PttUp, 1000);
        let fx = hk(&mut d, Cancel, 1100);
        assert_eq!(
            fx,
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
        assert_eq!(d.phase(), &Phase::Processing { session: s2 });
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
    fn new_dictation_can_start_during_pasted_or_error_linger() {
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
        assert_eq!(d.phase(), &Phase::Processing { session: s });
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
        // A later PttUp for the dead session does nothing.
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
        assert!(matches!(d.phase(), Phase::Listening { session, .. } if *session == s2));
    }

    #[test]
    fn max_samples_is_20_min_at_16k() {
        assert_eq!(Policy::default().max_samples(), 20 * 60 * 16_000);
    }
}
