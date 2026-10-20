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

/// Normalised key (left/right variants collapse).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Ctrl,
    Win,
    Alt,
    Shift,
    Space,
    Esc,
    /// Any other virtual-key code.
    Vk(u16),
}

impl Key {
    /// Map a Win32 virtual-key code to a normalised key.
    pub fn from_vk(vk: u16) -> Key {
        match vk {
            0x11 | 0xA2 | 0xA3 => Key::Ctrl,
            0x5B | 0x5C => Key::Win,
            0x12 | 0xA4 | 0xA5 => Key::Alt,
            0x10 | 0xA0 | 0xA1 => Key::Shift,
            0x20 => Key::Space,
            0x1B => Key::Esc,
            other => Key::Vk(other),
        }
    }

    pub fn is_modifier(self) -> bool {
        matches!(self, Key::Ctrl | Key::Win | Key::Alt | Key::Shift)
    }

    /// Keys whose lone release triggers a shell action (Start menu / menu bar).
    pub fn needs_release_mask(self) -> bool {
        matches!(self, Key::Win | Key::Alt)
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
        Key::Win => "Win".into(),
        Key::Alt => "Alt".into(),
        Key::Shift => "Shift".into(),
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
            if !keys.insert(k) {
                return Err(ChordError::Duplicate(tok.trim().into()));
            }
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
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings {
            ptt: Chord::new([Key::Ctrl, Key::Win]),
            hands_free: Chord::new([Key::Ctrl, Key::Win, Key::Space]),
            cancel: Chord::new([Key::Esc]),
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
}

impl ChordMatcher {
    pub fn new(bindings: Bindings) -> Self {
        ChordMatcher {
            bindings,
            pressed: BTreeSet::new(),
            ptt_active: false,
            swallowed: BTreeSet::new(),
            needs_mask: BTreeSet::new(),
        }
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
            if self.pressed == *self.bindings.hands_free.keys() {
                // Hands-free chord supersedes a PTT that started on the way.
                self.ptt_active = false;
                d.actions.push(HotkeyAction::HandsFreeToggle);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.pressed == *self.bindings.ptt.keys() {
                self.ptt_active = true;
                d.actions.push(HotkeyAction::PttDown);
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
                && !self.bindings.ptt.contains(ev.key)
                && !self.bindings.hands_free.contains(ev.key)
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
            if self.ptt_active && self.bindings.ptt.contains(ev.key) {
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
            c.contains(key)
        } else {
            self.pressed == *c.keys()
        }
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
    fn vk_codes_normalise_left_and_right_variants() {
        assert_eq!(Key::from_vk(0xA2), Key::Ctrl);
        assert_eq!(Key::from_vk(0xA3), Key::Ctrl);
        assert_eq!(Key::from_vk(0x5B), Key::Win);
        assert_eq!(Key::from_vk(0x5C), Key::Win);
        assert_eq!(Key::from_vk(0xA5), Key::Alt);
        assert_eq!(Key::from_vk(0x41), Key::Vk(0x41));
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
            r#"{"ptt":"Ctrl+Win","hands_free":"Ctrl+Win+Space","cancel":"Esc"}"#
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
