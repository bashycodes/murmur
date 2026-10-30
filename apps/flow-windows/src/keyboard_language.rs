//! D37: the dictation language follows the active keyboard layout.
//!
//! With `prefs.keyboard_language` on (the default; see [`crate::prefs`]
//! rev 4), every dictation start reads the keyboard layout of the
//! foreground window's thread (`GetKeyboardLayout(GetWindowThreadProcessId(
//! GetForegroundWindow()))`): its low word is the LANGID, whose primary
//! language (`LANGID & 0x3ff`, e.g. `0x0D` Hebrew, `0x09` English) becomes
//! the session's forced language, so a Hebrew layout routes to the Hebrew
//! model (D33). A layout whose language is unknown, or none, keeps today's
//! behaviour: `prefs.languages` (empty = auto-detect, with language ID when
//! `lid_path` is set, D34).
//!
//! The mapping and the resolution are pure (tested on any host); the
//! Win32 read is in [`win`].

use serde::Serialize;

/// Where a dictation's language came from (debug log `language_source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The foreground window's keyboard layout.
    Keyboard,
    /// `prefs.languages` (one or more languages the user chose).
    Fixed,
    /// Nothing forced: auto-detect.
    Auto,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Keyboard => "keyboard",
            Source::Fixed => "fixed",
            Source::Auto => "auto",
        }
    }
}

/// Primary language id (`PRIMARYLANGID`) → whisper / ISO 639-1 code.
/// Only unambiguous ids: `0x1A` (Croatian / Serbian / Bosnian) and the
/// like are left out, so those layouts fall back to auto-detect.
const PRIMARY: &[(u16, &str)] = &[
    (0x01, "ar"),
    (0x02, "bg"),
    (0x03, "ca"),
    (0x04, "zh"),
    (0x05, "cs"),
    (0x06, "da"),
    (0x07, "de"),
    (0x08, "el"),
    (0x09, "en"),
    (0x0A, "es"),
    (0x0B, "fi"),
    (0x0C, "fr"),
    (0x0D, "he"),
    (0x0E, "hu"),
    (0x0F, "is"),
    (0x10, "it"),
    (0x11, "ja"),
    (0x12, "ko"),
    (0x13, "nl"),
    (0x14, "no"),
    (0x15, "pl"),
    (0x16, "pt"),
    (0x18, "ro"),
    (0x19, "ru"),
    (0x1B, "sk"),
    (0x1C, "sq"),
    (0x1D, "sv"),
    (0x1E, "th"),
    (0x1F, "tr"),
    (0x20, "ur"),
    (0x21, "id"),
    (0x22, "uk"),
    (0x23, "be"),
    (0x24, "sl"),
    (0x25, "et"),
    (0x26, "lv"),
    (0x27, "lt"),
    (0x29, "fa"),
    (0x2A, "vi"),
    (0x2B, "hy"),
    (0x2C, "az"),
    (0x2D, "eu"),
    (0x2F, "mk"),
    (0x36, "af"),
    (0x37, "ka"),
    (0x39, "hi"),
    (0x3E, "ms"),
    (0x3F, "kk"),
    (0x41, "sw"),
    (0x45, "bn"),
    (0x46, "pa"),
    (0x47, "gu"),
    (0x49, "ta"),
    (0x4A, "te"),
    (0x4B, "kn"),
    (0x4C, "ml"),
    (0x4E, "mr"),
    (0x50, "mn"),
    (0x56, "gl"),
    (0x61, "ne"),
];

/// The language of a LANGID (`0x040D` he-IL → `he`, `0x0409` en-US →
/// `en`); `None` for 0 and ids the table does not know.
pub fn language_for_langid(langid: u16) -> Option<&'static str> {
    let primary = langid & 0x3ff;
    PRIMARY
        .iter()
        .find(|(id, _)| *id == primary)
        .map(|(_, c)| *c)
}

/// The LANGID of an HKL value: its low word.
pub fn langid_of_hkl(hkl: usize) -> u16 {
    (hkl & 0xffff) as u16
}

/// The session's languages and their source. `keyboard_pref` =
/// `prefs.keyboard_language`; `keyboard` = the layout's language (`None` =
/// unknown or not read). Without a keyboard language, `languages` as today.
pub fn resolve(
    languages: &[String],
    keyboard_pref: bool,
    keyboard: Option<&str>,
) -> (Vec<String>, Source) {
    if keyboard_pref {
        if let Some(k) = keyboard {
            return (vec![k.to_string()], Source::Keyboard);
        }
    }
    let source = if languages.is_empty() {
        Source::Auto
    } else {
        Source::Fixed
    };
    (languages.to_vec(), source)
}

/// Win32: the foreground window's keyboard layout.
#[cfg(windows)]
pub mod win {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    /// The LANGID of the foreground window's thread's layout; `None`
    /// without a foreground window (the layout of our own thread would be
    /// the wrong one). Cheap; any thread.
    pub fn foreground_langid() -> Option<u16> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_invalid() {
                return None;
            }
            let tid = GetWindowThreadProcessId(hwnd, None);
            if tid == 0 {
                return None;
            }
            let hkl = GetKeyboardLayout(tid);
            Some(super::langid_of_hkl(hkl.0 as usize)).filter(|id| *id != 0)
        }
    }

    /// [`super::language_for_langid`] of [`foreground_langid`].
    pub fn foreground_language() -> Option<&'static str> {
        foreground_langid().and_then(super::language_for_langid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn langid_maps_by_primary_language() {
        assert_eq!(language_for_langid(0x040D), Some("he"), "he-IL");
        assert_eq!(language_for_langid(0x000D), Some("he"));
        assert_eq!(language_for_langid(0x0409), Some("en"), "en-US");
        assert_eq!(language_for_langid(0x0809), Some("en"), "en-GB");
        assert_eq!(language_for_langid(0x0407), Some("de"));
        assert_eq!(language_for_langid(0x0419), Some("ru"));
        assert_eq!(language_for_langid(0x0401), Some("ar"));
    }

    #[test]
    fn unknown_langids_are_none() {
        assert_eq!(language_for_langid(0), None);
        assert_eq!(language_for_langid(0x041A), None, "hr/sr/bs share an id");
        assert_eq!(language_for_langid(0x0464), None, "Filipino");
        assert_eq!(language_for_langid(0x03ff), None);
    }

    #[test]
    fn hkl_low_word_is_the_langid() {
        assert_eq!(langid_of_hkl(0x040D_040D), 0x040D);
        assert_eq!(
            langid_of_hkl(0xF03D_0409),
            0x0409,
            "a layout variant (Dvorak) keeps the language"
        );
        assert_eq!(language_for_langid(langid_of_hkl(0x040D_040D)), Some("he"));
    }

    #[test]
    fn keyboard_wins_when_on_and_known() {
        let (l, s) = resolve(&[], true, Some("he"));
        assert_eq!((l, s), (vec!["he".to_string()], Source::Keyboard));
        let (l, s) = resolve(&["de".into()], true, Some("he"));
        assert_eq!((l, s), (vec!["he".to_string()], Source::Keyboard));
    }

    #[test]
    fn unknown_keyboard_falls_back_to_todays_languages() {
        assert_eq!(
            resolve(&[], true, None),
            (vec![], Source::Auto),
            "auto (LID when set)"
        );
        assert_eq!(
            resolve(&["de".into()], true, None),
            (vec!["de".to_string()], Source::Fixed)
        );
    }

    #[test]
    fn keyboard_off_ignores_the_layout() {
        assert_eq!(resolve(&[], false, Some("he")), (vec![], Source::Auto));
        assert_eq!(
            resolve(&["en".into()], false, Some("he")),
            (vec!["en".to_string()], Source::Fixed)
        );
    }

    #[test]
    fn source_names() {
        assert_eq!(Source::Keyboard.as_str(), "keyboard");
        assert_eq!(Source::Fixed.as_str(), "fixed");
        assert_eq!(Source::Auto.as_str(), "auto");
    }
}
