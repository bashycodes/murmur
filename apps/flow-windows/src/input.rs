//! Platform-independent description of synthetic keyboard input.
//!
//! Paste, modifier release and Win/Alt masking are all expressed as a
//! `Vec<Stroke>`; the Windows backend (`input::win`) turns that into one
//! atomic `SendInput` call. Keeping the sequences pure lets us unit-test the
//! exact keystrokes we inject.

use crate::hotkeys::Key;

/// VK_CONTROL, VK_V, VK_RETURN …
pub mod vk {
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const RETURN: u16 = 0x0D;
    pub const INSERT: u16 = 0x2D;
    pub const V: u16 = 0x56;
    /// Unassigned VK used as a harmless "something else was pressed" marker.
    pub const MASK: u16 = 0xFF;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stroke {
    Vk {
        vk: u16,
        up: bool,
    },
    /// KEYEVENTF_UNICODE with one UTF-16 code unit.
    Unicode {
        unit: u16,
        up: bool,
    },
}

impl Stroke {
    pub fn down(vk: u16) -> Stroke {
        Stroke::Vk { vk, up: false }
    }
    pub fn up(vk: u16) -> Stroke {
        Stroke::Vk { vk, up: true }
    }
}

/// Dummy VK 0xFF tap: tells the shell "a key was pressed while Win/Alt was
/// down" so the following Win/Alt release does not open Start / the menu bar.
pub fn mask_sequence() -> Vec<Stroke> {
    vec![Stroke::down(vk::MASK), Stroke::up(vk::MASK)]
}

/// Concrete VK codes that may be physically held for a normalised key (used
/// both to release them and for GetAsyncKeyState stale-key probes).
pub fn vks_for(key: Key) -> Vec<u16> {
    match key {
        Key::Ctrl => vec![0xA2, 0xA3],
        Key::Win => vec![vk::LWIN, vk::RWIN],
        Key::Alt => vec![0xA4, 0xA5],
        Key::Shift => vec![0xA0, 0xA1],
        Key::Space => vec![0x20],
        Key::Esc => vec![0x1B],
        Key::Vk(v) => vec![v],
    }
}

/// Key-ups for every held modifier, so our Ctrl+V isn't seen as e.g.
/// Ctrl+Win+V (clipboard history!) while the user still holds the PTT chord.
/// Win/Alt releases are preceded by a mask tap.
pub fn release_modifiers(held: &[Key]) -> Vec<Stroke> {
    let mut out = Vec::new();
    let needs_mask = held.iter().any(|k| k.needs_release_mask());
    if needs_mask {
        out.extend(mask_sequence());
    }
    for k in held.iter().filter(|k| k.is_modifier()) {
        for v in vks_for(*k) {
            out.push(Stroke::up(v));
        }
    }
    out
}

/// The paste keystroke: Ctrl+V, or Shift+Insert for terminals/remote viewers.
pub fn paste_chord(shift_insert: bool) -> Vec<Stroke> {
    if shift_insert {
        vec![
            Stroke::down(vk::SHIFT),
            Stroke::down(vk::INSERT),
            Stroke::up(vk::INSERT),
            Stroke::up(vk::SHIFT),
        ]
    } else {
        vec![
            Stroke::down(vk::CONTROL),
            Stroke::down(vk::V),
            Stroke::up(vk::V),
            Stroke::up(vk::CONTROL),
        ]
    }
}

/// KEYEVENTF_UNICODE typing fallback. Newlines become VK_RETURN (many apps
/// ignore U+000A typed as a character); CRLF collapses to one Enter.
pub fn unicode_text(text: &str) -> Vec<Stroke> {
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for ch in normalised.chars() {
        if ch == '\n' {
            out.push(Stroke::down(vk::RETURN));
            out.push(Stroke::up(vk::RETURN));
            continue;
        }
        let mut buf = [0u16; 2];
        for unit in ch.encode_utf16(&mut buf).iter().copied() {
            out.push(Stroke::Unicode { unit, up: false });
            out.push(Stroke::Unicode { unit, up: true });
        }
    }
    out
}

#[cfg(windows)]
pub mod win {
    use super::Stroke;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY,
    };

    /// Tag on dwExtraInfo so our own hook can recognise our injections even
    /// if LLKHF_INJECTED filtering is ever relaxed.
    pub const EXTRA_INFO_TAG: usize = 0x464C_4F57; // "FLOW"

    fn to_input(s: Stroke) -> INPUT {
        let (vk, scan, flags) = match s {
            Stroke::Vk { vk, up } => (
                vk,
                0u16,
                if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
            ),
            Stroke::Unicode { unit, up } => (
                0,
                unit,
                KEYEVENTF_UNICODE
                    | if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
            ),
        };
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: EXTRA_INFO_TAG,
                },
            },
        }
    }

    /// Inject all strokes atomically. Returns the number of events inserted
    /// (fewer than requested = blocked, e.g. by UIPI against an elevated app).
    pub fn send(strokes: &[Stroke]) -> usize {
        if strokes.is_empty() {
            return 0;
        }
        let inputs: Vec<INPUT> = strokes.iter().copied().map(to_input).collect();
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_is_vk_ff_tap() {
        assert_eq!(mask_sequence(), vec![Stroke::down(0xFF), Stroke::up(0xFF)]);
    }

    #[test]
    fn release_modifiers_masks_before_win_release() {
        let s = release_modifiers(&[Key::Ctrl, Key::Win]);
        assert_eq!(&s[..2], &mask_sequence()[..]);
        assert!(s.contains(&Stroke::up(0xA2)));
        assert!(s.contains(&Stroke::up(0x5B)));
        assert!(s.contains(&Stroke::up(0x5C)));
        assert!(s[2..]
            .iter()
            .all(|x| matches!(x, Stroke::Vk { up: true, .. })));
    }

    #[test]
    fn release_modifiers_ctrl_only_needs_no_mask_and_skips_non_modifiers() {
        let s = release_modifiers(&[Key::Ctrl, Key::Space]);
        assert_eq!(s, vec![Stroke::up(0xA2), Stroke::up(0xA3)]);
        assert!(release_modifiers(&[]).is_empty());
    }

    #[test]
    fn paste_chord_variants() {
        assert_eq!(
            paste_chord(false),
            vec![
                Stroke::down(0x11),
                Stroke::down(0x56),
                Stroke::up(0x56),
                Stroke::up(0x11)
            ]
        );
        assert_eq!(paste_chord(true)[1], Stroke::down(0x2D));
    }

    #[test]
    fn unicode_text_emits_down_up_per_utf16_unit() {
        let s = unicode_text("hé");
        assert_eq!(
            s,
            vec![
                Stroke::Unicode {
                    unit: 'h' as u16,
                    up: false
                },
                Stroke::Unicode {
                    unit: 'h' as u16,
                    up: true
                },
                Stroke::Unicode {
                    unit: 0xE9,
                    up: false
                },
                Stroke::Unicode {
                    unit: 0xE9,
                    up: true
                },
            ]
        );
    }

    #[test]
    fn unicode_text_surrogate_pairs() {
        let s = unicode_text("😀");
        assert_eq!(s.len(), 4);
        assert_eq!(
            s[0],
            Stroke::Unicode {
                unit: 0xD83D,
                up: false
            }
        );
        assert_eq!(
            s[2],
            Stroke::Unicode {
                unit: 0xDE00,
                up: false
            }
        );
    }

    #[test]
    fn unicode_text_newlines_become_enter() {
        let s = unicode_text("a\r\nb\n");
        let enters = s.iter().filter(|x| **x == Stroke::down(vk::RETURN)).count();
        assert_eq!(enters, 2);
        assert_eq!(s.len(), 2 + 2 + 2 + 2);
    }
}
