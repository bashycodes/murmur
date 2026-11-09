//! App classification by app id and web host, from the shared app groups
//! (`crate::app_groups`, `data/app-groups.json`).

use crate::app_groups as groups;
use crate::config::{AppCategory, Platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppType {
    Ai,
    Email,
    Other,
}

impl AppType {
    pub fn as_str(self) -> &'static str {
        match self {
            AppType::Ai => "ai",
            AppType::Email => "email",
            AppType::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Personal,
    Work,
    Email,
    Other,
}

impl Bucket {
    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Personal => "personal",
            Bucket::Work => "work",
            Bucket::Email => "email",
            Bucket::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppInfo {
    pub app_type: AppType,
    pub is_messaging: bool,
    pub bucket: Bucket,
    pub is_terminal: bool,
    pub is_notion: bool,
    pub is_flow_app: bool,
    /// Cursor / Windsurf (IDE @file tagging; VS Code excluded).
    pub is_tagging_ide: bool,
}

impl AppInfo {
    /// Coarse category for LLM prompting / cleanup skipping.
    pub fn category(&self) -> AppCategory {
        if self.is_terminal {
            AppCategory::Terminal
        } else if self.app_type == AppType::Ai {
            AppCategory::AiPrompt
        } else {
            match self.bucket {
                Bucket::Email => AppCategory::Email,
                Bucket::Work if self.is_messaging => AppCategory::WorkChat,
                Bucket::Personal => AppCategory::PersonalChat,
                _ => AppCategory::Other,
            }
        }
    }
}

fn host_of(url: &str) -> String {
    let u = url.trim().to_lowercase();
    let u = u.split("://").last().unwrap_or("");
    let h = u.split(['/', '?', '#']).next().unwrap_or("");
    h.strip_prefix("www.").unwrap_or(h).to_string()
}

pub fn classify(platform: Platform, bundle: &str, url: &str) -> AppInfo {
    let is_notion = bundle.starts_with("notion.id")
        || url.starts_with("notion.so")
        || bundle == "com.notion.id";
    let host = host_of(url);
    match platform {
        Platform::Windows => {
            let id = windows_id(bundle);
            let bundle = id.as_str();
            let g = |n: &str| groups::windows(n).matches(bundle, &host);
            let is_terminal = groups::windows("terminal").app(bundle);
            let app_type = if g("ai") {
                AppType::Ai
            } else if is_terminal {
                AppType::Other
            } else if g("email") {
                AppType::Email
            } else {
                AppType::Other
            };
            let is_messaging = g("messaging");
            let bucket = if app_type == AppType::Email {
                Bucket::Email
            } else if g("work") {
                Bucket::Work
            } else if is_messaging {
                Bucket::Personal
            } else {
                Bucket::Other
            };
            AppInfo {
                app_type,
                is_messaging,
                bucket,
                is_terminal,
                is_notion,
                is_flow_app: groups::windows("flow").app(bundle),
                is_tagging_ide: groups::windows("tagging_ide").app(bundle),
            }
        }
        Platform::Android => {
            let g = |n: &str| groups::android(n).matches(bundle, &host);
            let bucket = if g("personal") {
                Bucket::Personal
            } else if g("email") {
                Bucket::Email
            } else if g("work") {
                Bucket::Work
            } else {
                Bucket::Other
            };
            AppInfo {
                app_type: if bucket == Bucket::Email {
                    AppType::Email
                } else if g("ai") {
                    AppType::Ai
                } else {
                    AppType::Other
                },
                is_messaging: bucket == Bucket::Personal || g("work_chat"),
                bucket,
                is_terminal: groups::android("terminal").app(bundle),
                is_notion,
                is_flow_app: false,
                is_tagging_ide: false,
            }
        }
    }
}

/// A Windows process name compared the way the shell does: lower-case,
/// without a trailing `.exe`.
fn windows_id(app_id: &str) -> String {
    let lower = app_id.trim().to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(s) => s.to_string(),
        None => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: &str) -> AppInfo {
        classify(Platform::Windows, id, "")
    }

    #[test]
    fn windows_ids_match_without_exe_in_any_case() {
        // Fix: the id reached the groups with its `.exe`.
        for id in ["Outlook.EXE", "outlook.exe", "OUTLOOK"] {
            assert_eq!(win(id).app_type, AppType::Email, "{id}");
            assert_eq!(win(id).category(), AppCategory::Email, "{id}");
        }
        assert!(win("slack.exe").is_messaging);
        assert!(win("Slack.EXE").is_messaging);
        assert!(win("murmur.exe").is_flow_app);
        assert!(win("Flow-Windows.exe").is_flow_app);
        assert!(win("Cursor.exe").is_tagging_ide);
        assert!(!win("notepad.exe").is_messaging);
        assert_eq!(win("notepad.exe").category(), AppCategory::Other);
    }

    #[test]
    fn notion_is_the_exact_host_or_the_android_package() {
        assert!(classify(Platform::Windows, "chrome.exe", "notion.so").is_notion);
        assert!(classify(Platform::Windows, "chrome.exe", "notion.so/page").is_notion);
        assert!(classify(Platform::Android, "notion.id", "").is_notion);
        for (id, host) in [
            ("chrome.exe", "www.notion.so"),
            ("chrome.exe", "Notion.so"),
            ("chrome.exe", "team.notion.site"),
            ("Notion.exe", ""),
        ] {
            assert!(
                !classify(Platform::Windows, id, host).is_notion,
                "{id} {host}"
            );
        }
        assert!(!classify(Platform::Android, "dev.flowclone", "").is_flow_app);
    }
}
