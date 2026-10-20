//! `prefs.json` in `%APPDATA%\FlowClone\` (replaces Wispr's electron-store
//! `config.json`). Unknown/missing fields fall back to defaults so old files
//! keep loading; a corrupt file is moved aside rather than silently lost.

use crate::core_api::{CleanupLevel, SessionConfig, Styles};
use crate::dictation::Policy;
use crate::hotkeys::Bindings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "FlowClone";
pub const FILE_NAME: &str = "prefs.json";

/// Shared dictionary/snippet export shape (spec §9): `{word, replacement, isSnippet}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictEntry {
    pub word: String,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub is_snippet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteMethod {
    #[default]
    CtrlV,
    ShiftInsert,
    /// KEYEVENTF_UNICODE typing (no clipboard involvement at all).
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    #[default]
    Keep,
    Day,
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub shortcuts: Bindings,
    /// Input device name; `None` = system default.
    pub microphone: Option<String>,
    /// Empty = auto-detect.
    pub languages: Vec<String>,
    pub enable_sounds: bool,
    /// Wispr default on Windows: mute other apps while dictating.
    pub mute_audio_while_dictating: bool,
    pub launch_at_login: bool,
    pub show_flow_bar_always: bool,
    /// Read the focused field via UIA (local only; never leaves the machine).
    pub use_ax_context: bool,
    pub cleanup_level: CleanupLevel,
    pub raw_mode: bool,
    pub styles: Styles,
    pub dictionary: Vec<DictEntry>,
    pub paste_method: PasteMethod,
    pub restore_clipboard: bool,
    /// Wait between Ctrl+V and clipboard restore.
    pub paste_restore_delay_ms: u64,
    /// Texts at most this long use KEYEVENTF_UNICODE when the clipboard
    /// can't be opened.
    pub type_fallback_max_chars: usize,
    pub retention: Retention,
    pub max_dictation_minutes: u64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            shortcuts: Bindings::default(),
            microphone: None,
            languages: Vec::new(),
            enable_sounds: true,
            mute_audio_while_dictating: true,
            launch_at_login: false,
            show_flow_bar_always: true,
            use_ax_context: true,
            cleanup_level: CleanupLevel::Light,
            raw_mode: false,
            styles: Styles {
                personal: "casual".into(),
                work: "formal".into(),
                email: "formal".into(),
                other: "formal".into(),
            },
            dictionary: Vec::new(),
            paste_method: PasteMethod::CtrlV,
            restore_clipboard: true,
            paste_restore_delay_ms: 150,
            type_fallback_max_chars: 200,
            retention: Retention::Keep,
            max_dictation_minutes: 20,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    Missing,
    /// File was unparsable and moved to the contained path.
    Corrupt(PathBuf),
}

impl Prefs {
    /// `%APPDATA%\FlowClone\prefs.json` (falls back to the CWD).
    pub fn default_path(appdata: Option<&str>) -> PathBuf {
        let base = appdata.map(PathBuf::from).unwrap_or_default();
        base.join(APP_DIR).join(FILE_NAME)
    }

    pub fn load(path: &Path) -> (Prefs, LoadOutcome) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return (Prefs::default(), LoadOutcome::Missing),
        };
        match serde_json::from_str::<Prefs>(&text) {
            Ok(p) => (p, LoadOutcome::Loaded),
            Err(_) => {
                let bak = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, &bak);
                (Prefs::default(), LoadOutcome::Corrupt(bak))
            }
        }
    }

    /// Atomic write (tmp + rename) so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    pub fn policy(&self) -> Policy {
        let minutes = self.max_dictation_minutes.max(1);
        Policy {
            max_ms: minutes * 60_000,
            warn_ms: minutes.saturating_sub(1) * 60_000,
            ..Policy::default()
        }
    }

    pub fn session_config(&self) -> SessionConfig {
        let mut dictionary = Vec::new();
        let mut snippets = BTreeMap::new();
        let mut replacements = BTreeMap::new();
        for e in &self.dictionary {
            match (&e.replacement, e.is_snippet) {
                (Some(r), true) => {
                    snippets.insert(e.word.clone(), r.clone());
                }
                (Some(r), false) => {
                    replacements.insert(e.word.clone(), r.clone());
                }
                (None, _) => dictionary.push(e.word.clone()),
            }
        }
        SessionConfig {
            languages: self.languages.clone(),
            dictionary,
            snippets,
            replacements,
            styles: self.styles.clone(),
            cleanup_level: self.cleanup_level,
            raw_mode: self.raw_mode,
            max_samples: self.policy().max_samples(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("flow-windows-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn default_path_uses_appdata() {
        let p = Prefs::default_path(Some(r"C:\Users\u\AppData\Roaming"));
        assert!(p.ends_with(Path::new(APP_DIR).join(FILE_NAME)));
        assert!(p.starts_with(r"C:\Users\u\AppData\Roaming"));
        assert_eq!(
            Prefs::default_path(None),
            PathBuf::from(APP_DIR).join(FILE_NAME)
        );
    }

    #[test]
    fn missing_file_gives_defaults() {
        let d = tmpdir("missing");
        let (p, o) = Prefs::load(&d.join(FILE_NAME));
        assert_eq!(p, Prefs::default());
        assert_eq!(o, LoadOutcome::Missing);
    }

    #[test]
    fn save_then_load_roundtrip_creates_dirs() {
        let d = tmpdir("roundtrip");
        let path = d.join("nested").join(FILE_NAME);
        let mut p = Prefs::default();
        p.microphone = Some("USB Mic".into());
        p.shortcuts.ptt = "Ctrl+Alt".parse().unwrap();
        p.save(&path).unwrap();
        let (q, o) = Prefs::load(&path);
        assert_eq!(o, LoadOutcome::Loaded);
        assert_eq!(q, p);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let d = tmpdir("partial");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(FILE_NAME);
        std::fs::write(
            &path,
            r#"{"enable_sounds": false, "unknown_future_key": 1}"#,
        )
        .unwrap();
        let (p, o) = Prefs::load(&path);
        assert_eq!(o, LoadOutcome::Loaded);
        assert!(!p.enable_sounds);
        assert!(p.mute_audio_while_dictating);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let d = tmpdir("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(FILE_NAME);
        std::fs::write(&path, "{not json").unwrap();
        let (p, o) = Prefs::load(&path);
        assert_eq!(p, Prefs::default());
        match o {
            LoadOutcome::Corrupt(bak) => {
                assert_eq!(std::fs::read_to_string(&bak).unwrap(), "{not json");
                assert!(!path.exists());
            }
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn policy_respects_max_minutes() {
        let mut p = Prefs::default();
        assert_eq!(p.policy().max_ms, 20 * 60_000);
        assert_eq!(p.policy().warn_ms, 19 * 60_000);
        p.max_dictation_minutes = 5;
        assert_eq!(p.policy().max_ms, 5 * 60_000);
        assert_eq!(p.policy().warn_ms, 4 * 60_000);
        p.max_dictation_minutes = 0;
        assert_eq!(p.policy().max_ms, 60_000, "clamped to ≥1 min");
    }

    #[test]
    fn session_config_splits_dictionary() {
        let mut p = Prefs::default();
        p.languages = vec!["en".into()];
        p.dictionary = vec![
            DictEntry {
                word: "Wispr".into(),
                replacement: None,
                is_snippet: false,
            },
            DictEntry {
                word: "gh".into(),
                replacement: Some("GitHub".into()),
                is_snippet: false,
            },
            DictEntry {
                word: "my address".into(),
                replacement: Some("1 Main St".into()),
                is_snippet: true,
            },
        ];
        let c = p.session_config();
        assert_eq!(c.languages, vec!["en"]);
        assert_eq!(c.dictionary, vec!["Wispr"]);
        assert_eq!(c.replacements.get("gh").map(String::as_str), Some("GitHub"));
        assert_eq!(
            c.snippets.get("my address").map(String::as_str),
            Some("1 Main St")
        );
        assert_eq!(c.max_samples, p.policy().max_samples());
        assert_eq!(c.cleanup_level, CleanupLevel::Light);
    }

    #[test]
    fn dict_entry_json_shape_matches_android() {
        let e: DictEntry =
            serde_json::from_str(r#"{"word":"a","replacement":"b","isSnippet":true}"#).unwrap();
        assert!(e.is_snippet);
    }
}
