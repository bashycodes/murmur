//! Platform-independent chord matcher.
//!
//! The LL keyboard hook feeds every physical key event in; the matcher answers
//! (a) which hotkey actions fired, (b) whether to swallow the event, and
//! (c) whether a dummy VK 0xFF must be injected *before* letting a Win/Alt
//! key-up through (so a "lone" Win release doesn't open Start, and a lone Alt
//! release doesn't focus the menu bar — windows-research 02 §2).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// A key as the matcher sees it.
///
/// Ctrl, Alt and Shift come in three forms: the side-specific `L*`/`R*`
/// keys the hook reports (VK 0xA0–0xA5), and the either-side `Ctrl`/`Alt`/
/// `Shift` that a shortcut names when it doesn't care which side. In a
/// [`Chord`], `Alt` accepts a held `LAlt` or `RAlt`, while `RAlt` accepts
/// only `RAlt` (see [`Key::accepts`]). Win always folds to one key.
///
/// The variant order is the display order of a chord ("Ctrl+Win+Space"),
/// and is part of the saved format: don't reorder the existing variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Ctrl,
    LCtrl,
    RCtrl,
    Win,
    Alt,
    LAlt,
    RAlt,
    Shift,
    LShift,
    RShift,
    Space,
    Esc,
    /// Any other virtual-key code.
    Vk(u16),
}

/// Win32 `VK_LCONTROL`.
const VK_LCONTROL: u16 = 0xA2;

/// Scan code (`KBDLLHOOKSTRUCT::scanCode`) of the fake Left Ctrl that
/// Windows sends right before Right Alt on keyboard layouts with AltGr
/// (a real Left Ctrl is 0x1D). AutoHotkey calls it `sc21D`.
pub const ALTGR_FAKE_LCTRL_SCAN: u32 = 0x21D;

/// Is this hook event the fake Left Ctrl of an AltGr press?
pub fn is_altgr_fake_ctrl(vk: u16, scan: u32) -> bool {
    vk == VK_LCONTROL && scan == ALTGR_FAKE_LCTRL_SCAN
}

impl Key {
    /// Map a Win32 virtual-key code to a key. The L/R Ctrl/Alt/Shift codes
    /// keep their side; the sideless codes (VK_CONTROL/MENU/SHIFT, seen only
    /// from some injected input) become the either-side key.
    pub fn from_vk(vk: u16) -> Key {
        match vk {
            0x11 => Key::Ctrl,
            0xA2 => Key::LCtrl,
            0xA3 => Key::RCtrl,
            0x5B | 0x5C => Key::Win,
            0x12 => Key::Alt,
            0xA4 => Key::LAlt,
            0xA5 => Key::RAlt,
            0x10 => Key::Shift,
            0xA0 => Key::LShift,
            0xA1 => Key::RShift,
            0x20 => Key::Space,
            0x1B => Key::Esc,
            other => Key::Vk(other),
        }
    }

    /// The either-side key of a side-specific one (`RAlt` → `Alt`); every
    /// other key is its own.
    pub fn either_side(self) -> Key {
        match self {
            Key::LCtrl | Key::RCtrl => Key::Ctrl,
            Key::LAlt | Key::RAlt => Key::Alt,
            Key::LShift | Key::RShift => Key::Shift,
            k => k,
        }
    }

    /// Does this chord key accept `held` being down? `Alt` accepts either
    /// Alt; `RAlt` only Right Alt.
    pub fn accepts(self, held: Key) -> bool {
        self == held || self == held.either_side()
    }

    pub fn is_modifier(self) -> bool {
        matches!(
            self.either_side(),
            Key::Ctrl | Key::Win | Key::Alt | Key::Shift
        )
    }

    /// Keys whose lone release triggers a shell action (Start menu / menu bar).
    pub fn needs_release_mask(self) -> bool {
        matches!(self.either_side(), Key::Win | Key::Alt)
    }
}

/// A set of keys that must all be held simultaneously.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    keys: BTreeSet<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    Empty,
    UnknownKey(String),
    TooManyKeys(usize),
    NoModifier,
    Duplicate(String),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::Empty => write!(f, "empty chord"),
            ChordError::UnknownKey(k) => write!(f, "unknown key {k:?}"),
            ChordError::TooManyKeys(n) => write!(f, "chord has {n} keys (max 3)"),
            ChordError::NoModifier => write!(f, "chord needs at least one modifier"),
            ChordError::Duplicate(k) => write!(f, "key {k:?} repeated"),
        }
    }
}

impl std::error::Error for ChordError {}

impl Chord {
    pub fn new(keys: impl IntoIterator<Item = Key>) -> Chord {
        Chord {
            keys: keys.into_iter().collect(),
        }
    }

    pub fn keys(&self) -> &BTreeSet<Key> {
        &self.keys
    }

    pub fn contains(&self, k: Key) -> bool {
        self.keys.contains(&k)
    }

    /// Is the held key `held` part of this chord (accepted by one of its keys)?
    pub fn covers(&self, held: Key) -> bool {
        self.keys.iter().any(|k| k.accepts(held))
    }

    /// Exactly this chord is held: every chord key has a held key it
    /// accepts, and every held key belongs to the chord.
    pub fn matches(&self, pressed: &BTreeSet<Key>) -> bool {
        self.keys
            .iter()
            .all(|k| pressed.iter().any(|p| k.accepts(*p)))
            && pressed.iter().all(|p| self.covers(*p))
    }

    /// Validation used for user-configured chords (mirrors Wispr's rule:
    /// at most 3 keys, at least one modifier). `allow_single` permits a lone
    /// non-modifier key (used for the cancel key, Esc).
    pub fn parse_validated(s: &str, allow_single: bool) -> Result<Chord, ChordError> {
        let c: Chord = s.parse()?;
        if c.keys.len() > 3 {
            return Err(ChordError::TooManyKeys(c.keys.len()));
        }
        let has_mod = c.keys.iter().any(|k| k.is_modifier());
        let single_ok = allow_single && c.keys.len() == 1;
        if !(has_mod || single_ok) {
            return Err(ChordError::NoModifier);
        }
        Ok(c)
    }
}

fn parse_key(tok: &str) -> Option<Key> {
    let t = tok.trim().to_ascii_lowercase();
    // "Right Alt", "right-alt", "Right_Alt" and "RAlt" are one key.
    let squashed: String = t
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    let side = match squashed.as_str() {
        "lctrl" | "leftctrl" | "lcontrol" | "leftcontrol" => Some(Key::LCtrl),
        "rctrl" | "rightctrl" | "rcontrol" | "rightcontrol" => Some(Key::RCtrl),
        "lalt" | "leftalt" => Some(Key::LAlt),
        "ralt" | "rightalt" | "altgr" => Some(Key::RAlt),
        "lshift" | "leftshift" => Some(Key::LShift),
        "rshift" | "rightshift" => Some(Key::RShift),
        _ => None,
    };
    if side.is_some() {
        return side;
    }
    Some(match t.as_str() {
        "ctrl" | "control" => Key::Ctrl,
        "win" | "super" | "meta" | "windows" => Key::Win,
        "alt" => Key::Alt,
        "shift" => Key::Shift,
        "space" => Key::Space,
        "esc" | "escape" => Key::Esc,
        "enter" | "return" => Key::Vk(0x0D),
        "tab" => Key::Vk(0x09),
        _ => {
            if let Some(n) = t.strip_prefix('f') {
                if let Ok(n) = n.parse::<u16>() {
                    if (1..=24).contains(&n) {
                        return Some(Key::Vk(0x70 + n - 1));
                    }
                }
            }
            if let Some(hex) = t.strip_prefix("vk") {
                return u16::from_str_radix(hex.trim_start_matches("0x"), 16)
                    .ok()
                    .map(Key::from_vk);
            }
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => {
                    Key::Vk(c.to_ascii_uppercase() as u16)
                }
                _ => return None,
            }
        }
    })
}

fn key_name(k: Key) -> String {
    match k {
        Key::Ctrl => "Ctrl".into(),
        Key::LCtrl => "Left Ctrl".into(),
        Key::RCtrl => "Right Ctrl".into(),
        Key::Win => "Win".into(),
        Key::Alt => "Alt".into(),
        Key::LAlt => "Left Alt".into(),
        Key::RAlt => "Right Alt".into(),
        Key::Shift => "Shift".into(),
        Key::LShift => "Left Shift".into(),
        Key::RShift => "Right Shift".into(),
        Key::Space => "Space".into(),
        Key::Esc => "Esc".into(),
        Key::Vk(0x0D) => "Enter".into(),
        Key::Vk(0x09) => "Tab".into(),
        Key::Vk(v @ 0x70..=0x87) => format!("F{}", v - 0x70 + 1),
        Key::Vk(v @ (0x30..=0x39 | 0x41..=0x5A)) => ((v as u8) as char).to_string(),
        Key::Vk(v) => format!("VK{v:02X}"),
    }
}

impl FromStr for Chord {
    type Err = ChordError;
    fn from_str(s: &str) -> Result<Chord, ChordError> {
        let mut keys = BTreeSet::new();
        for tok in s.split('+') {
            if tok.trim().is_empty() {
                continue;
            }
            let k = parse_key(tok).ok_or_else(|| ChordError::UnknownKey(tok.trim().into()))?;
            // "Alt+Right Alt" names Right Alt twice.
            if keys.iter().any(|o: &Key| o.accepts(k) || k.accepts(*o)) {
                return Err(ChordError::Duplicate(tok.trim().into()));
            }
            keys.insert(k);
        }
        if keys.is_empty() {
            return Err(ChordError::Empty);
        }
        Ok(Chord { keys })
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self.keys.iter().map(|k| key_name(*k)).collect();
        write!(f, "{}", names.join("+"))
    }
}

impl Serialize for Chord {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Chord, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The configurable shortcut set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bindings {
    /// Hold to talk.
    pub ptt: Chord,
    /// Press to toggle hands-free (locked) dictation.
    pub hands_free: Chord,
    /// Cancel the current dictation (only swallowed while dictating).
    pub cancel: Chord,
    /// Re-insert the last transcript (Wispr Windows default Shift+Alt+Z).
    pub paste_last: Chord,
    /// Put the last transcript on the clipboard (Wispr default Shift+Alt+X).
    pub copy_last: Chord,
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings {
            ptt: Chord::new([Key::Ctrl, Key::Win]),
            hands_free: Chord::new([Key::Ctrl, Key::Win, Key::Space]),
            cancel: Chord::new([Key::Esc]),
            paste_last: Chord::new([Key::Shift, Key::Alt, Key::Vk(b'Z' as u16)]),
            copy_last: Chord::new([Key::Shift, Key::Alt, Key::Vk(b'X' as u16)]),
        }
    }
}

/// What the chord matcher emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    /// PTT chord became fully held.
    PttDown,
    /// PTT chord released (after a `PttDown`).
    PttUp,
    /// PTT was held but a foreign key was pressed (e.g. Ctrl+Win+Left = switch
    /// desktop): the user meant an OS shortcut, not dictation.
    PttAborted,
    /// Hands-free chord pressed (toggle lock).
    HandsFreeToggle,
    /// Cancel key pressed.
    Cancel,
    /// Paste-last-transcript chord pressed.
    PasteLast,
    /// Copy-last-transcript chord pressed.
    CopyLast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub down: bool,
}

impl KeyEvent {
    pub fn down(key: Key) -> Self {
        KeyEvent { key, down: true }
    }
    pub fn up(key: Key) -> Self {
        KeyEvent { key, down: false }
    }
}

/// Matcher verdict for one key event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    pub actions: Vec<HotkeyAction>,
    /// Return 1 from the hook (don't pass the event on).
    pub suppress: bool,
    /// Swallow this Win/Alt key-up and inject a masked release instead
    /// (`input::masked_release`: `[LCtrl↓, key↑, LCtrl↑]`). Implies `suppress`.
    pub mask_release: bool,
}

/// Stateful chord matcher. Feed physical (non-injected) events only.
#[derive(Debug, Clone)]
pub struct ChordMatcher {
    bindings: Bindings,
    pressed: BTreeSet<Key>,
    ptt_active: bool,
    /// Keys whose key-down we swallowed: their key-up must be swallowed too.
    swallowed: BTreeSet<Key>,
    /// Mask-needing keys (Win/Alt) held while some chord fired.
    needs_mask: BTreeSet<Key>,
    /// The fake Left Ctrl of an AltGr press is logically down.
    altgr_ctrl: bool,
}

impl ChordMatcher {
    pub fn new(bindings: Bindings) -> Self {
        ChordMatcher {
            bindings,
            pressed: BTreeSet::new(),
            ptt_active: false,
            swallowed: BTreeSet::new(),
            needs_mask: BTreeSet::new(),
            altgr_ctrl: false,
        }
    }

    /// Process one hook event as Windows reports it. The fake Left Ctrl of
    /// an AltGr press ([`is_altgr_fake_ctrl`]) is not a key the user
    /// pressed: it never takes part in a chord and always passes through
    /// untouched (so its key-up releases it and no Ctrl is left stuck).
    pub fn handle_raw(&mut self, vk: u16, scan: u32, down: bool, dictating: bool) -> Decision {
        if is_altgr_fake_ctrl(vk, scan) {
            self.altgr_ctrl = down;
            return Decision::default();
        }
        self.handle(
            KeyEvent {
                key: Key::from_vk(vk),
                down,
            },
            dictating,
        )
    }

    /// The AltGr fake Left Ctrl is down (we saw its key-down, not its key-up).
    pub fn altgr_ctrl_down(&self) -> bool {
        self.altgr_ctrl
    }

    /// The OS says Left Ctrl is up, so a fake-Ctrl key-up we never saw
    /// can't matter any more.
    pub fn forget_altgr_ctrl(&mut self) {
        self.altgr_ctrl = false;
    }

    /// Is the *user's own* Left Ctrl down? `async_down` = GetAsyncKeyState
    /// says VK_LCONTROL is down, which on an AltGr layout is also true for
    /// the fake Ctrl. A masked release must keep a real Left Ctrl down but
    /// may (and should) release the fake one.
    pub fn real_lctrl_held(&self, async_down: bool) -> bool {
        self.pressed.contains(&Key::LCtrl) || (async_down && !self.altgr_ctrl)
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    pub fn set_bindings(&mut self, b: Bindings) {
        self.bindings = b;
        self.reset();
    }

    /// Forget all held keys (e.g. after the hook was re-installed, or the
    /// workstation was locked).
    pub fn reset(&mut self) {
        self.pressed.clear();
        self.ptt_active = false;
        self.swallowed.clear();
        self.needs_mask.clear();
        self.altgr_ctrl = false;
    }

    pub fn pressed(&self) -> impl Iterator<Item = Key> + '_ {
        self.pressed.iter().copied()
    }

    /// Stale-key recovery: the OS says `key` is no longer down although we
    /// never saw its key-up (GetAsyncKeyState check). Treated as a release.
    pub fn force_release(&mut self, key: Key) -> Vec<HotkeyAction> {
        if self.pressed.contains(&key) {
            self.handle(KeyEvent::up(key), false).actions
        } else {
            Vec::new()
        }
    }

    /// Process one physical key event. `dictating` = a dictation is in
    /// progress (listening/processing); only then is the cancel key swallowed.
    pub fn handle(&mut self, ev: KeyEvent, dictating: bool) -> Decision {
        let mut d = Decision::default();
        if ev.down {
            let repeat = !self.pressed.insert(ev.key);
            if repeat {
                d.suppress = self.swallowed.contains(&ev.key);
                return d;
            }
            if self.newly_matches(&self.bindings.hands_free, ev.key) {
                // Hands-free chord supersedes a PTT that started on the way.
                self.ptt_active = false;
                d.actions.push(HotkeyAction::HandsFreeToggle);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.newly_matches(&self.bindings.ptt, ev.key) {
                self.ptt_active = true;
                d.actions.push(HotkeyAction::PttDown);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.newly_matches(&self.bindings.paste_last, ev.key) {
                d.actions.push(HotkeyAction::PasteLast);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.newly_matches(&self.bindings.copy_last, ev.key) {
                d.actions.push(HotkeyAction::CopyLast);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.is_cancel(ev.key) {
                d.actions.push(HotkeyAction::Cancel);
                if dictating {
                    d.suppress = true;
                }
            } else if self.ptt_active
                && !self.bindings.ptt.covers(ev.key)
                && !self.bindings.hands_free.covers(ev.key)
            {
                self.ptt_active = false;
                d.actions.push(HotkeyAction::PttAborted);
            }
            if d.suppress {
                self.swallowed.insert(ev.key);
            }
        } else {
            if !self.pressed.remove(&ev.key) {
                // Key-up for a key we never saw go down (hook installed while
                // held): pass it through untouched.
                return d;
            }
            if self.ptt_active && self.bindings.ptt.covers(ev.key) {
                self.ptt_active = false;
                d.actions.push(HotkeyAction::PttUp);
            }
            d.suppress = self.swallowed.remove(&ev.key);
            if !d.suppress && self.needs_mask.remove(&ev.key) {
                d.mask_release = true;
                d.suppress = true;
            }
        }
        d
    }

    /// A single-key cancel (Esc) fires regardless of other held keys; a
    /// multi-key cancel chord must match exactly.
    fn is_cancel(&self, key: Key) -> bool {
        let c = &self.bindings.cancel;
        if c.keys().len() == 1 {
            c.covers(key)
        } else {
            self.newly_matches(c, key)
        }
    }

    /// `key` just went down (it is in `pressed`) and completes chord `c`:
    /// it matches now and did not match a moment ago. Pressing the other
    /// Alt while an "Alt" chord is held keeps it matching but doesn't
    /// fire it again.
    fn newly_matches(&self, c: &Chord, key: Key) -> bool {
        if !c.matches(&self.pressed) {
            return false;
        }
        let mut before = self.pressed.clone();
        before.remove(&key);
        !c.matches(&before)
    }

    fn chord_fired(&mut self) {
        for k in self.pressed.iter().copied() {
            if k.needs_release_mask() {
                self.needs_mask.insert(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyAction::*;

    fn m() -> ChordMatcher {
        ChordMatcher::new(Bindings::default())
    }

    fn feed(m: &mut ChordMatcher, evs: &[KeyEvent]) -> Vec<HotkeyAction> {
        evs.iter()
            .flat_map(|e| m.handle(*e, false).actions)
            .collect()
    }

    #[test]
    fn copy_last_chord_fires_and_swallows_the_letter() {
        let mut m = m();
        let x = Key::Vk(b'X' as u16);
        m.handle(KeyEvent::down(Key::Shift), false);
        m.handle(KeyEvent::down(Key::Alt), false);
        let d = m.handle(KeyEvent::down(x), false);
        assert_eq!(d.actions, vec![CopyLast]);
        assert!(d.suppress, "X must not reach the app");
        assert!(m.handle(KeyEvent::up(x), false).suppress);
    }

    #[test]
    fn paste_last_chord_fires_and_swallows_the_letter() {
        let mut m = m();
        let z = Key::Vk(b'Z' as u16);
        assert!(m
            .handle(KeyEvent::down(Key::Shift), false)
            .actions
            .is_empty());
        assert!(m.handle(KeyEvent::down(Key::Alt), false).actions.is_empty());
        let d = m.handle(KeyEvent::down(z), false);
        assert_eq!(d.actions, vec![PasteLast]);
        assert!(d.suppress, "Z must not reach the app");
        assert!(m.handle(KeyEvent::up(z), false).suppress);
        let alt_up = m.handle(KeyEvent::up(Key::Alt), false);
        assert!(
            alt_up.mask_release,
            "lone Alt release would focus the menu bar"
        );
    }

    #[test]
    fn vk_codes_keep_the_side_of_ctrl_alt_shift_but_fold_win() {
        assert_eq!(Key::from_vk(0xA2), Key::LCtrl);
        assert_eq!(Key::from_vk(0xA3), Key::RCtrl);
        assert_eq!(Key::from_vk(0xA4), Key::LAlt);
        assert_eq!(Key::from_vk(0xA5), Key::RAlt);
        assert_eq!(Key::from_vk(0xA0), Key::LShift);
        assert_eq!(Key::from_vk(0xA1), Key::RShift);
        assert_eq!(Key::from_vk(0x12), Key::Alt, "sideless VK_MENU");
        assert_eq!(Key::from_vk(0x5B), Key::Win);
        assert_eq!(Key::from_vk(0x5C), Key::Win);
        assert_eq!(Key::from_vk(0x41), Key::Vk(0x41));
    }

    // --- Side-specific keys (Right Alt push-to-talk) ---

    const LCTRL_VK: u16 = 0xA2;
    const LALT_VK: u16 = 0xA4;
    const RALT_VK: u16 = 0xA5;
    const TAB_VK: u16 = 0x09;
    /// Scan code of a real Left Ctrl.
    const LCTRL_SCAN: u32 = 0x1D;
    /// Scan code of the fake Left Ctrl an AltGr layout sends with Right Alt.
    const FAKE_SCAN: u32 = 0x21D;
    const ALT_SCAN: u32 = 0x38;

    fn ralt_ptt() -> ChordMatcher {
        ChordMatcher::new(Bindings {
            ptt: "RAlt".parse().unwrap(),
            ..Bindings::default()
        })
    }

    fn raw(m: &mut ChordMatcher, evs: &[(u16, u32, bool)]) -> Vec<Decision> {
        evs.iter()
            .map(|(vk, scan, down)| m.handle_raw(*vk, *scan, *down, false))
            .collect()
    }

    fn actions(ds: &[Decision]) -> Vec<HotkeyAction> {
        ds.iter().flat_map(|d| d.actions.clone()).collect()
    }

    #[test]
    fn side_key_names_parse_and_display() {
        for s in [
            "RAlt",
            "ralt",
            "Right Alt",
            "right-alt",
            "AltGr",
            "Right_Alt",
        ] {
            assert_eq!(s.parse::<Chord>().unwrap(), Chord::new([Key::RAlt]), "{s}");
        }
        assert_eq!("RAlt".parse::<Chord>().unwrap().to_string(), "Right Alt");
        assert_eq!(
            "lctrl+Left Shift+lalt"
                .parse::<Chord>()
                .unwrap()
                .to_string(),
            "Left Ctrl+Left Alt+Left Shift"
        );
        assert_eq!(
            "Right Ctrl+Right Shift+X"
                .parse::<Chord>()
                .unwrap()
                .to_string(),
            "Right Ctrl+Right Shift+X"
        );
        for s in ["Left Alt", "Right Alt", "Left Ctrl+Right Shift"] {
            let c: Chord = s.parse().unwrap();
            assert_eq!(c.to_string().parse::<Chord>().unwrap(), c, "{s}");
        }
    }

    #[test]
    fn side_key_alone_is_a_valid_shortcut() {
        assert!(Chord::parse_validated("Right Alt", false).is_ok());
        assert!(Chord::parse_validated("LCtrl+Space", false).is_ok());
    }

    #[test]
    fn a_side_key_next_to_its_either_side_key_is_a_duplicate() {
        assert!(matches!(
            "Alt+RAlt".parse::<Chord>(),
            Err(ChordError::Duplicate(_))
        ));
        assert!(matches!(
            "LCtrl+Ctrl".parse::<Chord>(),
            Err(ChordError::Duplicate(_))
        ));
        assert!("LAlt+RAlt".parse::<Chord>().is_ok(), "both sides together");
    }

    #[test]
    fn right_alt_alone_fires_ptt_and_its_release_is_masked() {
        let mut m = ralt_ptt();
        let d = m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert_eq!(d.actions, vec![PttDown]);
        assert!(!d.suppress, "modifier key-down passes through");
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert_eq!(up.actions, vec![PttUp]);
        assert!(
            up.mask_release && up.suppress,
            "lone Right Alt release would focus the menu bar"
        );
        assert_eq!(m.pressed().count(), 0);
    }

    #[test]
    fn left_alt_does_not_fire_right_alt_ptt() {
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[(LALT_VK, ALT_SCAN, true), (LALT_VK, ALT_SCAN, false)],
        );
        assert!(actions(&ds).is_empty());
        assert!(!ds[1].mask_release, "nothing fired: plain Alt release");
    }

    #[test]
    fn alt_tab_does_not_fire_right_alt_ptt() {
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (LALT_VK, ALT_SCAN, true),
                (TAB_VK, 0x0F, true),
                (TAB_VK, 0x0F, false),
                (TAB_VK, 0x0F, true),
                (TAB_VK, 0x0F, false),
                (LALT_VK, ALT_SCAN, false),
            ],
        );
        assert!(actions(&ds).is_empty());
        assert!(ds.iter().all(|d| !d.suppress), "Alt+Tab untouched");
    }

    #[test]
    fn altgr_fake_ctrl_is_ignored_and_ptt_fires_and_releases() {
        // AltGr layout; here the fake Ctrl goes up before Right Alt.
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (LCTRL_VK, FAKE_SCAN, true), // auto-repeat
                (RALT_VK, ALT_SCAN, true),
                (LCTRL_VK, FAKE_SCAN, false),
                (RALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, PttUp]);
        for i in [0, 2, 4] {
            assert_eq!(
                ds[i],
                Decision::default(),
                "fake Ctrl #{i} passes untouched"
            );
        }
        assert!(ds[5].mask_release);
        assert_eq!(m.pressed().count(), 0, "nothing left held");
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn altgr_release_with_fake_ctrl_up_last_leaves_nothing_stuck() {
        let mut m = ralt_ptt();
        raw(
            &mut m,
            &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        assert!(m.altgr_ctrl_down());
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert_eq!(up.actions, vec![PttUp]);
        assert!(up.mask_release);
        // The fake Ctrl is logically down but is not the user's: the masked
        // release must send its trailing LCtrl up.
        assert!(!m.real_lctrl_held(true));
        let fake_up = m.handle_raw(LCTRL_VK, FAKE_SCAN, false, false);
        assert_eq!(fake_up, Decision::default(), "fake Ctrl up passes through");
        assert!(!m.altgr_ctrl_down());
        assert_eq!(m.pressed().count(), 0);
    }

    #[test]
    fn real_left_ctrl_still_counts_as_held_next_to_altgr() {
        let mut m = ralt_ptt();
        raw(
            &mut m,
            &[
                (LCTRL_VK, LCTRL_SCAN, true),
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
            ],
        );
        assert!(m.real_lctrl_held(true));
        let mut m = ralt_ptt();
        assert!(m.real_lctrl_held(true), "held before the hook saw it");
        m.handle_raw(LCTRL_VK, FAKE_SCAN, true, false);
        assert!(!m.real_lctrl_held(true), "only the fake one is down");
        assert!(!m.real_lctrl_held(false));
    }

    #[test]
    fn altgr_does_not_fire_a_ctrl_alt_chord() {
        // "Ctrl+Alt" used to fire on every AltGr press (fake Ctrl + Right Alt).
        let mut m = ChordMatcher::new(Bindings {
            ptt: "Ctrl+Alt".parse().unwrap(),
            ..Bindings::default()
        });
        let ds = raw(
            &mut m,
            &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        assert!(actions(&ds).is_empty());
        // A real Ctrl with Right Alt still matches "Ctrl+Alt".
        let d = m.handle_raw(LCTRL_VK, LCTRL_SCAN, true, false);
        assert_eq!(d.actions, vec![PttDown]);
    }

    #[test]
    fn altgr_character_aborts_right_alt_ptt_and_reaches_the_app() {
        let mut m = ralt_ptt();
        let e = 0x45;
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (e, 0x12, true),
                (e, 0x12, false),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
        assert!(!ds[2].suppress && !ds[3].suppress);
    }

    #[test]
    fn right_alt_tab_aborts_ptt() {
        let mut m = ralt_ptt();
        let ds = raw(&mut m, &[(RALT_VK, ALT_SCAN, true), (TAB_VK, 0x0F, true)]);
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
    }

    #[test]
    fn either_side_alt_matches_both_sides_and_does_not_refire() {
        let b = Bindings {
            ptt: "Alt".parse().unwrap(),
            ..Bindings::default()
        };
        for vk in [LALT_VK, RALT_VK] {
            let mut m = ChordMatcher::new(b.clone());
            assert_eq!(
                m.handle_raw(vk, ALT_SCAN, true, false).actions,
                vec![PttDown]
            );
            assert_eq!(
                m.handle_raw(vk, ALT_SCAN, false, false).actions,
                vec![PttUp]
            );
        }
        let mut m = ChordMatcher::new(b);
        let ds = raw(
            &mut m,
            &[
                (LALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (LALT_VK, ALT_SCAN, false),
                (RALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(
            actions(&ds),
            vec![PttDown, PttUp],
            "second side doesn't refire"
        );
    }

    #[test]
    fn default_ctrl_win_still_matches_either_ctrl() {
        for ctrl in [0xA2u16, 0xA3] {
            let mut m = m();
            let ds = raw(
                &mut m,
                &[
                    (ctrl, 0x1D, true),
                    (0x5B, 0x5B, true),
                    (ctrl, 0x1D, false),
                    (0x5B, 0x5B, false),
                ],
            );
            assert_eq!(actions(&ds), vec![PttDown, PttUp]);
            assert!(ds[3].mask_release, "Win release still masked");
        }
    }

    #[test]
    fn side_specific_chord_ignores_the_other_side() {
        let mut m = ChordMatcher::new(Bindings {
            ptt: "RCtrl+Win".parse().unwrap(),
            ..Bindings::default()
        });
        assert!(actions(&raw(&mut m, &[(0xA2, 0x1D, true), (0x5B, 0x5B, true)])).is_empty());
        m.reset();
        assert_eq!(
            actions(&raw(&mut m, &[(0xA3, 0x1D, true), (0x5B, 0x5B, true)])),
            vec![PttDown]
        );
    }

    #[test]
    fn force_release_of_a_side_key_ends_ptt() {
        let mut m = ralt_ptt();
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert_eq!(m.force_release(Key::RAlt), vec![PttUp]);
    }

    #[test]
    fn a_missed_fake_ctrl_up_can_be_forgotten() {
        let mut m = ralt_ptt();
        m.handle_raw(LCTRL_VK, FAKE_SCAN, true, false);
        m.forget_altgr_ctrl();
        assert!(!m.altgr_ctrl_down());
        m.handle_raw(LCTRL_VK, FAKE_SCAN, true, false);
        m.reset();
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn ptt_fires_on_full_chord_and_releases_on_first_key_up() {
        let mut m = m();
        assert!(m
            .handle(KeyEvent::down(Key::Ctrl), false)
            .actions
            .is_empty());
        assert_eq!(
            m.handle(KeyEvent::down(Key::Win), false).actions,
            vec![PttDown]
        );
        assert_eq!(
            m.handle(KeyEvent::up(Key::Ctrl), false).actions,
            vec![PttUp]
        );
        assert!(m.handle(KeyEvent::up(Key::Win), false).actions.is_empty());
    }

    #[test]
    fn ptt_order_of_modifiers_does_not_matter() {
        let mut m = m();
        let a = feed(
            &mut m,
            &[KeyEvent::down(Key::Win), KeyEvent::down(Key::Ctrl)],
        );
        assert_eq!(a, vec![PttDown]);
    }

    #[test]
    fn autorepeat_does_not_refire() {
        let mut m = m();
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::Ctrl),
                KeyEvent::down(Key::Win),
                KeyEvent::down(Key::Win),
                KeyEvent::down(Key::Ctrl),
            ],
        );
        assert_eq!(a, vec![PttDown]);
    }

    #[test]
    fn hands_free_supersedes_ptt_and_release_emits_nothing() {
        let mut m = m();
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::Ctrl),
                KeyEvent::down(Key::Win),
                KeyEvent::down(Key::Space),
                KeyEvent::up(Key::Space),
                KeyEvent::up(Key::Win),
                KeyEvent::up(Key::Ctrl),
            ],
        );
        assert_eq!(a, vec![PttDown, HandsFreeToggle]);
    }

    #[test]
    fn hands_free_space_is_swallowed_down_and_up() {
        let mut m = m();
        m.handle(KeyEvent::down(Key::Ctrl), false);
        m.handle(KeyEvent::down(Key::Win), false);
        assert!(m.handle(KeyEvent::down(Key::Space), false).suppress);
        assert!(
            m.handle(KeyEvent::down(Key::Space), false).suppress,
            "repeat"
        );
        assert!(m.handle(KeyEvent::up(Key::Space), false).suppress);
    }

    #[test]
    fn plain_space_is_not_swallowed() {
        let mut m = m();
        let d = m.handle(KeyEvent::down(Key::Space), true);
        assert!(!d.suppress);
        assert!(d.actions.is_empty());
    }

    #[test]
    fn modifier_events_pass_through_except_masked_release() {
        let mut m = m();
        assert!(!m.handle(KeyEvent::down(Key::Ctrl), false).suppress);
        assert!(!m.handle(KeyEvent::down(Key::Win), false).suppress);
        assert!(!m.handle(KeyEvent::up(Key::Ctrl), false).suppress);
        let d = m.handle(KeyEvent::up(Key::Win), false);
        assert!(d.suppress && d.mask_release);
    }

    #[test]
    fn win_release_after_chord_is_masked() {
        let mut m = m();
        m.handle(KeyEvent::down(Key::Ctrl), false);
        m.handle(KeyEvent::down(Key::Win), false);
        assert!(
            !m.handle(KeyEvent::up(Key::Ctrl), false).mask_release,
            "ctrl needs no mask"
        );
        let d = m.handle(KeyEvent::up(Key::Win), false);
        assert!(d.mask_release);
        assert!(d.suppress, "real key-up replaced by the masked one");
    }

    #[test]
    fn win_release_without_chord_is_not_masked() {
        let mut m = m();
        m.handle(KeyEvent::down(Key::Win), false);
        assert!(!m.handle(KeyEvent::up(Key::Win), false).mask_release);
    }

    #[test]
    fn alt_in_custom_chord_is_masked() {
        let b = Bindings {
            ptt: "Ctrl+Alt".parse().unwrap(),
            ..Bindings::default()
        };
        let mut m = ChordMatcher::new(b);
        m.handle(KeyEvent::down(Key::Alt), false);
        m.handle(KeyEvent::down(Key::Ctrl), false);
        assert!(m.handle(KeyEvent::up(Key::Alt), false).mask_release);
    }

    #[test]
    fn esc_cancels_and_is_swallowed_only_while_dictating() {
        let mut m = m();
        let d = m.handle(KeyEvent::down(Key::Esc), false);
        assert_eq!(d.actions, vec![Cancel]);
        assert!(!d.suppress);
        assert!(!m.handle(KeyEvent::up(Key::Esc), false).suppress);
        let d = m.handle(KeyEvent::down(Key::Esc), true);
        assert_eq!(d.actions, vec![Cancel]);
        assert!(d.suppress);
        assert!(m.handle(KeyEvent::up(Key::Esc), true).suppress);
    }

    #[test]
    fn foreign_key_during_ptt_aborts() {
        let mut m = m();
        let left = Key::Vk(0x25);
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::Ctrl),
                KeyEvent::down(Key::Win),
                KeyEvent::down(left),
                KeyEvent::up(left),
                KeyEvent::up(Key::Win),
                KeyEvent::up(Key::Ctrl),
            ],
        );
        assert_eq!(a, vec![PttDown, PttAborted]);
    }

    #[test]
    fn superset_chord_does_not_fire_ptt() {
        let mut m = m();
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::Shift),
                KeyEvent::down(Key::Ctrl),
                KeyEvent::down(Key::Win),
            ],
        );
        assert!(a.is_empty());
    }

    #[test]
    fn unseen_key_up_passes_through() {
        let mut m = m();
        let d = m.handle(KeyEvent::up(Key::Win), false);
        assert_eq!(d, Decision::default());
    }

    #[test]
    fn force_release_ends_stuck_ptt() {
        let mut m = m();
        feed(
            &mut m,
            &[KeyEvent::down(Key::Ctrl), KeyEvent::down(Key::Win)],
        );
        assert_eq!(m.force_release(Key::Win), vec![PttUp]);
        assert_eq!(m.pressed().count(), 1);
        assert!(m.force_release(Key::Space).is_empty());
    }

    #[test]
    fn chord_parse_and_display_roundtrip() {
        let c: Chord = "ctrl + win + space".parse().unwrap();
        assert_eq!(c, Chord::new([Key::Ctrl, Key::Win, Key::Space]));
        assert_eq!(c.to_string(), "Ctrl+Win+Space");
        let f: Chord = "Shift+Alt+Z".parse().unwrap();
        assert_eq!(f.to_string(), "Alt+Shift+Z");
        assert_eq!("Ctrl+F9".parse::<Chord>().unwrap().to_string(), "Ctrl+F9");
    }

    #[test]
    fn chord_validation_rules() {
        assert_eq!("".parse::<Chord>(), Err(ChordError::Empty));
        assert!(matches!(
            "Ctrl+Bogus".parse::<Chord>(),
            Err(ChordError::UnknownKey(_))
        ));
        assert!(matches!(
            "Ctrl+ctrl".parse::<Chord>(),
            Err(ChordError::Duplicate(_))
        ));
        assert_eq!(
            Chord::parse_validated("Ctrl+Win+Alt+Shift", false),
            Err(ChordError::TooManyKeys(4))
        );
        assert_eq!(
            Chord::parse_validated("A", false),
            Err(ChordError::NoModifier)
        );
        assert!(Chord::parse_validated("Esc", true).is_ok());
        assert_eq!(
            Chord::parse_validated("A+B", true),
            Err(ChordError::NoModifier)
        );
    }

    #[test]
    fn bindings_serde_as_strings() {
        let j = serde_json::to_string(&Bindings::default()).unwrap();
        assert_eq!(
            j,
            r#"{"ptt":"Ctrl+Win","hands_free":"Ctrl+Win+Space","cancel":"Esc","paste_last":"Alt+Shift+Z","copy_last":"Alt+Shift+X"}"#
        );
        let b: Bindings = serde_json::from_str(r#"{"ptt":"Ctrl+Shift"}"#).unwrap();
        assert_eq!(b.ptt.to_string(), "Ctrl+Shift");
        assert_eq!(b.cancel, Bindings::default().cancel);
    }

    #[test]
    fn set_bindings_resets_state() {
        let mut m = m();
        feed(
            &mut m,
            &[KeyEvent::down(Key::Ctrl), KeyEvent::down(Key::Win)],
        );
        m.set_bindings(Bindings::default());
        assert_eq!(m.pressed().count(), 0);
        assert!(m.handle(KeyEvent::up(Key::Ctrl), false).actions.is_empty());
    }
}
