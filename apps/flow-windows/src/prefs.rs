//! `prefs.json` in `%APPDATA%\FlowClone\` (replaces Wispr's electron-store
//! `config.json`). Unknown/missing fields fall back to defaults so old files
//! keep loading; a corrupt file is moved aside rather than silently lost.

pub use crate::core_api::DictEntry;
use crate::core_api::{CleanupLevel, SessionConfig, StyleBucket, Styles};
use crate::dictation::Policy;
use crate::hotkeys::{Bindings, Chord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "FlowClone";
pub const FILE_NAME: &str = "prefs.json";

/// `prefs.json` schema revision ([`Prefs::migrate`]). A file without
/// `prefs_rev` is 0.
/// * 1 (D22): the hands-free auto-stop default went 2000 → 1500 ms.
/// * 2 (D24): the server connect timeout default went 800 → 2500 ms.
/// * 3 (D27): chat styles default to casual, the LLM budget 1500 → 3000 ms,
///   and cleanup "None" became "No AI" (formatting rules still run): a
///   stored None moves to Verbatim, `raw_mode` to No AI ([`Prefs::migrate`]).
pub const PREFS_REV: u32 = 3;

/// The LLM budget default before D27 (rev ≤ 2): it timed out on nearly
/// every dictation (docs/findings/2026-10-23-server-llm-cleanup-cost.md).
const OLD_LLM_TIMEOUT_MS: u64 = 1500;
/// D27: Android's budget; prompt v2 + Qwen2.5-1.5B p50 ≈ 1.3 s.
pub const DEFAULT_LLM_TIMEOUT_MS: u64 = 3000;

/// D27 layer 1: chats casual (no stiff final period; Wispr Android's own
/// personal default), email and everything else formal.
pub fn default_styles() -> Styles {
    Styles {
        personal: "casual".into(),
        work: "casual".into(),
        email: "formal".into(),
        other: "formal".into(),
    }
}

/// The pre-D27 default: every bucket formal (03 §1.3, Wispr Windows).
fn all_formal(s: &Styles) -> bool {
    [&s.personal, &s.work, &s.email, &s.other]
        .iter()
        .all(|x| x.as_str() == "formal")
}

/// The hands-free auto-stop default before D22 (rev 0).
const OLD_HANDS_FREE_SILENCE_MS: u32 = 2000;

fn rev_zero() -> u32 {
    0
}

/// A prefs file without `onboarded` predates first-run setup: that user
/// already set the app up.
fn yes() -> bool {
    true
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

/// How long History keeps a dictation. JSON: `keep` (forever), `days_30`,
/// `days_7`, `day` (24 h, the pre-redesign option, kept so old files load
/// unchanged) and `never` (don't keep; wipes existing rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    #[default]
    Keep,
    #[serde(rename = "days_30")]
    Days30,
    #[serde(rename = "days_7")]
    Days7,
    Day,
    Never,
}

/// App colour scheme (design: Settings → General → Appearance). `system`
/// follows the Windows light/dark setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Local speech engine (D4/D12). Parakeet is English-only and needs a
/// build with the `parakeet` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrEngineChoice {
    #[default]
    Whisper,
    Parakeet,
}

/// The Hebrew model (D33 `languages.he`, docs/proposals/hebrew-stt.md):
/// dictations with the language set to Hebrew decode on it, everything
/// else on the default model. It loads next to the default model when its
/// folder is there (Windows keeps both, decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HebrewModel {
    /// ivrit-ai zipformer2-66m-streaming-he int8 (72 MB, sherpa online).
    #[default]
    Zipformer,
    /// Parakeet-TDT-0.6B-v3-he int8 (673 MB, the Parakeet engine).
    ParakeetHe,
    /// No Hebrew route: Hebrew dictations use the default model.
    None,
}

/// Recordings kept for retry by default (the newest history rows).
pub const DEFAULT_RETRY_AUDIO_KEEP: u32 = 20;
/// Most recordings the setting accepts.
pub const MAX_RETRY_AUDIO_KEEP: u32 = 200;

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
    /// `system` / `light` / `dark`.
    pub theme: Theme,
    /// First-run setup finished (or skipped). False only on a fresh
    /// install: a file from before this field reads as true.
    #[serde(default = "yes")]
    pub onboarded: bool,
    /// Read the focused field via UIA (local only; never leaves the machine).
    pub use_ax_context: bool,
    pub cleanup_level: CleanupLevel,
    pub raw_mode: bool,
    pub styles: Styles,
    /// "Style per app": exe stem (`slack`, lowercase, no `.exe`) or web
    /// host (`mail.example.com`) → bucket, over the built-in lists.
    pub app_buckets: BTreeMap<String, StyleBucket>,
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
    /// Retry (DECISIONS D35): keep the recordings of this many newest
    /// history rows so they can be re-transcribed (0 = keep none).
    pub retry_audio_keep: u32,
    pub max_dictation_minutes: u64,
    /// Whisper ggml model; `None` = `%APPDATA%\FlowClone\models\ggml-base.bin`.
    /// Never downloaded automatically (DECISIONS D9).
    pub model_path: Option<String>,
    /// ASR threads; `None` = core default (half the logical CPUs).
    pub asr_threads: Option<usize>,
    /// Which local engine `model_path` / `parakeet_model_dir` feed.
    pub asr_engine: AsrEngineChoice,
    /// Parakeet model directory; `None` = `%APPDATA%\FlowClone\models\`
    /// `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8`. Never downloaded.
    pub parakeet_model_dir: Option<String>,
    /// Model for dictations in Hebrew (`%APPDATA%\FlowClone\models\<folder>`,
    /// see [`HebrewModel`]); downloaded only from Settings → Speech to text.
    pub hebrew_model: HebrewModel,
    /// Deterministic spoken cleanup in the core (core-p1, D15): spoken
    /// punctuation, fillers, "scratch that", money/time/date formatting.
    pub spoken_cleanup: bool,
    /// Show the live partial transcript above the pill.
    pub live_preview: bool,
    pub live_preview_interval_ms: u32,
    /// Hands-free dictation stops by itself after this much silence
    /// following speech (D19; 300-10000 ms, 0 = never). Push-to-talk is
    /// never ended by silence.
    pub hands_free_silence_ms: u32,
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
    /// Developer settings: engine routing (local / self-hosted server).
    pub developer: crate::dev_settings::DevSettings,
    /// Transforms (docs/proposals/transforms.md): opt-in, auto-apply and
    /// the saved transforms with their shortcuts. A file without it gets
    /// the built-ins, off.
    pub transforms: crate::transforms::TransformPrefs,
    /// Schema revision ([`PREFS_REV`]); missing = 0 (a pre-D22 file).
    #[serde(default = "rev_zero")]
    pub prefs_rev: u32,
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
            theme: Theme::System,
            onboarded: false,
            use_ax_context: true,
            cleanup_level: CleanupLevel::Light,
            raw_mode: false,
            styles: default_styles(),
            app_buckets: BTreeMap::new(),
            dictionary: Vec::new(),
            paste_method: PasteMethod::CtrlV,
            restore_clipboard: true,
            paste_restore_delay_ms: None,
            type_fallback_max_chars: 200,
            retention: Retention::Keep,
            retry_audio_keep: DEFAULT_RETRY_AUDIO_KEEP,
            max_dictation_minutes: 20,
            model_path: None,
            asr_threads: None,
            asr_engine: AsrEngineChoice::Whisper,
            parakeet_model_dir: None,
            hebrew_model: HebrewModel::Zipformer,
            spoken_cleanup: true,
            live_preview: true,
            live_preview_interval_ms: 1500,
            hands_free_silence_ms: flow_core::endpoint::RECOMMENDED_SILENCE_MS,
            llm_enabled: false,
            llm_model_path: None,
            llm_lib_path: None,
            llm_timeout_ms: DEFAULT_LLM_TIMEOUT_MS,
            learn_from_edits: true,
            learn_window_ms: 45_000,
            learn_poll_ms: 1_000,
            developer: crate::dev_settings::DevSettings::default(),
            transforms: crate::transforms::TransformPrefs::default(),
            prefs_rev: PREFS_REV,
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
            Ok(mut p) => {
                p.migrate();
                (p, LoadOutcome::Loaded)
            }
            Err(_) => {
                let bak = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, &bak);
                // The app was set up before; don't replay first-run setup.
                let p = Prefs {
                    onboarded: true,
                    ..Prefs::default()
                };
                (p, LoadOutcome::Corrupt(bak))
            }
        }
    }

    /// Bring a file of an older [`PREFS_REV`] up to date. The shell saves
    /// every field, so a pre-D22 file always holds the hands-free value:
    /// exactly the old default (2000) is taken as "never changed" and moves
    /// to the new one; any other value is the user's and stays. From rev 1
    /// on, 2000 is an explicit choice and is kept.
    pub fn migrate(&mut self) {
        if self.prefs_rev < 1 && self.hands_free_silence_ms == OLD_HANDS_FREE_SILENCE_MS {
            self.hands_free_silence_ms = flow_core::endpoint::RECOMMENDED_SILENCE_MS;
        }
        // D24: 800 ms was the default and is too tight over a VPN relay.
        if self.prefs_rev < 2
            && self.developer.server.connect_timeout_ms == flow_router::LEGACY_CONNECT_TIMEOUT_MS
        {
            self.developer.server.connect_timeout_ms = flow_router::DEFAULT_CONNECT_TIMEOUT_MS;
        }
        if self.prefs_rev < 3 {
            // D27. Untouched old defaults move to the new ones; anything the
            // user chose stays. Behaviour the user had is kept: "None —
            // exactly what you said" was verbatim (now `spoken_cleanup`
            // off), `raw_mode` only ever skipped the LLM (now "No AI").
            if self.llm_timeout_ms == OLD_LLM_TIMEOUT_MS {
                self.llm_timeout_ms = DEFAULT_LLM_TIMEOUT_MS;
            }
            if all_formal(&self.styles) {
                self.styles = default_styles();
            }
            if self.cleanup_level == CleanupLevel::None {
                self.spoken_cleanup = false;
            } else if self.raw_mode {
                self.cleanup_level = CleanupLevel::None;
            }
            self.raw_mode = false;
        }
        // D36: the cleanup order holds all three engines (idempotent, so
        // not tied to a revision: an older shell may have saved over it).
        self.developer.migrate_d36();
        // Transforms: a file from before them was seeded with the
        // built-ins by serde; ids and the auto-apply choice are tidied.
        self.transforms.normalize();
        self.prefs_rev = PREFS_REV;
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
        if let Some(c) = &s.scratchpad {
            check("scratchpad", c, false)?;
        }
        if let Some(c) = &s.new_note {
            check("new-note", c, false)?;
        }
        check("command", &s.command, false)?;
        self.transforms.validate()?;
        let mut all = vec![
            &s.ptt,
            &s.hands_free,
            &s.cancel,
            &s.paste_last,
            &s.copy_last,
            &s.command,
        ];
        all.extend(s.scratchpad.as_ref());
        all.extend(s.new_note.as_ref());
        all.extend(self.transforms.all_chords());
        for (i, a) in all.iter().enumerate() {
            if all[i + 1..].contains(a) {
                return Err("shortcuts must differ from each other".into());
            }
        }
        if let Some(k) = self
            .app_buckets
            .keys()
            .find(|k| k.trim().is_empty() || k.contains('='))
        {
            return Err(format!(
                "style per app: \"{k}\" is not an app (exe name like slack.exe, or a web host like mail.example.com)"
            ));
        }
        let hf = self.hands_free_silence_ms;
        if hf != 0 && !(300..=10_000).contains(&hf) {
            return Err("hands-free auto-stop must be 0.3-10 s of silence (or off)".into());
        }
        if self.retry_audio_keep > MAX_RETRY_AUDIO_KEEP {
            return Err(format!(
                "keep recordings for retry: at most {MAX_RETRY_AUDIO_KEEP} dictations"
            ));
        }
        Ok(())
    }

    /// The shortcuts the keyboard hook serves: `shortcuts` plus the
    /// transform chords while Transforms is on.
    pub fn hotkey_bindings(&self) -> Bindings {
        let mut b = self.shortcuts.clone();
        (b.transforms, b.view_changes) = self.transforms.chords();
        b
    }

    /// `SessionConfig.endpoint_silence_ms` for a hands-free session
    /// (`None` = auto-stop off).
    pub fn hands_free_endpoint(&self) -> Option<u32> {
        (self.hands_free_silence_ms != 0).then(|| self.hands_free_silence_ms.clamp(300, 10_000))
    }

    /// Session config for a dictation started in `mode`: only hands-free
    /// sessions get the endpoint (D19). A Command Mode instruction skips
    /// the LLM cleanup pass (`raw_mode`): it is read by the command LLM,
    /// never pasted, so cleaning it would only add latency.
    pub fn session_config_for(&self, mode: crate::dictation::Mode) -> SessionConfig {
        let mut c = self.session_config();
        match mode {
            crate::dictation::Mode::Locked => c.endpoint_silence_ms = self.hands_free_endpoint(),
            crate::dictation::Mode::Command => c.raw_mode = true,
            crate::dictation::Mode::Ptt => {}
        }
        c
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
            spoken_cleanup: self.spoken_cleanup,
            // D22 "Decode while speaking": both modes; absent when off.
            segment_pause_ms: self
                .developer
                .decode_while_speaking
                .then_some(flow_core::segment::RECOMMENDED_PAUSE_MS),
            ..SessionConfig::default()
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

    /// The Parakeet model directory when that engine is chosen.
    pub fn resolved_parakeet_dir(&self, appdata: Option<&str>) -> PathBuf {
        match self.parakeet_model_dir.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => crate::core_adapter::default_parakeet_dir(appdata),
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
    fn theme_defaults_to_system_and_round_trips() {
        assert_eq!(Prefs::default().theme, Theme::System);
        let (old, _) = load_str("theme-old", r#"{"enable_sounds": true}"#);
        assert_eq!(old.theme, Theme::System, "files from before the pref");
        let (dark, _) = load_str("theme-dark", r#"{"theme": "dark"}"#);
        assert_eq!(dark.theme, Theme::Dark);
        let j = serde_json::to_value(&dark).unwrap();
        assert_eq!(j["theme"], "dark");
    }

    #[test]
    fn onboarded_false_only_for_fresh_installs() {
        let d = tmpdir("onboard-fresh");
        let (fresh, o) = Prefs::load(&d.join(FILE_NAME));
        assert_eq!(o, LoadOutcome::Missing);
        assert!(!fresh.onboarded, "no prefs file = first run");
        let (old, _) = load_str("onboard-old", r#"{"enable_sounds": true}"#);
        assert!(old.onboarded, "an upgrader skips first-run setup");
        let (mid, _) = load_str("onboard-mid", r#"{"onboarded": false}"#);
        assert!(!mid.onboarded, "setup left unfinished stays unfinished");
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let d = tmpdir("corrupt");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(FILE_NAME);
        std::fs::write(&path, "{not json").unwrap();
        let (p, o) = Prefs::load(&path);
        assert_eq!(
            p,
            Prefs {
                onboarded: true,
                ..Prefs::default()
            }
        );
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
        assert!(c.spoken_cleanup, "core-p1 default on");
        p.spoken_cleanup = false;
        assert!(!p.session_config().spoken_cleanup);
    }

    #[test]
    fn developer_section_round_trips_and_defaults() {
        let d = tmpdir("dev");
        let path = d.join(FILE_NAME);
        let mut p = Prefs::default();
        p.developer.server.host = "100.64.0.10".into();
        p.developer.token_dpapi = crate::secrets::protect("0123456789abcdef").unwrap();
        p.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"developer\""));
        assert!(!text.contains("0123456789abcdef"));
        let (q, _) = Prefs::load(&path);
        assert_eq!(q.developer, p.developer);
        assert_eq!(q.developer.token(), "0123456789abcdef");
        let old: Prefs = serde_json::from_str(r#"{"enable_sounds":true}"#).unwrap();
        assert_eq!(old.developer, crate::dev_settings::DevSettings::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn old_prefs_files_get_whisper_and_spoken_cleanup() {
        let p: Prefs = serde_json::from_str(r#"{"model_path":"C:/m/x.bin"}"#).unwrap();
        assert_eq!(p.asr_engine, AsrEngineChoice::Whisper);
        assert!(p.spoken_cleanup);
        let p: Prefs = serde_json::from_str(r#"{"asr_engine":"parakeet"}"#).unwrap();
        assert_eq!(p.asr_engine, AsrEngineChoice::Parakeet);
    }

    #[test]
    fn dict_entry_json_shape_matches_android() {
        let e: DictEntry =
            serde_json::from_str(r#"{"word":"a","replacement":"b","isSnippet":true}"#).unwrap();
        assert!(e.is_snippet);
    }

    #[test]
    fn hands_free_endpoint_only_for_locked_sessions() {
        use crate::dictation::Mode;
        let mut p = Prefs::default();
        // D22: flow_core::endpoint::RECOMMENDED_SILENCE_MS.
        assert_eq!(p.hands_free_silence_ms, 1500);
        assert_eq!(
            p.hands_free_silence_ms,
            flow_core::endpoint::RECOMMENDED_SILENCE_MS
        );
        assert_eq!(p.session_config_for(Mode::Ptt).endpoint_silence_ms, None);
        assert_eq!(
            p.session_config_for(Mode::Locked).endpoint_silence_ms,
            Some(1500)
        );
        p.hands_free_silence_ms = 0;
        assert_eq!(p.session_config_for(Mode::Locked).endpoint_silence_ms, None);
        assert_eq!(p.validate(), Ok(()));
        p.hands_free_silence_ms = 100;
        assert!(p.validate().is_err());
        p.hands_free_silence_ms = 10_001;
        assert!(p.validate().is_err());
        // Old prefs.json without the field: the new default.
        let (old, _) = load_str("nofield", "{}");
        assert_eq!(old.hands_free_silence_ms, 1500);
    }

    fn load_str(tag: &str, json: &str) -> (Prefs, LoadOutcome) {
        let d = tmpdir(tag);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(FILE_NAME);
        std::fs::write(&path, json).unwrap();
        Prefs::load(&path)
    }

    #[test]
    fn old_connect_timeout_default_migrates_once() {
        let (p, _) = load_str(
            "mig-ct-old",
            r#"{"prefs_rev":1,"developer":{"server":{"connect_timeout_ms":800}}}"#,
        );
        assert_eq!(p.developer.server.connect_timeout_ms, 2500);
        let (p, _) = load_str(
            "mig-ct-user",
            r#"{"prefs_rev":1,"developer":{"server":{"connect_timeout_ms":1200}}}"#,
        );
        assert_eq!(p.developer.server.connect_timeout_ms, 1200);
        let (p, _) = load_str(
            "mig-ct-new",
            &format!(
                r#"{{"prefs_rev":{PREFS_REV},"developer":{{"server":{{"connect_timeout_ms":800}}}}}}"#
            ),
        );
        assert_eq!(
            p.developer.server.connect_timeout_ms, 800,
            "chosen after D24"
        );
        assert_eq!(Prefs::default().developer.server.connect_timeout_ms, 2500);
    }

    #[test]
    fn old_hands_free_default_migrates_once_explicit_values_stay() {
        // A file from before D22 always holds 2000 (every field is saved),
        // so 2000 there is the old default: move it to 1500.
        let (p, o) = load_str("mig-old", r#"{"hands_free_silence_ms":2000}"#);
        assert_eq!(o, LoadOutcome::Loaded);
        assert_eq!(p.hands_free_silence_ms, 1500);
        assert_eq!(p.prefs_rev, PREFS_REV);
        // Any other pre-D22 value was chosen by the user.
        let (p, _) = load_str("mig-3000", r#"{"hands_free_silence_ms":3000}"#);
        assert_eq!(p.hands_free_silence_ms, 3000);
        let (p, _) = load_str("mig-off", r#"{"hands_free_silence_ms":0}"#);
        assert_eq!(p.hands_free_silence_ms, 0);
        // 2000 saved after the migration is an explicit choice: kept.
        let (p, _) = load_str(
            "mig-new",
            &format!(r#"{{"hands_free_silence_ms":2000,"prefs_rev":{PREFS_REV}}}"#),
        );
        assert_eq!(p.hands_free_silence_ms, 2000);
        // And it survives a save/load round trip.
        let d = tmpdir("mig-rt");
        let path = d.join(FILE_NAME);
        p.save(&path).unwrap();
        let (q, _) = Prefs::load(&path);
        assert_eq!(q.hands_free_silence_ms, 2000);
        assert_eq!(Prefs::default().prefs_rev, PREFS_REV);
    }

    #[test]
    fn decode_while_speaking_sets_segment_pause_in_every_mode() {
        use crate::dictation::Mode;
        let mut p = Prefs::default();
        assert!(p.developer.decode_while_speaking, "default on (D22)");
        for m in [Mode::Ptt, Mode::Locked] {
            assert_eq!(
                p.session_config_for(m).segment_pause_ms,
                Some(flow_core::segment::RECOMMENDED_PAUSE_MS)
            );
        }
        assert_eq!(p.session_config().segment_pause_ms, Some(800));
        p.developer.decode_while_speaking = false;
        for m in [Mode::Ptt, Mode::Locked] {
            assert_eq!(p.session_config_for(m).segment_pause_ms, None);
        }
        // Old prefs.json without the field: on.
        let old: Prefs = serde_json::from_str(r#"{"developer":{}}"#).unwrap();
        assert!(old.developer.decode_while_speaking);
    }

    #[test]
    fn d27_defaults_chats_casual_and_a_3_s_budget() {
        let p = Prefs::default();
        assert_eq!(p.styles.personal, "casual");
        assert_eq!(p.styles.work, "casual");
        assert_eq!(p.styles.email, "formal");
        assert_eq!(p.styles.other, "formal");
        assert_eq!(p.llm_timeout_ms, 3000);
        let c = p.session_config();
        assert_eq!(c.styles, default_styles());
        assert_eq!(c.llm_timeout_ms, 3000);
        assert!(p.app_buckets.is_empty());
    }

    #[test]
    fn d27_migration_moves_untouched_defaults_once() {
        let formal =
            r#""styles":{"personal":"formal","work":"formal","email":"formal","other":"formal"}"#;
        let (p, _) = load_str(
            "mig27-old",
            &format!(r#"{{"prefs_rev":2,"llm_timeout_ms":1500,{formal}}}"#),
        );
        assert_eq!(p.llm_timeout_ms, 3000);
        assert_eq!(p.styles, default_styles());
        assert_eq!(p.prefs_rev, PREFS_REV);
        // Chosen values stay.
        let (p, _) = load_str(
            "mig27-user",
            r#"{"prefs_rev":2,"llm_timeout_ms":2000,"styles":{"personal":"very_casual","work":"formal","email":"formal","other":"formal"}}"#,
        );
        assert_eq!(p.llm_timeout_ms, 2000);
        assert_eq!(p.styles.personal, "very_casual");
        assert_eq!(p.styles.work, "formal");
        // After the migration, 1500 and all-formal are explicit choices.
        let (p, _) = load_str(
            "mig27-new",
            &format!(r#"{{"prefs_rev":{PREFS_REV},"llm_timeout_ms":1500,{formal}}}"#),
        );
        assert_eq!(p.llm_timeout_ms, 1500);
        assert!(all_formal(&p.styles));
    }

    #[test]
    fn d27_migration_keeps_what_none_and_raw_mode_did() {
        // "None — exactly what you said" was verbatim: now spoken_cleanup off.
        let (p, _) = load_str("mig27-none", r#"{"prefs_rev":2,"cleanup_level":"none"}"#);
        assert_eq!(p.cleanup_level, CleanupLevel::None);
        assert!(!p.spoken_cleanup);
        assert!(!p.session_config().spoken_cleanup);
        // raw_mode only ever skipped the LLM: now "No AI", formatting on.
        let (p, _) = load_str(
            "mig27-raw",
            r#"{"prefs_rev":2,"cleanup_level":"light","raw_mode":true}"#,
        );
        assert_eq!(p.cleanup_level, CleanupLevel::None);
        assert!(!p.raw_mode);
        assert!(p.spoken_cleanup);
        // A rev-3 "No AI" keeps formatting.
        let (p, _) = load_str(
            "mig27-noai",
            &format!(r#"{{"prefs_rev":{PREFS_REV},"cleanup_level":"none"}}"#),
        );
        assert!(p.spoken_cleanup);
    }

    #[test]
    fn app_buckets_round_trip_and_reject_bad_keys() {
        let d = tmpdir("buckets");
        let path = d.join(FILE_NAME);
        let mut p = Prefs::default();
        p.app_buckets.insert("mychat".into(), StyleBucket::Personal);
        p.app_buckets
            .insert("mail.example.com".into(), StyleBucket::Email);
        p.save(&path).unwrap();
        let (q, _) = Prefs::load(&path);
        assert_eq!(q.app_buckets, p.app_buckets);
        assert_eq!(q.validate(), Ok(()));
        p.app_buckets
            .insert("rakuten viber".into(), StyleBucket::Personal);
        assert_eq!(p.validate(), Ok(()), "exe names may contain spaces");
        p.app_buckets.insert("a=b".into(), StyleBucket::Work);
        assert!(p.validate().unwrap_err().contains("a=b"));
        p.app_buckets.clear();
        p.app_buckets.insert(" ".into(), StyleBucket::Work);
        assert!(p.validate().is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn app_buckets_from_the_text_box_era_load_and_save_unchanged() {
        // The old `app = group` box stored keys as typed (`.exe`, case);
        // the picker keeps them as they are, the matcher normalises.
        let (p, o) = load_str(
            "buckets-old",
            &format!(
                r#"{{"prefs_rev":{PREFS_REV},"app_buckets":{{"MyChat.exe":"work","mail.example.com":"email","notepad":"personal"}}}}"#
            ),
        );
        assert_eq!(o, LoadOutcome::Loaded);
        let keys: Vec<&str> = p.app_buckets.keys().map(String::as_str).collect();
        assert_eq!(keys, ["MyChat.exe", "mail.example.com", "notepad"]);
        assert_eq!(p.app_buckets["MyChat.exe"], StyleBucket::Work);
        let d = tmpdir("buckets-old-save");
        let path = d.join(FILE_NAME);
        p.save(&path).unwrap();
        assert_eq!(Prefs::load(&path).0.app_buckets, p.app_buckets);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn right_alt_ptt_roundtrips_and_validates() {
        let d = tmpdir("ralt");
        let path = d.join(FILE_NAME);
        let mut p = Prefs::default();
        p.shortcuts.ptt = "RAlt".parse().unwrap();
        assert_eq!(p.validate(), Ok(()));
        p.save(&path).unwrap();
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains(r#""ptt": "Right Alt""#), "{json}");
        let (q, o) = Prefs::load(&path);
        assert_eq!(o, LoadOutcome::Loaded);
        assert_eq!(
            q.shortcuts.ptt,
            crate::hotkeys::Chord::new([crate::hotkeys::Key::RAlt])
        );
        assert_eq!(q, p);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn validate_accepts_defaults() {
        assert_eq!(Prefs::default().validate(), Ok(()));
    }

    #[test]
    fn retry_audio_keep_defaults_to_20_and_is_bounded() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p.retry_audio_keep, 20, "a file from before retry");
        let mut p = Prefs::default();
        p.retry_audio_keep = 0;
        assert_eq!(p.validate(), Ok(()), "0 = keep none");
        p.retry_audio_keep = MAX_RETRY_AUDIO_KEEP + 1;
        assert!(p.validate().unwrap_err().contains("retry"));
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
        let mut p = Prefs::default();
        p.shortcuts.scratchpad = Some("N".parse().unwrap());
        assert!(p.validate().unwrap_err().contains("scratchpad"));
        let mut p = Prefs::default();
        p.shortcuts.scratchpad = Some(p.shortcuts.copy_last.clone());
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.shortcuts.scratchpad = Some("Ctrl+Alt+N".parse().unwrap());
        assert!(p.validate().is_ok());
        p.shortcuts.new_note = Some("M".parse().unwrap());
        assert!(p.validate().unwrap_err().contains("new-note"));
        p.shortcuts.new_note = p.shortcuts.scratchpad.clone();
        assert!(p.validate().unwrap_err().contains("differ"));
        p.shortcuts.new_note = Some("Ctrl+Alt+M".parse().unwrap());
        assert!(p.validate().is_ok());
        let mut p = Prefs::default();
        p.shortcuts.command = p.shortcuts.hands_free.clone();
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.shortcuts.command = "C".parse().unwrap();
        assert!(p.validate().unwrap_err().contains("command"));
    }

    #[test]
    fn transform_chords_must_differ_from_every_other_shortcut() {
        let mut p = Prefs::default();
        p.transforms.items[0].shortcut = Some(p.shortcuts.paste_last.clone());
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.transforms.items[1].shortcut = p.transforms.items[0].shortcut.clone();
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.transforms.view_changes = p.transforms.items[0].shortcut.clone();
        assert!(p.validate().unwrap_err().contains("differ"));
        let mut p = Prefs::default();
        p.transforms.items[0].transform.instruction = " ".into();
        assert!(p.validate().unwrap_err().contains("instruction"));
    }

    #[test]
    fn hotkey_bindings_carry_transform_chords_only_when_enabled() {
        let mut p = Prefs::default();
        let b = p.hotkey_bindings();
        assert!(b.transforms.is_empty() && b.view_changes.is_none());
        assert_eq!(b.ptt, p.shortcuts.ptt);
        p.transforms.enabled = true;
        let b = p.hotkey_bindings();
        assert_eq!(b.transforms.len(), 2);
        assert_eq!(b.view_changes.unwrap().to_string(), "Win+Alt+O");
    }

    #[test]
    fn old_prefs_get_the_builtin_transforms_off_and_they_round_trip() {
        let (p, _) = load_str("transforms-old", r#"{"enable_sounds": true}"#);
        assert!(!p.transforms.enabled);
        assert_eq!(p.transforms.items.len(), 2);
        let d = tmpdir("transforms-rt");
        let path = d.join(FILE_NAME);
        let mut q = p.clone();
        q.transforms.enabled = true;
        q.transforms.auto_apply = Some("polish".into());
        q.transforms.items[1].shortcut = None;
        q.save(&path).unwrap();
        assert_eq!(Prefs::load(&path).0, q);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn command_default_and_old_prefs_without_it() {
        let p = Prefs::default();
        assert_eq!(p.shortcuts.command.to_string(), "Ctrl+Win+Alt");
        let q: Prefs = serde_json::from_str(r#"{"shortcuts":{"ptt":"Ctrl+Shift"}}"#).unwrap();
        assert_eq!(q.shortcuts.command, p.shortcuts.command);
        use crate::dictation::Mode;
        assert!(
            p.session_config_for(Mode::Command).raw_mode,
            "no LLM on the instruction"
        );
        assert!(!p.session_config_for(Mode::Ptt).raw_mode);
    }
}
