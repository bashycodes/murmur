//! Focused-field context: platform-independent shaping (truncation,
//! password suppression, app → style bucket) + UIA capture on Windows.

#[cfg(windows)]
pub mod uia;

use crate::core_api::{AppKind, FieldContext, StyleBucket};

pub const MAX_FIELD_CHARS: usize = 500;

/// Raw facts gathered from the OS at dictation start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawSnapshot {
    pub exe_path: String,
    pub window_title: String,
    pub url: Option<String>,
    pub before: String,
    pub selected: String,
    pub after: String,
    pub is_password: bool,
}

/// Last `n` chars (text nearest the caret).
pub fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// First `n` chars.
pub fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `C:\Program Files\Slack\slack.exe` → `slack.exe`.
pub fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// `https://mail.google.com/mail/u/0` → `mail.google.com`.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest.split_once("://").map(|(_, r)| r).unwrap_or(rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = authority.split(':').next().unwrap_or("");
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// The app lists (windows-research `findings/data/app-lists.json`,
/// copied verbatim into `data/`). Process names are exe stems, lowercase.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct AppLists {
    ai_apps: Vec<String>,
    email_apps: Vec<String>,
    document_apps: Vec<String>,
    messaging_apps: Vec<String>,
    work_messaging_apps: Vec<String>,
    windows_ai_process_names: Vec<String>,
    windows_email_process_names: Vec<String>,
    windows_messaging_process_names: Vec<String>,
    windows_terminal_process_names: Vec<String>,
}

fn lists() -> &'static AppLists {
    static L: std::sync::OnceLock<AppLists> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        serde_json::from_str(include_str!("../../data/app-lists.json"))
            .expect("data/app-lists.json is valid")
    })
}

/// Windows process names the lists miss (new Outlook, new Teams).
const EXTRA_EMAIL_PROCS: &[&str] = &["olk", "hxoutlook"];
const EXTRA_WORK_PROCS: &[&str] = &["ms-teams"];

fn has(list: &[String], s: &str) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(s))
}

/// App category. Precedence: work chat > email > messaging > AI > terminal
/// > document > other.
pub fn app_kind(exe: &str, host: Option<&str>) -> AppKind {
    let l = lists();
    if let Some(h) = host {
        let h = h.to_ascii_lowercase();
        let h = h.strip_prefix("www.").unwrap_or(&h);
        return if has(&l.work_messaging_apps, h) {
            AppKind::WorkMessaging
        } else if has(&l.email_apps, h) {
            AppKind::Email
        } else if has(&l.messaging_apps, h) {
            AppKind::Messaging
        } else if has(&l.ai_apps, h) {
            AppKind::Ai
        } else if has(&l.document_apps, h) {
            AppKind::Document
        } else {
            AppKind::Other
        };
    }
    let lower = exe.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    if has(&l.work_messaging_apps, stem) || EXTRA_WORK_PROCS.contains(&stem) {
        AppKind::WorkMessaging
    } else if has(&l.windows_email_process_names, stem) || EXTRA_EMAIL_PROCS.contains(&stem) {
        AppKind::Email
    } else if has(&l.windows_messaging_process_names, stem) {
        AppKind::Messaging
    } else if has(&l.windows_ai_process_names, stem) {
        AppKind::Ai
    } else if has(&l.windows_terminal_process_names, stem) {
        AppKind::Terminal
    } else if has(&l.document_apps, stem) {
        AppKind::Document
    } else {
        AppKind::Other
    }
}

const WORK_TITLES: &[&str] = &["slack", "microsoft teams"];
const EMAIL_TITLES: &[&str] = &["gmail", "outlook", "proton mail"];
const PERSONAL_TITLES: &[&str] = &["whatsapp", "messenger", "telegram"];

/// Style bucket: work chat → work, email → email, other messaging →
/// personal, everything else (AI apps, terminals, documents) → other.
pub fn bucket_of(kind: AppKind) -> StyleBucket {
    match kind {
        AppKind::WorkMessaging => StyleBucket::Work,
        AppKind::Email => StyleBucket::Email,
        AppKind::Messaging => StyleBucket::Personal,
        _ => StyleBucket::Other,
    }
}

/// App → style bucket, with a tab-title fallback for browsers whose address
/// bar couldn't be read.
pub fn classify(exe: &str, host: Option<&str>, title: &str) -> StyleBucket {
    let kind = app_kind(exe, host);
    if kind != AppKind::Other || host.is_some() {
        return bucket_of(kind);
    }
    // "… - Gmail - Google Chrome": match " - "-separated segments.
    let segs: Vec<String> = title
        .split(" - ")
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    let any = |list: &[&str]| segs.iter().any(|s| list.contains(&s.as_str()));
    if any(EMAIL_TITLES) {
        StyleBucket::Email
    } else if any(WORK_TITLES) {
        StyleBucket::Work
    } else if any(PERSONAL_TITLES) {
        StyleBucket::Personal
    } else {
        StyleBucket::Other
    }
}

/// Capitalised words from the window title, used as ASR hotwords.
pub fn title_words(title: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in title.split(|c: char| !c.is_alphanumeric()) {
        let starts_upper = w.chars().next().is_some_and(|c| c.is_uppercase());
        if starts_upper && w.chars().count() >= 3 && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
            if out.len() == 20 {
                break;
            }
        }
    }
    out
}

pub fn build_field_context(raw: &RawSnapshot) -> FieldContext {
    let app_id = exe_name(&raw.exe_path);
    let url_host = raw.url.as_deref().and_then(url_host);
    let app_kind = app_kind(&app_id, url_host.as_deref());
    let app_category = classify(&app_id, url_host.as_deref(), &raw.window_title);
    let (before, selected, after) = if raw.is_password {
        (String::new(), String::new(), String::new())
    } else {
        (
            tail_chars(&raw.before, MAX_FIELD_CHARS),
            head_chars(&raw.selected, MAX_FIELD_CHARS),
            head_chars(&raw.after, MAX_FIELD_CHARS),
        )
    };
    FieldContext {
        app_id,
        url_host,
        app_category,
        app_kind,
        before,
        selected,
        after,
        screen_words: title_words(&raw.window_title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_and_head_are_char_based() {
        assert_eq!(tail_chars("héllo", 3), "llo");
        assert_eq!(tail_chars("hi", 10), "hi");
        assert_eq!(head_chars("héllo", 2), "hé");
        assert_eq!(head_chars("", 2), "");
        let long = "x".repeat(800);
        assert_eq!(tail_chars(&long, MAX_FIELD_CHARS).chars().count(), 500);
    }

    #[test]
    fn exe_name_from_paths() {
        assert_eq!(exe_name(r"C:\Program Files\Slack\slack.exe"), "slack.exe");
        assert_eq!(exe_name("C:/x/OUTLOOK.EXE"), "outlook.exe");
        assert_eq!(exe_name("notepad.exe"), "notepad.exe");
    }

    #[test]
    fn url_host_parsing() {
        assert_eq!(
            url_host("https://mail.google.com/mail/u/0").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(
            url_host("app.slack.com/client").as_deref(),
            Some("app.slack.com")
        );
        assert_eq!(
            url_host("http://user@Example.com:8080/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(url_host(""), None);
    }

    #[test]
    fn app_kinds_from_lists() {
        assert_eq!(
            app_kind("code.exe", None),
            AppKind::Ai,
            "VS Code is an AI app"
        );
        assert_eq!(app_kind("Cursor.exe", None), AppKind::Ai);
        assert_eq!(app_kind("WindowsTerminal.exe", None), AppKind::Terminal);
        assert_eq!(app_kind("pwsh.exe", None), AppKind::Terminal);
        assert_eq!(
            app_kind("warp.exe", None),
            AppKind::Ai,
            "AI wins over terminal"
        );
        assert_eq!(app_kind("slack.exe", None), AppKind::WorkMessaging);
        assert_eq!(app_kind("discord.exe", None), AppKind::Messaging);
        assert_eq!(app_kind("outlook.exe", None), AppKind::Email);
        assert_eq!(app_kind("notepad.exe", None), AppKind::Document);
        assert_eq!(app_kind("calc.exe", None), AppKind::Other);
        assert_eq!(app_kind("chrome.exe", Some("claude.ai")), AppKind::Ai);
        assert_eq!(
            app_kind("chrome.exe", Some("docs.google.com")),
            AppKind::Document
        );
    }

    #[test]
    fn terminals_and_ai_apps_use_other_style() {
        assert_eq!(
            classify("windowsterminal.exe", None, ""),
            StyleBucket::Other
        );
        assert_eq!(classify("code.exe", None, ""), StyleBucket::Other);
    }

    #[test]
    fn classify_native_apps() {
        assert_eq!(classify("slack.exe", None, ""), StyleBucket::Work);
        assert_eq!(classify("ms-teams.exe", None, ""), StyleBucket::Work);
        assert_eq!(classify("outlook.exe", None, ""), StyleBucket::Email);
        assert_eq!(classify("olk.exe", None, ""), StyleBucket::Email);
        assert_eq!(classify("whatsapp.exe", None, ""), StyleBucket::Personal);
        assert_eq!(classify("telegram.exe", None, ""), StyleBucket::Personal);
        assert_eq!(classify("notepad.exe", None, ""), StyleBucket::Other);
    }

    #[test]
    fn classify_web_apps_by_host_then_title() {
        assert_eq!(
            classify("chrome.exe", Some("mail.google.com"), ""),
            StyleBucket::Email
        );
        assert_eq!(
            classify("msedge.exe", Some("app.slack.com"), ""),
            StyleBucket::Work
        );
        assert_eq!(
            classify("firefox.exe", Some("web.whatsapp.com"), ""),
            StyleBucket::Personal
        );
        assert_eq!(
            classify(
                "chrome.exe",
                None,
                "Inbox (3) - me@x.com - Gmail - Google Chrome"
            ),
            StyleBucket::Email
        );
        assert_eq!(
            classify("chrome.exe", Some("example.com"), "Example"),
            StyleBucket::Other
        );
    }

    #[test]
    fn title_words_extracts_capitalised_tokens() {
        assert_eq!(
            title_words("Re: Project Falcon budget - Outlook"),
            vec!["Project", "Falcon", "Outlook"]
        );
        assert!(title_words("").is_empty());
    }

    #[test]
    fn password_fields_yield_no_text() {
        let raw = RawSnapshot {
            exe_path: r"C:\a\chrome.exe".into(),
            window_title: "Login".into(),
            before: "secret".into(),
            selected: "s".into(),
            after: "t".into(),
            is_password: true,
            ..Default::default()
        };
        let c = build_field_context(&raw);
        assert_eq!(c.app_id, "chrome.exe");
        assert!(c.before.is_empty() && c.selected.is_empty() && c.after.is_empty());
    }

    #[test]
    fn field_context_truncates_around_caret() {
        let raw = RawSnapshot {
            exe_path: "slack.exe".into(),
            url: None,
            before: format!("{}END", "a".repeat(600)),
            selected: "b".repeat(600),
            after: format!("START{}", "c".repeat(600)),
            ..Default::default()
        };
        let c = build_field_context(&raw);
        assert_eq!(c.app_category, StyleBucket::Work);
        assert_eq!(c.before.chars().count(), 500);
        assert!(c.before.ends_with("END"));
        assert!(c.after.starts_with("START"));
        assert_eq!(c.after.chars().count(), 500);
        assert_eq!(c.selected.chars().count(), 500);
    }

    #[test]
    fn field_context_carries_url_host() {
        let raw = RawSnapshot {
            exe_path: "chrome.exe".into(),
            url: Some("https://mail.google.com/x".into()),
            ..Default::default()
        };
        let c = build_field_context(&raw);
        assert_eq!(c.url_host.as_deref(), Some("mail.google.com"));
        assert_eq!(c.app_category, StyleBucket::Email);
    }
}
