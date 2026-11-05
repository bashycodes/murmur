//! Platform-independent chord matcher.
//!
//! The LL keyboard hook feeds every physical key event in; the matcher answers
//! (a) which hotkey actions fired, (b) whether to swallow the event, and
//! (c) whether a dummy VK 0xFF must be injected *before* letting a Win/Alt
//! key-up through (so a "lone" Win release doesn't open Start, and a lone Alt
//! release doesn't focus the menu bar — windows-research 02 §2).
//!
//! A push-to-talk shortcut that is one modifier on its own (the default,
//! Right Alt) is *armed* on key-down instead of firing (D57): it fires
//! once held alone for [`LONE_KEY_ARM_MS`], or as a tap when released
//! sooner, and any other key pressed meanwhile disarms it silently. So
//! AltGr + E still types € on layouts where Right Alt is AltGr, with no
//! dictation starting and stopping on the way.

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

    /// One modifier key on its own ("Right Alt"): such a push-to-talk
    /// shortcut is armed before it fires ([`LONE_KEY_ARM_MS`]).
    pub fn is_lone_modifier(&self) -> bool {
        self.keys.len() == 1 && self.keys.iter().all(|k| k.is_modifier())
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
    /// A lone-key push-to-talk was armed: start a [`LONE_KEY_ARM_MS`]
    /// timer and call [`ChordMatcher::arm_elapsed`] when it ends.
    pub arm: bool,
}

/// How long a one-modifier push-to-talk (Right Alt) must be held alone
/// before dictation starts (D57). AltGr + a letter is usually typed well
/// within it; a key pressed later still aborts the dictation, as any
/// foreign key does.
pub const LONE_KEY_ARM_MS: u32 = 200;

/// Where the push-to-talk shortcut is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ptt {
    Idle,
    /// A lone-key push-to-talk is held alone but has not fired yet.
    Armed,
    /// `PttDown` was reported; `PttUp` follows at the release.
    Active,
}

/// Stateful chord matcher. Feed physical (non-injected) events only.
#[derive(Debug, Clone)]
pub struct ChordMatcher {
    bindings: Bindings,
    pressed: BTreeSet<Key>,
    ptt: Ptt,
    /// The Command chord fired and none of its keys was released yet.
    command_active: bool,
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
            ptt: Ptt::Idle,
            command_active: false,
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
        self.ptt = Ptt::Idle;
        self.command_active = false;
        self.swallowed.clear();
        self.needs_mask.clear();
        self.altgr_ctrl = false;
    }

    pub fn pressed(&self) -> impl Iterator<Item = Key> + '_ {
        self.pressed.iter().copied()
    }

    /// Stale-key recovery: the OS says `key` is no longer down although we
    /// never saw its key-up (GetAsyncKeyState check). Treated as a release.
    /// An armed push-to-talk is dropped, not reported as a tap.
    pub fn force_release(&mut self, key: Key) -> Vec<HotkeyAction> {
        if self.pressed.contains(&key) {
            if self.ptt == Ptt::Armed {
                self.ptt = Ptt::Idle;
            }
            self.handle(KeyEvent::up(key), false).actions
        } else {
            Vec::new()
        }
    }

    /// The arm timer ([`Decision::arm`]) ended: an armed push-to-talk
    /// still held alone fires now. Nothing when it was disarmed meanwhile.
    pub fn arm_elapsed(&mut self) -> Vec<HotkeyAction> {
        if self.ptt != Ptt::Armed {
            return Vec::new();
        }
        self.ptt = Ptt::Active;
        // Only now is its release masked: a disarmed Right Alt (AltGr in
        // use) goes up untouched.
        self.chord_fired();
        vec![HotkeyAction::PttDown]
    }

    /// A lone-key push-to-talk is armed (its timer still matters).
    pub fn is_armed(&self) -> bool {
        self.ptt == Ptt::Armed
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
            // Another key with an armed lone-key push-to-talk: it was a
            // combination (AltGr + E, Right Alt + Tab), not a hold.
            if self.ptt == Ptt::Armed {
                self.ptt = Ptt::Idle;
            }
            if self.newly_matches(&self.bindings.hands_free, ev.key) {
                // Hands-free chord supersedes a PTT that started on the way.
                self.ptt = Ptt::Idle;
                d.actions.push(HotkeyAction::HandsFreeToggle);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.newly_matches(&self.bindings.command, ev.key) {
                // Command supersedes a PTT that started on the way
                // (Ctrl+Win, then Alt).
                self.ptt = Ptt::Idle;
                self.command_active = true;
                d.actions.push(HotkeyAction::CommandDown);
                self.chord_fired();
                if !ev.key.is_modifier() {
                    d.suppress = true;
                }
            } else if self.newly_matches(&self.bindings.ptt, ev.key) {
                if self.bindings.ptt.is_lone_modifier() {
                    // Not marked for a masked release yet: that waits
                    // until it fires or is tapped (`arm_elapsed`, key-up).
                    self.ptt = Ptt::Armed;
                    d.arm = true;
                } else {
                    self.ptt = Ptt::Active;
                    self.chord_fired();
                    d.actions.push(HotkeyAction::PttDown);
                    if !ev.key.is_modifier() {
                        d.suppress = true;
                    }
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
            } else if self.ptt == Ptt::Active
                && !self.bindings.ptt.covers(ev.key)
                && !self.bindings.hands_free.covers(ev.key)
                && !self.bindings.command.covers(ev.key)
            {
                self.ptt = Ptt::Idle;
                d.actions.push(HotkeyAction::PttAborted);
            } else if self.command_active && !self.bindings.command.covers(ev.key) {
                self.command_active = false;
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
            if self.ptt != Ptt::Idle && self.bindings.ptt.covers(ev.key) {
                if self.ptt == Ptt::Armed {
                    // A tap shorter than the arm time: report it as one, so
                    // a double tap still locks hands-free, and mask its
                    // release (a lone Alt tap would focus the menu bar).
                    d.actions.push(HotkeyAction::PttDown);
                    if ev.key.needs_release_mask() {
                        self.needs_mask.insert(ev.key);
                    }
                }
                self.ptt = Ptt::Idle;
                d.actions.push(HotkeyAction::PttUp);
            }
            if self.command_active && self.bindings.command.covers(ev.key) {
                self.command_active = false;
                d.actions.push(HotkeyAction::CommandUp);
            }
            d.suppress = self.swallowed.remove(&ev.key);
            if !d.suppress && self.needs_mask.remove(&ev.key) {
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
    fn right_alt_alone_fires_ptt_and_its_release_is_masked() {
        let mut m = ralt_ptt();
        let d = m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert!(d.actions.is_empty() && d.arm, "armed, not fired yet");
        assert!(!d.suppress, "modifier key-down passes through");
        assert_eq!(m.arm_elapsed(), vec![PttDown]);
        assert!(m.arm_elapsed().is_empty(), "fires once");
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
        assert_eq!(up.actions, vec![PttDown, PttUp], "a tap");
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
    fn altgr_character_never_starts_a_dictation_and_reaches_the_app() {
        // AltGr + E (€) typed within the arm time.
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
        assert_eq!(actions(&ds), vec![], "no start, no abort, no tap");
        assert!(ds[1].arm);
        assert!(m.arm_elapsed().is_empty(), "disarmed by the letter");
        assert!(
            ds[..4].iter().all(|d| !d.suppress),
            "AltGr + E reaches the app"
        );
        assert_eq!(ds[4], Decision::default(), "real Right Alt up, no mask");
        assert_eq!(ds[5], Decision::default(), "fake Ctrl up passes");
        assert_eq!(m.pressed().count(), 0);
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn altgr_character_release_order_fake_ctrl_up_first() {
        // Windows may release AltGr's fake Left Ctrl before Right Alt.
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
        assert_eq!(actions(&ds), vec![]);
        assert_eq!(ds[4], Decision::default());
        assert_eq!(ds[5], Decision::default(), "real Right Alt up, no mask");
        assert!(m.arm_elapsed().is_empty());
        assert_eq!(m.pressed().count(), 0);
        assert!(!m.altgr_ctrl_down());
    }

    #[test]
    fn right_alt_used_with_a_key_on_a_us_layout_goes_up_unmasked() {
        // No AltGr: Right Alt + F is Alt + F (the app's menu shortcut).
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (0x46, 0x21, true),
                (0x46, 0x21, false),
                (RALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(actions(&ds), vec![]);
        assert_eq!(ds[3], Decision::default());
    }

    #[test]
    fn altgr_character_after_the_arm_time_aborts_the_dictation() {
        let mut m = ralt_ptt();
        raw(
            &mut m,
            &[(LCTRL_VK, FAKE_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        assert_eq!(m.arm_elapsed(), vec![PttDown]);
        let d = m.handle_raw(0x45, 0x12, true, false);
        assert_eq!(d.actions, vec![PttAborted]);
        assert!(!d.suppress, "the letter still reaches the app");
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert!(
            up.actions.is_empty(),
            "an aborted dictation reports no release"
        );
    }

    #[test]
    fn right_alt_tab_is_not_a_dictation() {
        let mut m = ralt_ptt();
        let ds = raw(&mut m, &[(RALT_VK, ALT_SCAN, true), (TAB_VK, 0x0F, true)]);
        assert_eq!(actions(&ds), vec![]);
        assert!(m.arm_elapsed().is_empty());
        assert!(!ds[1].suppress, "Tab reaches the app");
        let mut m = ralt_ptt();
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        m.arm_elapsed();
        let d = m.handle_raw(TAB_VK, 0x0F, true, false);
        assert_eq!(d.actions, vec![PttAborted], "held past the arm time");
        m.handle_raw(TAB_VK, 0x0F, false, false);
        assert!(
            m.handle_raw(RALT_VK, ALT_SCAN, false, false).mask_release,
            "it fired: its release is masked"
        );
    }

    #[test]
    fn right_alt_tap_is_reported_at_release_and_double_tap_twice() {
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, false),
                (RALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, false),
            ],
        );
        assert_eq!(
            actions(&ds),
            vec![PttDown, PttUp, PttDown, PttUp],
            "the dictation state machine turns a quick double into hands-free"
        );
        assert!(
            ds[1].mask_release && ds[3].mask_release,
            "no menu bar on a tap"
        );
        assert!(
            m.arm_elapsed().is_empty(),
            "a late timer finds nothing armed"
        );
    }

    #[test]
    fn right_alt_autorepeat_keeps_it_armed() {
        let mut m = ralt_ptt();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
                (RALT_VK, ALT_SCAN, true),
            ],
        );
        assert!(ds[0].arm && !ds[1].arm && !ds[2].arm, "one timer");
        assert_eq!(m.arm_elapsed(), vec![PttDown]);
    }

    #[test]
    fn reset_or_new_bindings_drop_an_armed_ptt() {
        let mut m = ralt_ptt();
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        m.reset();
        assert!(m.arm_elapsed().is_empty());
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        m.set_bindings(Bindings::default());
        assert!(m.arm_elapsed().is_empty());
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert!(
            m.force_release(Key::RAlt).is_empty(),
            "a stuck key is no tap"
        );
        assert!(m.arm_elapsed().is_empty());
    }

    #[test]
    fn only_a_lone_modifier_ptt_is_armed() {
        assert!(Chord::new([Key::RAlt]).is_lone_modifier());
        assert!(Chord::new([Key::Ctrl]).is_lone_modifier());
        assert!(!Chord::new([Key::Ctrl, Key::Win]).is_lone_modifier());
        assert!(!Chord::new([Key::Vk(0x70)]).is_lone_modifier());
        let mut m = ctrl_win_ptt();
        let d = feed(
            &mut m,
            &[KeyEvent::down(Key::Ctrl), KeyEvent::down(Key::Win)],
        );
        assert_eq!(d, vec![PttDown], "Ctrl+Win fires at once, as before");
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
        assert!(m
            .handle_raw(LALT_VK, ALT_SCAN, true, false)
            .actions
            .is_empty());
        assert!(!m.handle_raw(LALT_VK, ALT_SCAN, true, false).arm);
        assert!(m.arm_elapsed().is_empty(), "Left Alt is not push-to-talk");
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
        assert!(
            ds.iter().all(|d| !d.suppress && !d.arm),
            "Ctrl+Win untouched"
        );
    }

    #[test]
    fn default_hands_free_and_command_still_fire_without_push_to_talk() {
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
        // Ctrl+Win, then Right Alt as Alt; on an AltGr layout the fake
        // Left Ctrl comes first. Right Alt is not alone: never armed.
        for altgr in [false, true] {
            let mut m = default_bindings();
            let mut evs = vec![(LCTRL_VK, LCTRL_SCAN, true), (0x5B, 0x5B, true)];
            if altgr {
                evs.push((LCTRL_VK, FAKE_SCAN, true));
            }
            evs.push((RALT_VK, ALT_SCAN, true));
            let ds = raw(&mut m, &evs);
            assert_eq!(actions(&ds), vec![CommandDown], "altgr={altgr}");
            assert!(ds.iter().all(|d| !d.arm));
            let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
            assert_eq!(up.actions, vec![CommandUp]);
            assert!(up.mask_release);
        }
    }

    #[test]
    fn right_alt_then_ctrl_win_becomes_a_command() {
        // Held past the arm time: the dictation turns into a command.
        let mut m = default_bindings();
        m.handle_raw(RALT_VK, ALT_SCAN, true, false);
        assert_eq!(m.arm_elapsed(), vec![PttDown]);
        let d = m.handle_raw(LCTRL_VK, LCTRL_SCAN, true, false);
        assert!(
            d.actions.is_empty(),
            "Ctrl belongs to the Command chord: no abort"
        );
        let d = m.handle_raw(0x5B, 0x5B, true, false);
        assert_eq!(d.actions, vec![CommandDown]);
        let up = m.handle_raw(RALT_VK, ALT_SCAN, false, false);
        assert_eq!(up.actions, vec![CommandUp], "no PttUp: superseded");
        // Within the arm time: just the command, no dictation first.
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[
                (RALT_VK, ALT_SCAN, true),
                (LCTRL_VK, LCTRL_SCAN, true),
                (0x5B, 0x5B, true),
            ],
        );
        assert_eq!(actions(&ds), vec![CommandDown]);
        assert!(m.arm_elapsed().is_empty());
    }

    #[test]
    fn right_alt_does_not_fire_with_a_real_ctrl_held() {
        // Ctrl + Right Alt is not "Right Alt alone" (and on a non-AltGr
        // layout it is Ctrl+Alt+…, an app shortcut).
        let mut m = default_bindings();
        let ds = raw(
            &mut m,
            &[(LCTRL_VK, LCTRL_SCAN, true), (RALT_VK, ALT_SCAN, true)],
        );
        assert_eq!(actions(&ds), vec![]);
        assert!(!ds[1].arm);
    }

    #[test]
    fn either_side_alt_matches_both_sides_and_does_not_refire() {
        let b = Bindings {
            ptt: "Alt".parse().unwrap(),
            ..Bindings::default()
        };
        for vk in [LALT_VK, RALT_VK] {
            let mut m = ChordMatcher::new(b.clone());
            assert!(m.handle_raw(vk, ALT_SCAN, true, false).arm);
            assert_eq!(m.arm_elapsed(), vec![PttDown]);
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
            vec![],
            "two Alts aren't one held alone: disarmed, nothing fires"
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
        m.arm_elapsed();
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
        let mut m = default_bindings();
        assert_eq!(
            m.handle(KeyEvent::up(Key::RAlt), false),
            Decision::default()
        );
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
        m.set_bindings(Bindings::default());
        assert_eq!(m.pressed().count(), 0);
        assert!(m.handle(KeyEvent::up(Key::Ctrl), false).actions.is_empty());
    }
}
