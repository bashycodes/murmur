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
    pub const TAB: u16 = 0x09;
    pub const INSERT: u16 = 0x2D;
    pub const V: u16 = 0x56;
    /// Masking key for lone Win/Alt releases (Wispr uses VK_LCONTROL, not 0xFF).
    pub const LCONTROL: u16 = 0xA2;
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

/// Masked release of Win/Alt (`vk` = the concrete L/R key), replacing the
/// physical key-up the hook swallowed: `[LCtrl↓, key↑, LCtrl↑]`. The shell
/// sees "another key was involved", so Start / the menu bar don't open.
/// The trailing LCtrl↑ is skipped when LCtrl is physically held (the user's
/// Ctrl must stay logically down). Mirrors Wispr's helper (02 §REA note 3).
pub fn masked_release(vk: u16, lctrl_held: bool) -> Vec<Stroke> {
    let mut v = vec![Stroke::down(vk::LCONTROL), Stroke::up(vk)];
    if !lctrl_held {
        v.push(Stroke::up(vk::LCONTROL));
    }
    v
}

/// Modifiers that get a synthetic (masked) key-up before the paste chord.
pub const RELEASE_BEFORE_PASTE_VKS: [u16; 5] = [vk::MENU, 0xA4, 0xA5, vk::LWIN, vk::RWIN];

/// Hold time between the paste chord's down and up batches.
pub const PASTE_HOLD_MS: u64 = 50;

/// For each physically-held Alt/Win: `[Ctrl↓, key↑, Ctrl↑]`, so our Ctrl+V
/// isn't read as Ctrl+Win+V / Ctrl+Alt+V while the user still holds the PTT
/// chord. Held Ctrl/Shift are left alone (Wispr parity, 02 §REA note 6).
pub fn release_before_paste(held_vks: &[u16]) -> Vec<Stroke> {
    held_vks
        .iter()
        .filter(|v| RELEASE_BEFORE_PASTE_VKS.contains(v))
        .flat_map(|v| {
            [
                Stroke::down(vk::CONTROL),
                Stroke::up(*v),
                Stroke::up(vk::CONTROL),
            ]
        })
        .collect()
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

/// The paste chord as (down batch, up batch): `[Ctrl↓, V↓]` … 50 ms …
/// `[V↑, Ctrl↑]` (or Shift+Insert). Sent as two `SendInput` calls.
pub fn paste_chord(shift_insert: bool) -> (Vec<Stroke>, Vec<Stroke>) {
    let keys = if shift_insert {
        [vk::SHIFT, vk::INSERT]
    } else {
        [vk::CONTROL, vk::V]
    };
    let downs = keys.iter().map(|k| Stroke::down(*k)).collect();
    let ups = keys.iter().rev().map(|k| Stroke::up(*k)).collect();
    (downs, ups)
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

/// `dwExtraInfo` on every event we inject; our hook passes events carrying
/// it straight to `CallNextHookEx` (Wispr does the same with "WFMK").
pub const EXTRA_INFO_TAG: usize = 0x464C_4F57; // "FLOW"

#[cfg(windows)]
pub mod win {
    use super::{vk, Stroke, EXTRA_INFO_TAG};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
    };

    fn to_input(s: Stroke) -> INPUT {
        let (vk, scan, flags) = match s {
            Stroke::Vk { vk: v, up } => {
                let mut f = if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                };
                if v == vk::INSERT {
                    f |= KEYEVENTF_EXTENDEDKEY;
                }
                (v, 0u16, f)
            }
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

    // Wispr helper 0x1403235c0 (02 §REA note 3): [LCtrl↓, key↑, LCtrl↑].
    #[test]
    fn masked_release_wraps_key_up_in_lcontrol() {
        assert_eq!(
            masked_release(0x5B, false),
            vec![Stroke::down(0xA2), Stroke::up(0x5B), Stroke::up(0xA2)]
        );
    }

    #[test]
    fn masked_release_keeps_physically_held_lcontrol_down() {
        assert_eq!(
            masked_release(0x5C, true),
            vec![Stroke::down(0xA2), Stroke::up(0x5C)]
        );
    }

    // 02 §REA note 6: only held Alt/Win get [Ctrl↓, key↑, Ctrl↑]; Ctrl and
    // Shift are left alone.
    #[test]
    fn release_before_paste_only_touches_alt_and_win() {
        assert_eq!(
            release_before_paste(&[0xA2, 0x5B, 0xA0, 0xA4]),
            vec![
                Stroke::down(0x11),
                Stroke::up(0x5B),
                Stroke::up(0x11),
                Stroke::down(0x11),
                Stroke::up(0xA4),
                Stroke::up(0x11),
            ]
        );
        assert!(release_before_paste(&[0xA2, 0xA1]).is_empty());
        assert!(release_before_paste(&[]).is_empty());
    }

    // 02 §REA note 4: downs in one SendInput, 50 ms, ups in reverse order.
    #[test]
    fn paste_chord_is_down_batch_then_reversed_up_batch() {
        let (d, u) = paste_chord(false);
        assert_eq!(d, vec![Stroke::down(0x11), Stroke::down(0x56)]);
        assert_eq!(u, vec![Stroke::up(0x56), Stroke::up(0x11)]);
        let (d, u) = paste_chord(true);
        assert_eq!(d, vec![Stroke::down(0x10), Stroke::down(0x2D)]);
        assert_eq!(u, vec![Stroke::up(0x2D), Stroke::up(0x10)]);
        assert_eq!(PASTE_HOLD_MS, 50);
    }

    #[test]
    fn modifier_vk_lists() {
        assert!(RELEASE_BEFORE_PASTE_VKS.contains(&0x12));
        assert!(RELEASE_BEFORE_PASTE_VKS.contains(&0x5C));
        assert!(!RELEASE_BEFORE_PASTE_VKS.contains(&0xA2));
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
