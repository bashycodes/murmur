//! "Press the keys" shortcut recording (platform-independent).
//!
//! Settings' Change button asks the hook to record instead of match: the
//! user holds the keys of the new shortcut and lets go, and the chord they
//! held is what gets saved. The low-level hook sees what the webview
//! can't: Right Alt apart from AltGr's Ctrl+Alt, the Win key, and keys the
//! hook itself swallows. [`ChordRecorder`] is the hook-free half, fed raw
//! hook events (vk, scan code, down/up) and answering when the chord is
//! complete.

use super::chord::{is_altgr_fake_ctrl, Chord, Key};
use std::collections::BTreeSet;

/// Win32 `VK_ESCAPE`.
const VK_ESCAPE: u16 = 0x1B;

/// What one key event means for the recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordStep {
    /// Still recording (keys are held, or nothing pressed yet).
    Pending,
    /// Every key was released: this is the chord the user held.
    Done(Chord),
    /// Esc on its own: the user gave up.
    Cancelled,
}

/// Verdict for one hook event while recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDecision {
    pub step: RecordStep,
    /// Swallow the event: no app (nor the Settings page) sees the keys of
    /// a shortcut being recorded. A key-up of a key that was already down
    /// when recording began passes, so nothing is left logically stuck.
    pub suppress: bool,
}

/// Collects the keys held during a recording; done when all are let go.
#[derive(Debug, Clone, Default)]
pub struct ChordRecorder {
    /// Keys down right now (pressed after recording began).
    held: BTreeSet<Key>,
    /// Every key pressed since recording began: the chord.
    seen: BTreeSet<Key>,
}

impl ChordRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one hook event as Windows reports it.
    pub fn handle_raw(&mut self, vk: u16, scan: u32, down: bool) -> RecordDecision {
        self.handle_hook(vk, scan, down, false)
    }

    /// [`handle_raw`](Self::handle_raw) plus `os_down`: the OS already had
    /// this key down before the event (GetAsyncKeyState inside the hook,
    /// which runs before the async state is updated). A key-down we hold
    /// nothing for while the OS has the key down is the auto-repeat of a
    /// key held since before recording began (a modifier held while
    /// clicking Record): it passes and is not recorded, so its key-up
    /// passes too and the key can't stick.
    pub fn handle_hook(
        &mut self,
        vk: u16,
        scan: u32,
        down: bool,
        os_down: bool,
    ) -> RecordDecision {
        // AltGr's fake Left Ctrl is not a key the user pressed; let it
        // through untouched (like the matcher does) so Ctrl never sticks.
        if is_altgr_fake_ctrl(vk, scan) {
            return RecordDecision {
                step: RecordStep::Pending,
                suppress: false,
            };
        }
        let key = Key::from_vk(vk);
        if down && os_down && !self.held.contains(&key) {
            return RecordDecision {
                step: RecordStep::Pending,
                suppress: false,
            };
        }
        if down {
            if vk == VK_ESCAPE && self.seen.is_empty() {
                return RecordDecision {
                    step: RecordStep::Cancelled,
                    suppress: true,
                };
            }
            // Auto-repeat re-sends the key-down: sets make it a no-op.
            self.held.insert(key);
            self.seen.insert(key);
            return RecordDecision {
                step: RecordStep::Pending,
                suppress: true,
            };
        }
        if !self.held.remove(&key) {
            // Down before recording began (e.g. the mouse click's
            // modifier): not part of the chord, and its up must reach
            // the OS.
            return RecordDecision {
                step: RecordStep::Pending,
                suppress: false,
            };
        }
        let step = if self.held.is_empty() {
            RecordStep::Done(recorded_chord(&self.seen))
        } else {
            RecordStep::Pending
        };
        RecordDecision {
            step,
            suppress: true,
        }
    }
}

/// The chord to save for the keys the user held.
///
/// A lone key keeps its side: holding Right Alt records "Right Alt", which
/// matches Right Alt only (and never AltGr's Ctrl+Alt). In a combination
/// the modifiers fold to either side ("Ctrl+Win", as the defaults are
/// written, matches with either Ctrl), except Right Alt, which stays Right
/// Alt: on AltGr layouts it is a different key from Left Alt.
pub fn recorded_chord(keys: &BTreeSet<Key>) -> Chord {
    if keys.len() <= 1 {
        return Chord::new(keys.iter().copied());
    }
    let folded: BTreeSet<Key> = keys
        .iter()
        .map(|k| match k {
            Key::RAlt => Key::RAlt,
            k => k.either_side(),
        })
        .collect();
    // Left Alt + Right Alt would fold to "Alt+Right Alt", which names
    // Right Alt twice (the parser refuses it): record plain Alt.
    if folded.contains(&Key::RAlt) && folded.contains(&Key::Alt) {
        return Chord::new(folded.into_iter().filter(|k| *k != Key::RAlt));
    }
    Chord::new(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCTRL: u16 = 0xA2;
    const RCTRL: u16 = 0xA3;
    const LWIN: u16 = 0x5B;
    const LALT: u16 = 0xA4;
    const RALT: u16 = 0xA5;
    const LSHIFT: u16 = 0xA0;
    const SPACE: u16 = 0x20;
    const REAL_CTRL_SCAN: u32 = 0x1D;

    /// Press `keys` in order, then release them in order; the final step.
    fn press_release(r: &mut ChordRecorder, keys: &[u16]) -> RecordStep {
        for &k in keys {
            assert_eq!(r.handle_raw(k, 0, true).step, RecordStep::Pending);
        }
        let mut last = RecordStep::Pending;
        for (i, &k) in keys.iter().enumerate() {
            last = r.handle_raw(k, 0, false).step;
            if i + 1 < keys.len() {
                assert_eq!(last, RecordStep::Pending, "done before all were released");
            }
        }
        last
    }

    fn done(s: RecordStep) -> String {
        match s {
            RecordStep::Done(c) => c.to_string(),
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn right_alt_alone_is_right_alt() {
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[RALT])), "Right Alt");
    }

    #[test]
    fn altgr_records_right_alt_not_ctrl_alt() {
        // AltGr = fake Left Ctrl (scan 0x21D) + Right Alt.
        let mut r = ChordRecorder::new();
        let fake = r.handle_raw(LCTRL, 0x21D, true);
        assert!(!fake.suppress, "AltGr's fake Ctrl must pass through");
        assert!(r.handle_raw(RALT, 0x38, true).suppress);
        assert_eq!(r.handle_raw(LCTRL, 0x21D, false).step, RecordStep::Pending);
        assert_eq!(done(r.handle_raw(RALT, 0x38, false).step), "Right Alt");
    }

    #[test]
    fn lone_modifiers_keep_their_side() {
        for (vk, name) in [(RCTRL, "Right Ctrl"), (LALT, "Left Alt"), (LSHIFT, "Left Shift")] {
            let mut r = ChordRecorder::new();
            assert_eq!(done(press_release(&mut r, &[vk])), name);
        }
    }

    #[test]
    fn combinations_fold_to_either_side_in_display_order() {
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LWIN, LCTRL])), "Ctrl+Win");
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LCTRL, LWIN, SPACE])), "Ctrl+Win+Space");
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LWIN, LALT, b'1' as u16])), "Win+Alt+1");
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LSHIFT, LALT, b'Z' as u16])), "Alt+Shift+Z");
    }

    #[test]
    fn right_alt_stays_right_alt_in_a_combination() {
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[RALT, SPACE])), "Right Alt+Space");
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LALT, RALT])), "Alt");
    }

    #[test]
    fn chord_is_everything_held_even_if_released_early() {
        // Ctrl down, Win down, Ctrl up (Win still held), Win up.
        let mut r = ChordRecorder::new();
        r.handle_raw(LCTRL, REAL_CTRL_SCAN, true);
        r.handle_raw(LWIN, 0, true);
        assert_eq!(r.handle_raw(LCTRL, REAL_CTRL_SCAN, false).step, RecordStep::Pending);
        assert_eq!(done(r.handle_raw(LWIN, 0, false).step), "Ctrl+Win");
    }

    #[test]
    fn auto_repeat_does_not_add_keys() {
        let mut r = ChordRecorder::new();
        for _ in 0..5 {
            r.handle_raw(RALT, 0, true);
        }
        assert_eq!(done(r.handle_raw(RALT, 0, false).step), "Right Alt");
    }

    #[test]
    fn a_key_held_before_recording_repeats_and_releases_through() {
        // Ctrl held while clicking Record: its repeats look like fresh
        // key-downs, but the OS already has it down.
        let mut r = ChordRecorder::new();
        let ctrl = 0xA2;
        for _ in 0..3 {
            let d = r.handle_hook(ctrl, 0x1D, true, true);
            assert_eq!(d.step, RecordStep::Pending);
            assert!(!d.suppress, "repeat of a key held before recording");
        }
        let up = r.handle_hook(ctrl, 0x1D, false, true);
        assert!(!up.suppress, "its key-up reaches the OS: Ctrl can't stick");
        assert_eq!(up.step, RecordStep::Pending, "nothing was recorded");
        // A real press afterwards records normally.
        assert!(r.handle_hook(0xA5, 0x38, true, false).suppress);
        assert_eq!(
            r.handle_hook(0xA5, 0x38, false, true).step,
            RecordStep::Done("Right Alt".parse().unwrap())
        );
    }

    #[test]
    fn esc_alone_cancels_and_is_swallowed() {
        let mut r = ChordRecorder::new();
        let d = r.handle_raw(0x1B, 1, true);
        assert_eq!(d.step, RecordStep::Cancelled);
        assert!(d.suppress);
    }

    #[test]
    fn esc_after_a_modifier_is_part_of_the_chord() {
        let mut r = ChordRecorder::new();
        assert_eq!(done(press_release(&mut r, &[LSHIFT, 0x1B])), "Shift+Esc");
    }

    #[test]
    fn key_up_of_a_key_held_before_recording_passes_through() {
        let mut r = ChordRecorder::new();
        let d = r.handle_raw(LCTRL, REAL_CTRL_SCAN, false);
        assert_eq!(d.step, RecordStep::Pending);
        assert!(!d.suppress);
        // And it is not part of the chord recorded next.
        assert_eq!(done(press_release(&mut r, &[RALT])), "Right Alt");
    }

    #[test]
    fn recorded_chords_round_trip_through_the_parser() {
        let mut r = ChordRecorder::new();
        // An OEM key without a name (VK_OEM_1 = 0xBA) still saves and parses.
        let c = done(press_release(&mut r, &[LCTRL, 0xBA]));
        assert_eq!(c, "Ctrl+VKBA");
        assert_eq!(Chord::parse_validated(&c, false).unwrap().to_string(), c);
        for s in ["Right Alt", "Ctrl+Win+Space", "Win+Alt+1", "Right Alt+Space"] {
            assert!(Chord::parse_validated(s, false).is_ok(), "{s}");
        }
    }

    #[test]
    fn a_lone_letter_records_but_fails_validation() {
        // Recording reports what was held; the save refuses it as before.
        let mut r = ChordRecorder::new();
        let c = done(press_release(&mut r, &[b'A' as u16]));
        assert_eq!(c, "A");
        assert!(Chord::parse_validated(&c, false).is_err());
    }
}
