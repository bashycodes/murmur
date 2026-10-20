//! `prefs.json` in `%APPDATA%\FlowClone\` (replaces Wispr's electron-store
//! `config.json`). Unknown/missing fields fall back to defaults so old files
//! keep loading; a corrupt file is moved aside rather than silently lost.

pub use crate::core_api::DictEntry;
use crate::core_api::{CleanupLevel, SessionConfig, Styles};
use crate::dictation::Policy;
use crate::hotkeys::{Bindings, Chord};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "FlowClone";
pub const FILE_NAME: &str = "prefs.json";

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
    /// Override for the wait between Ctrl+V and clipboard restore; `None` =
    /// per-app default (500 ms, 3 s Outlook/Word/WhatsApp, 5 s remote viewers).
    pub paste_restore_delay_ms: Option<u64>,
    /// Texts at most this long use KEYEVENTF_UNICODE when the clipboard
    /// can't be opened.
    pub type_fallback_max_chars: usize,
    pub retention: Retention,
    pub max_dictation_minutes: u64,
    /// Whisper ggml model; `None` = `%APPDATA%\FlowClone\models\ggml-base.bin`.
    /// Never downloaded automatically (DECISIONS D9).
    pub model_path: Option<String>,
    /// ASR threads; `None` = core default (half the logical CPUs).
    pub asr_threads: Option<usize>,
    /// Show the live partial transcript above the pill.
    pub live_preview: bool,
    pub live_preview_interval_ms: u32,
    /// Local LLM cleanup (llama.cpp via the `flow_llm` plugin). Off by
    /// default (DECISIONS D5): deterministic post-processing always runs.
    pub llm_enabled: bool,
    /// GGUF instruct model; `None` = `%APPDATA%\FlowClone\models\`
    /// `qwen2.5-0.5b-instruct-q4_k_m.gguf` (D11). Never downloaded.
    pub llm_model_path: Option<String>,
    /// Explicit `flow_llm.dll`; `None` = next to the exe (the shipped layout).
    pub llm_lib_path: Option<String>,
    /// Cleanup budget per dictation before falling back to RAW_TRANSCRIPT.
    pub llm_timeout_ms: u64,
    /// Watch the field after a paste and suggest dictionary words from the
    /// user's corrections (03 §3). Suggestions are never auto-added.
    pub learn_from_edits: bool,
    /// How long to watch after a paste.
    pub learn_window_ms: u64,
    /// Re-read interval while watching.
    pub learn_poll_ms: u64,
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
            // 03 §1.3: every Wispr bucket defaults to formal + light.
            styles: Styles {
                personal: "formal".into(),
                work: "formal".into(),
                email: "formal".into(),
                other: "formal".into(),
            },
            dictionary: Vec::new(),
            paste_method: PasteMethod::CtrlV,
            restore_clipboard: true,
            paste_restore_delay_ms: None,
            type_fallback_max_chars: 200,
            retention: Retention::Keep,
            max_dictation_minutes: 20,
            model_path: None,
            asr_threads: None,
            live_preview: true,
            live_preview_interval_ms: 1500,
            llm_enabled: false,
            llm_model_path: None,
            llm_lib_path: None,
            llm_timeout_ms: 1500,
            learn_from_edits: true,
            learn_window_ms: 45_000,
            learn_poll_ms: 1_000,
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

    /// Reject shortcut sets the matcher can't serve sensibly.
    pub fn validate(&self) -> Result<(), String> {
        let s = &self.shortcuts;
        let check = |name: &str, c: &Chord, allow_single: bool| {
            Chord::parse_validated(&c.to_string(), allow_single)
                .map(|_| ())
                .map_err(|e| format!("{name} shortcut: {e}"))
        };
        check("push-to-talk", &s.ptt, false)?;
        check("hands-free", &s.hands_free, false)?;
        check("cancel", &s.cancel, true)?;
        check("paste-last", &s.paste_last, false)?;
        check("copy-last", &s.copy_last, false)?;
        let all = [
            &s.ptt,
            &s.hands_free,
            &s.cancel,
            &s.paste_last,
            &s.copy_last,
        ];
        for (i, a) in all.iter().enumerate() {
            if all[i + 1..].contains(a) {
                return Err("shortcuts must differ from each other".into());
            }
        }
        Ok(())
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
        SessionConfig {
            languages: self.languages.clone(),
            dictionary: self
                .dictionary
                .iter()
                .filter(|e| !e.word.trim().is_empty())
                .cloned()
                .collect(),
            styles: self.styles.clone(),
            cleanup_level: self.cleanup_level,
            raw_mode: self.raw_mode,
            max_samples: self.policy().max_samples(),
            partial_interval_ms: self
                .live_preview
                .then_some(self.live_preview_interval_ms.max(250)),
            llm_timeout_ms: self.llm_timeout_ms.clamp(200, 30_000),
        }
    }

    pub fn watch_opts(&self) -> crate::learn::WatchOpts {
        let window_ms = self.learn_window_ms.clamp(1_000, 120_000);
        crate::learn::WatchOpts {
            window_ms,
            baseline_ms: 3_000.min(window_ms),
            poll_ms: self.learn_poll_ms.clamp(100, 5_000),
        }
    }

    /// The GGUF to load when cleanup is enabled.
    pub fn resolved_llm_path(&self, appdata: Option<&str>) -> PathBuf {
        match self.llm_model_path.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => crate::core_adapter::default_llm_path(appdata),
        }
    }

    /// `%APPDATA%\FlowClone\models\ggml-base.bin` unless overridden.
    pub fn resolved_model_path(&self, appdata: Option<&str>) -> PathBuf {
        match self.model_path.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => crate::core_adapter::default_model_path(appdata),
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
    fn session_config_passes_dictionary_in_order() {
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
        p.dictionary.push(DictEntry {
            word: "  ".into(),
            replacement: None,
            is_snippet: false,
        });
        let c = p.session_config();
        assert_eq!(c.languages, vec!["en"]);
        assert_eq!(c.dictionary.len(), 3, "blank word dropped, order kept");
        assert_eq!(c.dictionary[0].word, "Wispr");
        assert_eq!(c.dictionary[1].replacement.as_deref(), Some("GitHub"));
        assert!(c.dictionary[2].is_snippet);
        assert_eq!(c.partial_interval_ms, Some(1500));
        assert_eq!(c.max_samples, p.policy().max_samples());
        assert_eq!(c.cleanup_level, CleanupLevel::Light);
    }

    #[test]
    fn dict_entry_json_shape_matches_android() {
        let e: DictEntry =
            serde_json::from_str(r#"{"word":"a","replacement":"b","isSnippet":true}"#).unwrap();
        assert!(e.is_snippet);
    }

    #[test]
    fn validate_accepts_defaults() {
        assert_eq!(Prefs::default().validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_modifierless_or_clashing_shortcuts() {
        let mut p = Prefs::default();
        p.shortcuts.ptt = "A".parse().unwrap();
        assert!(p.validate().unwrap_err().contains("push-to-talk"));
        let mut p = Prefs::default();
        p.shortcuts.hands_free = p.shortcuts.ptt.clone();
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.shortcuts.cancel = "Ctrl+Win".parse().unwrap();
        assert!(p.validate().is_err(), "cancel == ptt");
        let mut p = Prefs::default();
        p.shortcuts.ptt = "Ctrl+Win+Alt+Shift".parse().unwrap();
        assert!(p.validate().unwrap_err().contains("max 3"));
    }
}
