//! Platform-independent chord matcher.
//!
//! The LL keyboard hook feeds every physical key event in; the matcher answers
//! (a) which hotkey actions fired, (b) whether to swallow the event, and
//! (c) whether a dummy VK 0xFF must be injected *before* letting a Win/Alt
//! key-up through (so a "lone" Win release doesn't open Start, and a lone Alt
//! release doesn't focus the menu bar — windows-research 02 §2).
//!
//! While push-to-talk is Right Alt alone (the default, D57), every Right
//! Alt event and AltGr's fake Left Ctrl are swallowed: the app sees no Alt
//! (no menu bar), no Ctrl and no AltGr character. AltGr characters are
//! typed with Left Ctrl+Alt, which is untouched.

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
    /// Hold to talk (default Right Alt, D57; Wispr's is Ctrl+Win).
    pub ptt: Chord,
    /// Press to toggle hands-free (locked) dictation.
    pub hands_free: Chord,
    /// Cancel the current dictation (only swallowed while dictating).
    pub cancel: Chord,
    /// Re-insert the last transcript (Wispr Windows default Shift+Alt+Z).
    pub paste_last: Chord,
    /// Put the last transcript on the clipboard (Wispr default Shift+Alt+X).
    pub copy_last: Chord,
    /// Hold to speak a Command Mode instruction for the selected text
    /// (Wispr default Ctrl+Win+Alt). It may extend a push-to-talk on the
    /// way: Right Alt held, then Ctrl+Win (or a Ctrl+Win push-to-talk, then
    /// Alt) turns that dictation into a command.
    pub command: Chord,
    /// Open the Scratchpad window. Unset by default; `""` in prefs = unset.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_chord"
    )]
    pub scratchpad: Option<Chord>,
    /// New note + hands-free dictation into it. Unset by default.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_chord"
    )]
    pub new_note: Option<Chord>,
    /// Start / stop recording a meeting (Notetaker, D45). Unset by default.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_chord"
    )]
    pub meeting: Option<Chord>,
    /// Transforms (docs/proposals/transforms.md): one entry per saved
    /// transform, in its order (`None` = no shortcut). Not saved here (the
    /// transforms own them, `Prefs::hotkey_bindings`); empty while
    /// Transforms is off, so no chord is registered.
    #[serde(skip)]
    pub transforms: Vec<Option<Chord>>,
    /// Transforms' "view changes" shortcut (`None` = off / Transforms off).
    #[serde(skip)]
    pub view_changes: Option<Chord>,
}

/// An optional shortcut: missing, `null` or blank = none.
pub(crate) fn optional_chord<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Chord>, D::Error> {
    match Option::<String>::deserialize(d)? {
        Some(s) if !s.trim().is_empty() => s.parse().map(Some).map_err(serde::de::Error::custom),
        _ => Ok(None),
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings {
            ptt: Chord::new([Key::RAlt]),
            hands_free: Chord::new([Key::Ctrl, Key::Win, Key::Space]),
            cancel: Chord::new([Key::Esc]),
            paste_last: Chord::new([Key::Shift, Key::Alt, Key::Vk(b'Z' as u16)]),
            copy_last: Chord::new([Key::Shift, Key::Alt, Key::Vk(b'X' as u16)]),
            scratchpad: None,
            new_note: None,
            meeting: None,
            command: Chord::new([Key::Ctrl, Key::Win, Key::Alt]),
            transforms: Vec::new(),
            view_changes: None,
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
    /// PTT (or the Command chord) was held but a foreign key was pressed
    /// (e.g. Ctrl+Win+Left = switch desktop): the user meant an OS
    /// shortcut, not dictation.
    PttAborted,
    /// Hands-free chord pressed (toggle lock).
    HandsFreeToggle,
    /// Cancel key pressed.
    Cancel,
    /// Paste-last-transcript chord pressed.
    PasteLast,
    /// Copy-last-transcript chord pressed.
    CopyLast,
    /// Open-the-Scratchpad chord pressed.
    Scratchpad,
    /// New-note chord pressed (create a note, open it, dictate into it).
    NewNote,
    /// Meeting chord pressed (start or stop a meeting recording).
    Meeting,
    /// Command chord became fully held (supersedes a push-to-talk that
    /// started on the way).
    CommandDown,
    /// Command chord released (after a `CommandDown`).
    CommandUp,
    /// The shortcut of the transform at this index was pressed.
    Transform(u8),
    /// Transforms' "view changes" shortcut was pressed.
    ViewChanges,
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
    /// The Command chord fired and none of its keys was released yet.
    command_active: bool,
    /// Keys whose key-down we swallowed: their key-up must be swallowed
    /// too, so it outlives [`reset`](Self::reset) (the app never saw the
    /// key go down). Only [`forget`](Self::forget) clears it: after missed
    /// events a key-up that passes is harmless, a swallowed one whose
    /// key-down passed would leave the key stuck.
    swallowed: BTreeSet<Key>,
    /// Mask-needing keys (Win/Alt) held while some chord fired.
    needs_mask: BTreeSet<Key>,
    /// The fake Left Ctrl of an AltGr press is logically down.
    altgr_ctrl: bool,
    /// The current fake Left Ctrl's key-down was swallowed (`Some(true)`)
    /// or passed (`Some(false)`); `None` = none seen. Its key-up follows
    /// its key-down, whatever the bindings are by then. Kept like
    /// `swallowed`.
    altgr_ctrl_swallowed: Option<bool>,
}

impl ChordMatcher {
    pub fn new(bindings: Bindings) -> Self {
        ChordMatcher {
            bindings,
            pressed: BTreeSet::new(),
            ptt_active: false,
            command_active: false,
            swallowed: BTreeSet::new(),
            needs_mask: BTreeSet::new(),
            altgr_ctrl: false,
            altgr_ctrl_swallowed: None,
        }
    }

    /// Process one hook event as Windows reports it. The fake Left Ctrl of
    /// an AltGr press ([`is_altgr_fake_ctrl`]) is not a key the user
    /// pressed: it never takes part in a chord. It passes through untouched
    /// (so its key-up releases it and no Ctrl is left stuck), except while
    /// Right Alt is push-to-talk: then its key-down is swallowed, and so is
    /// the key-up of a swallowed key-down (only that one).
    pub fn handle_raw(&mut self, vk: u16, scan: u32, down: bool, dictating: bool) -> Decision {
        if is_altgr_fake_ctrl(vk, scan) {
            self.altgr_ctrl = down;
            let suppress = if down {
                // Auto-repeat keeps the first key-down's fate.
                *self
                    .altgr_ctrl_swallowed
                    .get_or_insert(self.swallows_right_alt())
            } else {
                self.altgr_ctrl_swallowed.take().unwrap_or(false)
            };
            return Decision {
                suppress,
                ..Decision::default()
            };
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

    /// Push-to-talk is Right Alt alone: Right Alt (and AltGr's fake Left
    /// Ctrl) never reach the app (D57).
    fn swallows_right_alt(&self) -> bool {
        self.bindings.ptt == Chord::new([Key::RAlt])
    }

    /// New shortcuts. A push-to-talk or Command held right now is aborted
    /// (returned for the caller to send), so the dictation doesn't keep
    /// listening for a key-up that no longer belongs to a shortcut. The
    /// same shortcuts again (any other setting was saved) change nothing.
    pub fn set_bindings(&mut self, b: Bindings) -> Vec<HotkeyAction> {
        if b == self.bindings {
            return Vec::new();
        }
        let held = self.ptt_active || self.command_active;
        self.bindings = b;
        self.reset();
        if held {
            vec![HotkeyAction::PttAborted]
        } else {
            Vec::new()
        }
    }

    /// Forget all held keys (new bindings, a shortcut recording starts).
    /// Which key-downs were swallowed is kept: their key-ups still are.
    pub fn reset(&mut self) {
        self.pressed.clear();
        self.ptt_active = false;
        self.command_active = false;
        self.needs_mask.clear();
        self.altgr_ctrl = false;
    }

    /// [`reset`](Self::reset) after events may have been missed (the hook
    /// was re-installed): forget swallowed key-downs too, so every later
    /// key-up passes and no key can be left stuck.
    pub fn forget(&mut self) {
        self.reset();
        self.swallowed.clear();
        self.altgr_ctrl_swallowed = None;
    }

    /// An event the shortcut recorder handled instead of the matcher
    /// (`suppressed` = the recorder swallowed it): keeps the swallowed
    /// key-down bookkeeping right for key-ups that arrive after recording.
    pub fn note_recorded(&mut self, vk: u16, scan: u32, down: bool, suppressed: bool) {
        if is_altgr_fake_ctrl(vk, scan) {
            self.altgr_ctrl_swallowed = down.then_some(suppressed);
            return;
        }
        let key = Key::from_vk(vk);
        if down && suppressed {
            self.swallowed.insert(key);
        } else {
            self.swallowed.remove(&key);
        }
    }

    /// Held keys the OS says are up although we never saw their key-up
    /// (stale-key recovery; `physically_down` asks GetAsyncKeyState). A
    /// key we swallowed never shows there, and silence proves nothing (a
    /// held key stops auto-repeating once another key is pressed), so it
    /// is released only by its key-up, a [`forget`](Self::forget) or the
    /// dictation's own time limit.
    pub fn stale_keys(&self, physically_down: impl Fn(Key) -> bool) -> Vec<Key> {
        self.pressed
            .iter()
            .copied()
            .filter(|k| !self.swallowed.contains(k) && !physically_down(*k))
            .collect()
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
            } else if self.newly_matches(&self.bindings.command, ev.key) {
                // Command supersedes a PTT that started on the way
                // (Ctrl+Win, then Alt).
                self.ptt_active = false;
                self.command_active = true;
                d.actions.push(HotkeyAction::CommandDown);
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
            } else if self
                .bindings
                .scratchpad
                .as_ref()
                .is_some_and(|c| self.newly_matches(c, ev.key))
            {
                d.actions.push(HotkeyAction::Scratchpad);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self
                .bindings
                .new_note
                .as_ref()
                .is_some_and(|c| self.newly_matches(c, ev.key))
            {
                d.actions.push(HotkeyAction::NewNote);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self
                .bindings
                .meeting
                .as_ref()
                .is_some_and(|c| self.newly_matches(c, ev.key))
            {
                d.actions.push(HotkeyAction::Meeting);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if let Some(i) = self.transform_fired(ev.key) {
                d.actions.push(HotkeyAction::Transform(i));
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self
                .bindings
                .view_changes
                .as_ref()
                .is_some_and(|c| self.newly_matches(c, ev.key))
            {
                d.actions.push(HotkeyAction::ViewChanges);
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
                && !self.bindings.command.covers(ev.key)
            {
                self.ptt_active = false;
                d.actions.push(HotkeyAction::PttAborted);
            } else if self.command_active && !self.bindings.command.covers(ev.key) {
                self.command_active = false;
                d.actions.push(HotkeyAction::PttAborted);
            }
            if ev.key == Key::RAlt && self.swallows_right_alt() {
                d.suppress = true;
            }
            if d.suppress {
                self.swallowed.insert(ev.key);
            } else {
                self.swallowed.remove(&ev.key);
            }
        } else {
            if !self.pressed.remove(&ev.key) {
                // Key-up for a key we hold no state for (hook installed or
                // matcher reset while held): passes, unless we swallowed
                // its key-down.
                d.suppress = self.swallowed.remove(&ev.key);
                return d;
            }
            if self.ptt_active && self.bindings.ptt.covers(ev.key) {
                self.ptt_active = false;
                d.actions.push(HotkeyAction::PttUp);
            }
            if self.command_active && self.bindings.command.covers(ev.key) {
                self.command_active = false;
                d.actions.push(HotkeyAction::CommandUp);
            }
            // A swallowed key-up needs no mask: the app never saw it down.
            let mask = self.needs_mask.remove(&ev.key);
            d.suppress = self.swallowed.remove(&ev.key);
            if !d.suppress && mask {
                d.mask_release = true;
                d.suppress = true;
            }
        }
        d
    }

    /// Index of the transform whose shortcut `key` just completed.
    fn transform_fired(&self, key: Key) -> Option<u8> {
        self.bindings
            .transforms
            .iter()
            .take(usize::from(u8::MAX) + 1)
            .position(|c| c.as_ref().is_some_and(|c| self.newly_matches(c, key)))
            .map(|i| i as u8)
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

    /// Wispr's push-to-talk, Ctrl+Win (the default before D57), with the
    /// other defaults: most chord tests use a multi-key push-to-talk.
    fn ctrl_win_ptt() -> ChordMatcher {
        ChordMatcher::new(Bindings {
            ptt: "Ctrl+Win".parse().unwrap(),
            ..Bindings::default()
        })
    }

    fn feed(m: &mut ChordMatcher, evs: &[KeyEvent]) -> Vec<HotkeyAction> {
        evs.iter()
            .flat_map(|e| m.handle(*e, false).actions)
            .collect()
    }

    #[test]
    fn copy_last_chord_fires_and_swallows_the_letter() {
        let mut m = ctrl_win_ptt();
        let x = Key::Vk(b'X' as u16);
        m.handle(KeyEvent::down(Key::Shift), false);
        m.handle(KeyEvent::down(Key::Alt), false);
        let d = m.handle(KeyEvent::down(x), false);
        assert_eq!(d.actions, vec![CopyLast]);
        assert!(d.suppress, "X must not reach the app");
        assert!(m.handle(KeyEvent::up(x), false).suppress);
    }

    #[test]
    fn command_chord_extends_a_push_to_talk_on_the_way() {
        let mut m = ctrl_win_ptt();
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::LCtrl),
                KeyEvent::down(Key::Win),
                KeyEvent::down(Key::LAlt),
            ],
        );
        assert_eq!(
            a,
            vec![PttDown, CommandDown],
            "Alt is not a foreign key here"
        );
        let up = m.handle(KeyEvent::up(Key::LAlt), false);
        assert_eq!(up.actions, vec![CommandUp]);
        assert!(up.mask_release, "lone Alt release would focus the menu bar");
        assert!(
            feed(&mut m, &[KeyEvent::up(Key::Win), KeyEvent::up(Key::LCtrl)]).is_empty(),
            "the superseded push-to-talk never reports a release"
        );
    }

    #[test]
    fn command_chord_alone_and_its_foreign_key_abort() {
        let mut m = ctrl_win_ptt();
        let a = feed(
            &mut m,
            &[
                KeyEvent::down(Key::LAlt),
                KeyEvent::down(Key::LCtrl),
                KeyEvent::down(Key::Win),
            ],
        );
        assert_eq!(a, vec![CommandDown]);
        let d = m.handle(KeyEvent::down(Key::Vk(0x25)), false);
        assert_eq!(
            d.actions,
            vec![PttAborted],
            "Ctrl+Win+Alt+Left is an OS shortcut"
        );
        assert!(
            feed(
                &mut m,
                &[KeyEvent::up(Key::Vk(0x25)), KeyEvent::up(Key::Win)]
            )
            .is_empty(),
            "an aborted command reports no release"
        );
    }

    #[test]
    fn paste_last_chord_fires_and_swallows_the_letter() {
        let mut m = ctrl_win_ptt();
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
    fn right_alt_alone_fires_ptt_at_key_down_and_never_reaches_the_app() {
        let mut m = ralt_ptt();
        let d = m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert_eq!(d.actions, vec![PttDown], "at key-down, no wait");
        assert!(d.suppress, "no Alt for the app: no menu bar, no AltGr");
        let rep = m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert!(rep.suppress && rep.actions.is_empty(), "auto-repeat");
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert_eq!(up.actions, vec![PttUp]);
        assert!(
            up.suppress && !up.mask_release,
            "swallowed: nothing to mask"
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
    fn altgr_fake_ctrl_is_swallowed_and_ptt_fires_and_releases() {
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
        for (i, d) in ds.iter().enumerate() {
            assert!(d.suppress && !d.mask_release, "event #{i} swallowed");
        }
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
        assert!(up.suppress && !up.mask_release);
        assert!(!m.real_lctrl_held(true), "the fake Ctrl is not the user's");
        let fake_up = m.handle_raw(LCTRL_VK, FAKE_SCAN, false, false);
        assert!(fake_up.suppress, "its down was swallowed: so is its up");
        assert!(fake_up.actions.is_empty());
        assert!(!m.altgr_ctrl_down());
        assert_eq!(m.pressed().count(), 0);
        // Same bindings, same key-up, but its key-down was never swallowed
        // (the matcher started mid-hold): the key-up must pass, or Left
        // Ctrl stays down system-wide.
        let mut fresh = ralt_ptt();
        let fake_up = fresh.handle_raw(LCTRL_VK, FAKE_SCAN, false, false);
        assert_eq!(
            fake_up,
            Decision::default(),
            "its down passed: so does its up"
        );
    }

    // --- Swallowed key-ups follow their key-downs (D57) ---

    /// Ctrl+Win push-to-talk, AltGr pressed (fake Ctrl + Right Alt pass),
    /// then Right Alt becomes push-to-talk mid-hold.
    #[test]
    fn rebinding_to_right_alt_mid_hold_passes_the_ups_of_passed_downs() {
        for fake_first in [true, false] {
            let mut m = ctrl_win_ptt();
            let ds = raw(
                &mut m,
                &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
            );
            assert!(ds.iter().all(|d| !d.suppress));
            assert!(m.set_bindings(Bindings::default()).is_empty());
            let ups = if fake_first {
                [(LCTRL_VK, FAKE_SCAN, false), (RALT_VK, ALT_SCAN, false)]
            } else {
                [(RALT_VK, ALT_SCAN, false), (LCTRL_VK, FAKE_SCAN, false)]
            };
            for (i, d) in raw(&mut m, &ups).iter().enumerate() {
                assert_eq!(*d, Decision::default(), "fake_first={fake_first} up #{i}");
            }
        }
    }

    #[test]
    fn reset_mid_hold_still_swallows_the_ups_of_swallowed_downs() {
        for fake_first in [true, false] {
            let mut m = ralt_ptt();
            let ds = raw(
                &mut m,
                &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
            );
            assert!(ds.iter().all(|d| d.suppress));
            m.reset();
            let ups = if fake_first {
                [(LCTRL_VK, FAKE_SCAN, false), (RALT_VK, ALT_SCAN, false)]
            } else {
                [(RALT_VK, ALT_SCAN, false), (LCTRL_VK, FAKE_SCAN, false)]
            };
            for (i, d) in raw(&mut m, &ups).iter().enumerate() {
                assert!(
                    d.suppress && d.actions.is_empty(),
                    "fake_first={fake_first} up #{i}"
                );
            }
        }
    }

    #[test]
    fn rebinding_away_from_right_alt_mid_hold_aborts_and_swallows_its_ups() {
        let mut m = ralt_ptt();
        raw(
            &mut m,
            &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        let b = Bindings {
            ptt: "Ctrl+Win".parse().unwrap(),
            ..Bindings::default()
        };
        assert_eq!(
            m.set_bindings(b),
            vec![PttAborted],
            "no dictation left listening"
        );
        let ds = raw(
            &mut m,
            &[(RALT_VK, ALT_SCAN, false), (LCTRL_VK, FAKE_SCAN, false)],
        );
        assert!(
            ds.iter().all(|d| d.suppress),
            "the app never saw them go down"
        );
        // The next AltGr press passes: the new push-to-talk isn't Right Alt.
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, false),
                (LCTRL_VK, FAKE_SCAN, false),
            ],
        );
        assert!(ds.iter().all(|d| *d == Decision::default()));
    }

    #[test]
    fn late_right_alt_up_after_an_abort_is_swallowed() {
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (0x45, 0x12, true),
                (0x45, 0x12, false),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert!(up.suppress && !up.mask_release && up.actions.is_empty());
    }

    /// Recording a shortcut (`start_capture`: force-release + reset, then
    /// the recorder sees the events) mid-way through an AltGr press.
    #[test]
    fn shortcut_recording_hands_back_the_right_key_up_fates() {
        use crate::hotkeys::capture::ChordRecorder;
        // Right Alt recorded: the recorder passes the fake Ctrl down and
        // swallows Right Alt; recording ends at Right Alt up, so the fake
        // Ctrl up (released last) reaches the matcher, which must pass it.
        let mut m = ralt_ptt();
        // An earlier swallowed press whose fake Ctrl up the recorder got.
        m.handle_raw(LCTRL_VK, FAKE_SCAN, true, false);
        m.reset();
        let mut rec = ChordRecorder::new();
        for (vk, scan, down) in [
            (LCTRL_VK, FAKE_SCAN, false),
            (LCTRL_VK, FAKE_SCAN, true),
            (RALT_VK, ALT_SCAN, true),
            (RALT_VK, ALT_SCAN, false),
        ] {
            let r = rec.handle_raw(vk, scan, down);
            m.note_recorded(vk, scan, down, r.suppress);
        }
        let fake_up = m.handle_raw(LCTRL_VK, FAKE_SCAN, false, false);
        assert_eq!(fake_up, Decision::default(), "Left Ctrl must not stick");
        // Recording stopped mid-hold: Right Alt's key-down was swallowed
        // by the recorder, so its key-up is swallowed too.
        let mut m = ralt_ptt();
        let mut rec = ChordRecorder::new();
        let r = rec.handle_raw(RALT_VK, ALT_SCAN, true);
        m.note_recorded(RALT_VK, ALT_SCAN, true, r.suppress);
        assert!(m.handle_raw(RALT_VK, ALT_SCAN, false, false).suppress);
    }

    #[test]
    fn forget_after_missed_events_lets_every_key_up_pass() {
        let mut m = ralt_ptt();
        raw(
            &mut m,
            &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        m.forget();
        let ds = raw(
            &mut m,
            &[(RALT_VK, ALT_SCAN, false), (LCTRL_VK, FAKE_SCAN, false)],
        );
        assert!(ds.iter().all(|d| *d == Decision::default()));
    }

    // --- Stale-key recovery never releases a swallowed key ---

    #[test]
    fn a_right_alt_hold_without_repeats_stays_held() {
        // Zero-repeat / FilterKeys, or the 2 s that used to end a hold: the
        // OS never sees the swallowed Right Alt, and silence proves nothing.
        let mut m = ralt_ptt();
        m.handle_raw(LCTRL_VK, FAKE_SCAN, true, false);
        assert_eq!(
            m.handle_raw(RALT_VK, ALT_SCAN, true, false).actions,
            vec![PttDown]
        );
        for _second in 0..5 {
            assert!(m.stale_keys(|_| false).is_empty());
        }
        assert_eq!(
            m.handle_raw(RALT_VK, ALT_SCAN, false, false).actions,
            vec![PttUp]
        );
    }

    #[test]
    fn right_alt_ctrl_win_command_held_with_only_win_repeating_stays_a_command() {
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, CommandDown]);
        // Seconds pass; only Win auto-repeats. The OS sees Ctrl and Win
        // (they passed), never Right Alt.
        for _second in 0..5 {
            assert!(m.handle_raw(0x5B, 0x5B, true, false).actions.is_empty());
            let stale = m.stale_keys(|k| k != Key::RAlt);
            assert!(stale.is_empty(), "{stale:?}");
        }
        assert_eq!(
            m.handle_raw(0x5B, 0x5B, false, false).actions,
            vec![CommandUp]
        );
    }

    #[test]
    fn a_passed_key_the_os_no_longer_holds_is_stale() {
        let mut m = ctrl_win_ptt();
        raw(&mut m, &[(LCTRL_VK, LCTRL_SCAN, true), (0x5B, 0x5B, true)]);
        assert_eq!(m.stale_keys(|k| k != Key::Win), vec![Key::Win]);
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
    fn right_alt_with_a_letter_aborts_and_the_plain_letter_reaches_the_app() {
        // AltGr layout: Right Alt + E during a hold. The app never saw
        // AltGr, so it gets a plain E, never €; the dictation is aborted.
        let mut m = ralt_ptt();
        let e = 0x45;
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (e, 0x12, true),
                (e, 0x12, false),
                (RALT_VK, ALT_SCAN, false),
                (LCTRL_VK, FAKE_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
        assert!(!ds[2].suppress && !ds[3].suppress, "E passes");
        for i in [0, 1, 4, 5] {
            assert!(ds[i].suppress && !ds[i].mask_release, "event #{i}");
        }
        assert_eq!(m.pressed().count(), 0);
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn right_alt_with_a_letter_fake_ctrl_up_first() {
        // The other release order of AltGr: fake Ctrl up, then Right Alt.
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (0x45, 0x12, true),
                (0x45, 0x12, false),
                (LCTRL_VK, FAKE_SCAN, false),
                (RALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
        for i in [0, 1, 4, 5] {
            assert!(ds[i].suppress && !ds[i].mask_release, "event #{i}");
        }
        assert_eq!(m.pressed().count(), 0);
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn right_alt_tab_aborts_ptt_and_tab_passes() {
        let mut m = ralt_ptt();
        let ds = raw(&mut m, &[(RALT_VK, ALT_SCAN, true), (TAB_VK, 0x0F, true)]);
        assert_eq!(actions(&ds), vec![PttDown, PttAborted]);
        assert!(!ds[1].suppress);
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert!(up.suppress && up.actions.is_empty());
    }

    #[test]
    fn left_ctrl_alt_altgr_path_is_untouched_with_right_alt_ptt() {
        // AltGr characters are typed with Left Ctrl+Left Alt instead.
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, LCTRL_SCAN, true),
                (LALT_VK, ALT_SCAN, true),
                (0x45, 0x12, true),
                (0x45, 0x12, false),
                (LALT_VK, ALT_SCAN, false),
                (LCTRL_VK, LCTRL_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![]);
        for (i, d) in ds.iter().enumerate() {
            assert_eq!(*d, Decision::default(), "event #{i} untouched");
        }
    }

    #[test]
    fn right_alt_passes_through_when_it_is_not_push_to_talk() {
        // Ctrl+Win push-to-talk: AltGr works as it always did.
        let mut m = ctrl_win_ptt();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, FAKE_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (0x45, 0x12, true),
                (0x45, 0x12, false),
                (RALT_VK, ALT_SCAN, false),
                (LCTRL_VK, FAKE_SCAN, false),
            ],
        );
        for (i, d) in ds.iter().enumerate() {
            assert_eq!(*d, Decision::default(), "event #{i} untouched");
        }
    }

    // --- Defaults (D57): Right Alt push-to-talk next to the Ctrl+Win chords ---

    fn default_bindings() -> ChordMatcher {
        ChordMatcher::new(Bindings::default())
    }

    #[test]
    fn default_push_to_talk_is_right_alt() {
        let b = Bindings::default();
        assert_eq!(b.ptt.to_string(), "Right Alt");
        assert_eq!(b.hands_free.to_string(), "Ctrl+Win+Space");
        assert_eq!(b.command.to_string(), "Ctrl+Win+Alt");
        let mut m = default_bindings();
        let d = m.handle_raw(LALT_VK, ALT_SCAN, true, false);
        assert_eq!(d, Decision::default(), "Left Alt is not push-to-talk");
    }

    #[test]
    fn default_ctrl_win_alone_is_no_shortcut_any_more() {
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
                (0x5B, 0x5B, false),
                (LCTRL_VK, LCTRL_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![]);
        assert!(ds.iter().all(|d| !d.suppress), "Ctrl+Win untouched");
    }

    #[test]
    fn default_hands_free_and_command_still_fire() {
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
                (0x20, 0x39, true),
            ],
        );
        assert_eq!(actions(&ds), vec![HandsFreeToggle]);
        assert!(ds[2].suppress);
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
                (LALT_VK, ALT_SCAN, true),
                (LALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![CommandDown, CommandUp]);
        assert!(ds[3].mask_release);
    }

    #[test]
    fn default_command_with_right_alt_is_a_command_not_a_dictation() {
        // Ctrl+Win, then Right Alt as Alt (with AltGr's fake Ctrl or not).
        for altgr in [false, true] {
            let mut m = default_bindings();
            let mut evs = vec![(LCTRL_VK, LCTRL_SCAN, true), (0x5B, 0x5B, true)];
            if altgr {
                evs.push((LCTRL_VK, FAKE_SCAN, true));
            }
            evs.push((RALT_VK, ALT_SCAN, true));
            let ds = raw(&mut m, &evs);
            assert_eq!(actions(&ds), vec![CommandDown], "altgr={altgr}");
            let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
            assert_eq!(up.actions, vec![CommandUp]);
            assert!(up.suppress && !up.mask_release, "never reached the app");
            let win_up = m.handle_raw(0x5B, 0x5B, false, false);
            assert!(win_up.mask_release, "no Start menu");
        }
    }

    #[test]
    fn right_alt_then_ctrl_win_becomes_a_command() {
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
            ],
        );
        assert_eq!(actions(&ds), vec![PttDown, CommandDown]);
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert_eq!(up.actions, vec![CommandUp], "no PttUp: superseded");
    }

    #[test]
    fn right_alt_does_not_fire_with_a_real_ctrl_held() {
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[(LCTRL_VK, LCTRL_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        assert_eq!(actions(&ds), vec![]);
        assert!(ds[1].suppress, "Right Alt is still swallowed");
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
            let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
        let a = feed(
            &mut m,
            &[KeyEvent::down(Key::Win), KeyEvent::down(Key::Ctrl)],
        );
        assert_eq!(a, vec![PttDown]);
    }

    #[test]
    fn autorepeat_does_not_refire() {
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
        let d = m.handle(KeyEvent::down(Key::Space), true);
        assert!(!d.suppress);
        assert!(d.actions.is_empty());
    }

    #[test]
    fn modifier_events_pass_through_except_masked_release() {
        let mut m = ctrl_win_ptt();
        assert!(!m.handle(KeyEvent::down(Key::Ctrl), false).suppress);
        assert!(!m.handle(KeyEvent::down(Key::Win), false).suppress);
        assert!(!m.handle(KeyEvent::up(Key::Ctrl), false).suppress);
        let d = m.handle(KeyEvent::up(Key::Win), false);
        assert!(d.suppress && d.mask_release);
    }

    #[test]
    fn win_release_after_chord_is_masked() {
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
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
        let mut m = ctrl_win_ptt();
        let d = m.handle(KeyEvent::up(Key::Win), false);
        assert_eq!(d, Decision::default());
    }

    #[test]
    fn force_release_ends_stuck_ptt() {
        let mut m = ctrl_win_ptt();
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
            r#"{"ptt":"Right Alt","hands_free":"Ctrl+Win+Space","cancel":"Esc","paste_last":"Alt+Shift+Z","copy_last":"Alt+Shift+X","command":"Ctrl+Win+Alt"}"#
        );
        let b: Bindings = serde_json::from_str(r#"{"ptt":"Ctrl+Shift"}"#).unwrap();
        assert_eq!(b.ptt.to_string(), "Ctrl+Shift");
        assert_eq!(b.cancel, Bindings::default().cancel);
    }

    #[test]
    fn scratchpad_shortcut_is_unset_by_default_and_round_trips() {
        assert_eq!(Bindings::default().scratchpad, None);
        let b: Bindings = serde_json::from_str(r#"{"scratchpad":""}"#).unwrap();
        assert_eq!(b.scratchpad, None, "empty field = unset");
        let b: Bindings = serde_json::from_str(r#"{"scratchpad":null}"#).unwrap();
        assert_eq!(b.scratchpad, None);
        let b: Bindings = serde_json::from_str(r#"{"scratchpad":"Ctrl+Alt+N"}"#).unwrap();
        assert_eq!(b.scratchpad, Some("Ctrl+Alt+N".parse().unwrap()));
        let j = serde_json::to_value(&b).unwrap();
        assert_eq!(j["scratchpad"], "Ctrl+Alt+N");
    }

    #[test]
    fn scratchpad_chord_fires_only_when_set() {
        let n = Key::Vk(b'N' as u16);
        let press = |m: &mut ChordMatcher| {
            m.handle(KeyEvent::down(Key::Ctrl), false);
            m.handle(KeyEvent::down(Key::Alt), false);
            m.handle(KeyEvent::down(n), false)
        };
        let mut unset = ctrl_win_ptt();
        let d = press(&mut unset);
        assert!(d.actions.is_empty());
        assert!(!d.suppress, "an unset shortcut leaves N to the app");
        let mut set = ChordMatcher::new(Bindings {
            scratchpad: Some("Ctrl+Alt+N".parse().unwrap()),
            ..Bindings::default()
        });
        let d = press(&mut set);
        assert_eq!(d.actions, vec![Scratchpad]);
        assert!(d.suppress, "N must not reach the app");
    }

    #[test]
    fn meeting_shortcut_is_optional_and_fires_when_set() {
        assert_eq!(Bindings::default().meeting, None);
        let j = serde_json::to_value(Bindings::default()).unwrap();
        assert!(j.get("meeting").is_none(), "unset is not written");
        let b: Bindings = serde_json::from_str(r#"{"meeting":"Ctrl+Alt+R"}"#).unwrap();
        assert_eq!(b.meeting, Some("Ctrl+Alt+R".parse().unwrap()));
        let press = |m: &mut ChordMatcher| {
            m.handle(KeyEvent::down(Key::Ctrl), false);
            m.handle(KeyEvent::down(Key::Alt), false);
            m.handle(KeyEvent::down(Key::Vk(b'R' as u16)), false)
        };
        let d = press(&mut ctrl_win_ptt());
        assert!(d.actions.is_empty() && !d.suppress);
        let d = press(&mut ChordMatcher::new(b));
        assert_eq!(d.actions, vec![Meeting]);
        assert!(d.suppress);
    }

    #[test]
    fn new_note_shortcut_is_optional_and_fires_when_set() {
        assert_eq!(Bindings::default().new_note, None);
        let j = serde_json::to_value(Bindings::default()).unwrap();
        assert!(j.get("new_note").is_none(), "unset is not written");
        let b: Bindings = serde_json::from_str(r#"{"new_note":""}"#).unwrap();
        assert_eq!(b.new_note, None);
        let b: Bindings = serde_json::from_str(r#"{"new_note":"Ctrl+Alt+M"}"#).unwrap();
        assert_eq!(b.new_note, Some("Ctrl+Alt+M".parse().unwrap()));
        assert_eq!(serde_json::to_value(&b).unwrap()["new_note"], "Ctrl+Alt+M");
        let press = |m: &mut ChordMatcher| {
            m.handle(KeyEvent::down(Key::Ctrl), false);
            m.handle(KeyEvent::down(Key::Alt), false);
            m.handle(KeyEvent::down(Key::Vk(b'M' as u16)), false)
        };
        let d = press(&mut ctrl_win_ptt());
        assert!(d.actions.is_empty() && !d.suppress);
        let d = press(&mut ChordMatcher::new(b));
        assert_eq!(d.actions, vec![NewNote]);
        assert!(d.suppress);
    }

    fn win_alt(k: char) -> Chord {
        Chord::new([Key::Win, Key::Alt, Key::Vk(k as u16)])
    }

    #[test]
    fn transform_chords_fire_their_index_and_swallow_the_digit() {
        let b = Bindings {
            transforms: vec![Some(win_alt('1')), None, Some(win_alt('3'))],
            view_changes: Some(win_alt('O')),
            ..Default::default()
        };
        let mut m = ChordMatcher::new(b);
        m.handle(KeyEvent::down(Key::Win), false);
        m.handle(KeyEvent::down(Key::LAlt), false);
        let d = m.handle(KeyEvent::down(Key::Vk(b'3' as u16)), false);
        assert_eq!(d.actions, vec![Transform(2)]);
        assert!(d.suppress, "the digit must not reach the app");
        assert!(m.handle(KeyEvent::up(Key::Vk(b'3' as u16)), false).suppress);
        let d = m.handle(KeyEvent::down(Key::Vk(b'O' as u16)), false);
        assert_eq!(d.actions, vec![ViewChanges]);
        m.handle(KeyEvent::up(Key::Vk(b'O' as u16)), false);
        assert!(
            m.handle(KeyEvent::up(Key::Win), false).mask_release,
            "a lone Win release would open Start"
        );
    }

    #[test]
    fn no_transform_chord_without_bindings_and_never_saved() {
        let mut m = ctrl_win_ptt();
        m.handle(KeyEvent::down(Key::Win), false);
        m.handle(KeyEvent::down(Key::LAlt), false);
        let d = m.handle(KeyEvent::down(Key::Vk(b'1' as u16)), false);
        assert!(d.actions.is_empty());
        assert!(
            !d.suppress,
            "Win+Alt+1 stays the taskbar's while Transforms is off"
        );
        let b = Bindings {
            transforms: vec![Some(win_alt('1'))],
            view_changes: Some(win_alt('O')),
            ..Default::default()
        };
        let j = serde_json::to_value(&b).unwrap();
        assert!(j.get("transforms").is_none() && j.get("view_changes").is_none());
    }

    #[test]
    fn set_bindings_resets_state() {
        let mut m = ctrl_win_ptt();
        feed(
            &mut m,
            &[KeyEvent::down(Key::Ctrl), KeyEvent::down(Key::Win)],
        );
        let same = m.bindings().clone();
        assert!(
            m.set_bindings(same).is_empty(),
            "unchanged: the hold goes on"
        );
        assert_eq!(m.pressed().count(), 2);
        assert_eq!(m.set_bindings(Bindings::default()), vec![PttAborted]);
        assert_eq!(m.pressed().count(), 0);
        assert!(m.handle(KeyEvent::up(Key::Ctrl), false).actions.is_empty());
    }
}
