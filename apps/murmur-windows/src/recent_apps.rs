//! Recently dictated-into apps, for the "Style per app" picker.
//!
//! Only exe names are kept (`slack.exe`): no window titles, no text, no
//! times. Newest first, at most [`MAX_RECENT`]. Stored in
//! `%APPDATA%\FlowClone\recent_apps.json`, written atomically like
//! `prefs.json`, and only when the order changes (dictating into the same
//! app again writes nothing).

use crate::context::{app_kind, bucket_key, bucket_of, built_in_exe_buckets, exe_name};
use crate::core_api::StyleBucket;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "recent_apps.json";
pub const MAX_RECENT: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentApps {
    /// Lowercase exe file names, newest first, unique by [`bucket_key`].
    pub apps: Vec<String>,
}

impl RecentApps {
    /// Move `exe` (a name or a full path) to the front. Returns whether the
    /// list changed.
    pub fn record(&mut self, exe: &str) -> bool {
        let exe = exe_name(exe.trim());
        let key = bucket_key(&exe);
        if key.is_empty() || key.contains('=') {
            return false;
        }
        if self.apps.first().is_some_and(|a| bucket_key(a) == key) {
            return false;
        }
        self.apps.retain(|a| bucket_key(a) != key);
        self.apps.insert(0, exe);
        self.apps.truncate(MAX_RECENT);
        true
    }

    pub fn clear(&mut self) -> bool {
        let had = !self.apps.is_empty();
        self.apps.clear();
        had
    }
}

/// [`RecentApps`] bound to a file (`None` = in memory only, for tests).
#[derive(Debug, Default)]
pub struct RecentAppStore {
    path: Option<PathBuf>,
    pub data: RecentApps,
}

pub fn default_path(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join(FILE_NAME)
}

impl RecentAppStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load (a missing or unreadable file = empty; entries re-checked).
    pub fn open(path: &Path) -> Self {
        let stored: RecentApps = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // Keep the first (newest) spelling of each app; drop junk.
        let mut data = RecentApps::default();
        for a in &stored.apps {
            let exe = exe_name(a.trim());
            let key = bucket_key(&exe);
            if !key.is_empty()
                && !key.contains('=')
                && !data.apps.iter().any(|x| bucket_key(x) == key)
            {
                data.apps.push(exe);
            }
        }
        data.apps.truncate(MAX_RECENT);
        RecentAppStore {
            path: Some(path.to_path_buf()),
            data,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.data).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    /// Record `exe`; written to disk when it changed and `persist` (history
    /// retention "Never" keeps the list in memory only).
    pub fn record(&mut self, exe: &str, persist: bool) {
        if self.data.record(exe) && persist {
            if let Err(e) = self.save() {
                eprintln!("[recent_apps] save failed: {e}");
            }
        }
    }
}

/// One app the picker can offer: `key` is what goes into
/// `prefs.app_buckets`, `default_bucket` the built-in group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppChoice {
    pub key: String,
    pub exe: String,
    pub name: Option<String>,
    pub default_bucket: StyleBucket,
}

impl AppChoice {
    pub fn of(exe: &str) -> AppChoice {
        let exe = exe_name(exe.trim());
        let key = bucket_key(&exe);
        AppChoice {
            name: friendly_name(&key).map(str::to_string),
            default_bucket: bucket_of(app_kind(&exe, None)),
            key,
            exe,
        }
    }
}

/// What the Writing style page shows: the recent apps and the built-in
/// exe → group mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StyleChoices {
    pub recent: Vec<AppChoice>,
    pub defaults: Vec<AppChoice>,
}

pub fn style_choices(recent: &RecentApps) -> StyleChoices {
    StyleChoices {
        recent: recent.apps.iter().map(|a| AppChoice::of(a)).collect(),
        defaults: built_in_exe_buckets()
            .into_iter()
            .map(|(stem, b)| AppChoice {
                name: friendly_name(&stem).map(str::to_string),
                exe: format!("{stem}.exe"),
                key: stem,
                default_bucket: b,
            })
            .collect(),
    }
}

/// Display names for common Windows exe stems (no file-version lookup).
pub fn friendly_name(stem: &str) -> Option<&'static str> {
    Some(match stem {
        "slack" => "Slack",
        "ms-teams" | "msteams" | "teams" => "Microsoft Teams",
        "outlook" => "Outlook (classic)",
        "olk" => "Outlook",
        "hxoutlook" => "Mail",
        "thunderbird" => "Thunderbird",
        "whatsapp" | "whatsapp.root" => "WhatsApp",
        "telegram" => "Telegram",
        "signal" => "Signal",
        "discord" => "Discord",
        "messenger" => "Messenger",
        "zoom" => "Zoom",
        "chrome" => "Google Chrome",
        "msedge" => "Microsoft Edge",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "winword" => "Word",
        "excel" => "Excel",
        "powerpnt" => "PowerPoint",
        "onenote" => "OneNote",
        "notepad" => "Notepad",
        "code" => "Visual Studio Code",
        "cursor" => "Cursor",
        "notion" => "Notion",
        "obsidian" => "Obsidian",
        "windowsterminal" => "Windows Terminal",
        "explorer" => "File Explorer",
        "chatgpt" => "ChatGPT",
        "claude" => "Claude",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_dedupes_moves_to_front_and_caps() {
        let mut r = RecentApps::default();
        assert!(r.record(r"C:\Program Files\Slack\Slack.exe"));
        assert!(r.record("OUTLOOK.EXE"));
        assert_eq!(r.apps, ["outlook.exe", "slack.exe"]);
        assert!(!r.record("outlook.exe"), "already newest: no write");
        assert!(r.record("slack"), "same app without .exe");
        assert_eq!(r.apps, ["slack", "outlook.exe"]);
        assert!(!r.record("  "));
        assert!(!r.record("a=b.exe"));
        for i in 0..(MAX_RECENT + 3) {
            r.record(&format!("app{i}.exe"));
        }
        assert_eq!(r.apps.len(), MAX_RECENT);
        assert_eq!(r.apps[0], format!("app{}.exe", MAX_RECENT + 2));
        assert!(r.clear());
        assert!(!r.clear());
    }

    #[test]
    fn store_round_trips_and_skips_unchanged_writes() {
        let d = std::env::temp_dir().join(format!("flow-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE_NAME);
        let mut st = RecentAppStore::open(&p);
        assert!(st.data.apps.is_empty());
        st.record("slack.exe", false);
        assert!(!p.exists(), "persist off = memory only");
        st.record("olk.exe", true);
        assert!(p.exists());
        let back = RecentAppStore::open(&p);
        assert_eq!(back.data, st.data);
        assert_eq!(back.data.apps, ["olk.exe", "slack.exe"]);
        // A hand-edited file with junk and duplicates loads cleaned up.
        std::fs::write(
            &p,
            r#"{"apps":["Slack.exe","","slack","x=y","notepad.exe"]}"#,
        )
        .unwrap();
        assert_eq!(
            RecentAppStore::open(&p).data.apps,
            ["slack.exe", "notepad.exe"]
        );
        std::fs::write(&p, "not json").unwrap();
        assert!(RecentAppStore::open(&p).data.apps.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn choices_carry_key_name_and_default_group() {
        let mut r = RecentApps::default();
        r.record("notepad.exe");
        r.record("Slack.exe");
        let c = style_choices(&r);
        assert_eq!(
            c.recent[0],
            AppChoice {
                key: "slack".into(),
                exe: "slack.exe".into(),
                name: Some("Slack".into()),
                default_bucket: StyleBucket::Work,
            }
        );
        assert_eq!(c.recent[1].default_bucket, StyleBucket::Other);
        assert_eq!(c.recent[1].name.as_deref(), Some("Notepad"));
        let olk = c.defaults.iter().find(|a| a.key == "olk").unwrap();
        assert_eq!(olk.default_bucket, StyleBucket::Email);
        assert_eq!(olk.exe, "olk.exe");
        assert!(c
            .defaults
            .iter()
            .all(|a| a.default_bucket != StyleBucket::Other));
        let json = serde_json::to_value(&c.recent[0]).unwrap();
        assert_eq!(json["default_bucket"], "work");
    }
}
