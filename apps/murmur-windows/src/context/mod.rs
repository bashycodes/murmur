//! Focused-field context: platform-independent shaping (truncation,
//! password suppression, app → style bucket) + UIA capture on Windows.

#[cfg(windows)]
pub mod uia;

use crate::core_api::{AppKind, FieldContext, StyleBucket, Styles};

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

/// App category from the Windows app groups (murmur-core `app_groups`,
/// `data/app-groups.json`; exe stems exact, hosts with subdomains).
/// Precedence: work chat > email > messaging > AI > terminal > document > other.
pub fn app_kind(exe: &str, host: Option<&str>) -> AppKind {
    if let Some(h) = host {
        let is = |n: &str| murmur_core::app_groups::windows(n).host(h);
        return if is("work") {
            AppKind::WorkMessaging
        } else if is("email") {
            AppKind::Email
        } else if is("messaging") {
            AppKind::Messaging
        } else if is("ai") {
            AppKind::Ai
        } else if is("document") {
            AppKind::Document
        } else {
            AppKind::Other
        };
    }
    let lower = exe.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    let is = |n: &str| murmur_core::app_groups::windows(n).app(stem);
    if is("work") {
        AppKind::WorkMessaging
    } else if is("email") {
        AppKind::Email
    } else if is("messaging") {
        AppKind::Messaging
    } else if is("ai") {
        AppKind::Ai
    } else if is("terminal") {
        AppKind::Terminal
    } else if is("document") {
        AppKind::Document
    } else {
        AppKind::Other
    }
}

/// Built-in exe stems that land in a style bucket other than "other"
/// (work chat, email, personal chat), sorted by stem. Shown read-only in
/// the "Style per app" table so the user sees why an app is in a group.
/// Host and macOS bundle-id entries are left out (anything with a dot).
pub fn built_in_exe_buckets() -> Vec<(String, StyleBucket)> {
    use murmur_core::app_groups::windows_exact;
    let mut stems: Vec<String> = windows_exact("work")
        .iter()
        .chain(windows_exact("email"))
        .chain(windows_exact("messaging"))
        .filter(|s| !s.is_empty() && !s.contains('.'))
        .cloned()
        .collect();
    stems.sort();
    stems.dedup();
    stems
        .into_iter()
        .map(|s| {
            let b = bucket_of(app_kind(&s, None));
            (s, b)
        })
        .filter(|(_, b)| *b != StyleBucket::Other)
        .collect()
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

/// The bucket murmur-core picks the style by for `ctx` (mirrors
/// `core_adapter::to_core_category`: the app kind wins, AI apps, terminals
/// and documents use "other"). Debug log `style`.
pub fn effective_bucket(ctx: &FieldContext) -> StyleBucket {
    match ctx.app_kind {
        AppKind::Email => StyleBucket::Email,
        AppKind::WorkMessaging => StyleBucket::Work,
        AppKind::Messaging => StyleBucket::Personal,
        AppKind::Ai | AppKind::Document | AppKind::Terminal => StyleBucket::Other,
        AppKind::Other => ctx.app_category,
    }
}

/// The style sent for `ctx`'s bucket; no context (UIA off / not read yet)
/// = the core's default category, "other".
pub fn style_for<'a>(styles: &'a Styles, ctx: Option<&FieldContext>) -> &'a str {
    match ctx.map_or(StyleBucket::Other, effective_bucket) {
        StyleBucket::Personal => &styles.personal,
        StyleBucket::Work => &styles.work,
        StyleBucket::Email => &styles.email,
        StyleBucket::Other => &styles.other,
    }
}

/// `prefs.app_buckets` key for an exe (`Slack.exe` → `slack`) or a host
/// (`www.Mail.Example.com` → `mail.example.com`).
pub fn bucket_key(s: &str) -> String {
    let l = s.trim().to_ascii_lowercase();
    let l = l.strip_suffix(".exe").unwrap_or(&l);
    l.strip_prefix("www.").unwrap_or(l).to_string()
}

/// "Style per app": the user's bucket for this exe or web host replaces the
/// built-in one. The app kind follows it (the core prefers the kind), except
/// that a move to "other" keeps an AI / terminal / document kind, whose
/// formatting rules still apply.
pub fn apply_bucket_override(
    ctx: &mut FieldContext,
    map: &std::collections::BTreeMap<String, StyleBucket>,
) {
    if map.is_empty() {
        return;
    }
    let find = |k: &str| {
        let k = bucket_key(k);
        map.iter()
            .find(|(m, _)| bucket_key(m) == k)
            .map(|(_, b)| *b)
    };
    let hit = ctx
        .url_host
        .as_deref()
        .and_then(find)
        .or_else(|| find(&ctx.app_id));
    let Some(b) = hit else { return };
    ctx.app_category = b;
    ctx.app_kind = match b {
        StyleBucket::Personal => AppKind::Messaging,
        StyleBucket::Work => AppKind::WorkMessaging,
        StyleBucket::Email => AppKind::Email,
        StyleBucket::Other => match ctx.app_kind {
            k @ (AppKind::Ai | AppKind::Terminal | AppKind::Document) => k,
            _ => AppKind::Other,
        },
    };
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

/// What UI Automation said about the focused control's selection
/// (Command Mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiaSelection {
    /// TextPattern selection text (empty = nothing selected).
    Text(String),
    Password,
    /// No TextPattern (or no focused element): ask the app via Ctrl+C.
    Unsupported,
}

/// Refused selection: never copy out of / paste into a password field.
pub const PASSWORD_SELECTION_MSG: &str = "Command Mode doesn't work in password fields";

/// Command Mode selection: trust UI Automation when the control speaks
/// TextPattern (an empty selection there = speak-to-generate, no Ctrl+C
/// that some editors turn into "copy the whole line"); otherwise, and when
/// UIA timed out (`None`), fall back to `copy` (Ctrl+C with the clipboard
/// restored).
pub fn resolve_selection(
    uia: Option<UiaSelection>,
    copy: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    match uia {
        Some(UiaSelection::Text(t)) if t.is_empty() => Ok(None),
        Some(UiaSelection::Text(t)) => Ok(Some(t)),
        Some(UiaSelection::Password) => Err(PASSWORD_SELECTION_MSG.into()),
        Some(UiaSelection::Unsupported) | None => copy(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_prefers_uia_and_falls_back_to_copy() {
        let never = || -> Result<Option<String>, String> { panic!("no Ctrl+C wanted") };
        let copy = || Ok(Some("copied".to_string()));
        assert_eq!(
            resolve_selection(Some(UiaSelection::Text("sel".into())), never),
            Ok(Some("sel".into()))
        );
        assert_eq!(
            resolve_selection(Some(UiaSelection::Text(String::new())), never),
            Ok(None),
            "TextPattern says nothing is selected: generate, no Ctrl+C"
        );
        assert_eq!(
            resolve_selection(Some(UiaSelection::Password), never),
            Err(PASSWORD_SELECTION_MSG.into())
        );
        assert_eq!(
            resolve_selection(Some(UiaSelection::Unsupported), copy),
            Ok(Some("copied".into()))
        );
        assert_eq!(
            resolve_selection(None, copy),
            Ok(Some("copied".into())),
            "UIA timed out"
        );
    }

    fn ctx(exe: &str, host: Option<&str>) -> FieldContext {
        build_field_context(&RawSnapshot {
            exe_path: format!(r"C:\Apps\{exe}"),
            url: host.map(|h| format!("https://{h}/x")),
            ..RawSnapshot::default()
        })
    }

    #[test]
    fn built_in_exe_list_matches_the_classifier() {
        let all = built_in_exe_buckets();
        let get = |s: &str| all.iter().find(|(k, _)| k == s).map(|(_, b)| *b);
        assert_eq!(
            get("slack"),
            Some(StyleBucket::Work),
            "work beats messaging"
        );
        assert_eq!(get("ms-teams"), Some(StyleBucket::Work));
        assert_eq!(get("outlook"), Some(StyleBucket::Email));
        assert_eq!(get("olk"), Some(StyleBucket::Email));
        assert_eq!(get("whatsapp"), Some(StyleBucket::Personal));
        assert_eq!(get("cursor"), None, "AI apps are 'other'");
        assert!(all.iter().all(|(k, _)| !k.contains('.')), "no hosts");
        let mut keys: Vec<&String> = all.iter().map(|(k, _)| k).collect();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "unique and sorted");
        for (k, b) in &all {
            assert_eq!(effective_bucket(&ctx(&format!("{k}.exe"), None)), *b, "{k}");
        }
    }

    #[test]
    fn built_in_buckets_for_the_common_apps() {
        for (exe, want) in [
            ("WhatsApp.exe", StyleBucket::Personal),
            ("Telegram.exe", StyleBucket::Personal),
            ("Signal.exe", StyleBucket::Personal),
            ("Discord.exe", StyleBucket::Personal),
            ("Messenger.exe", StyleBucket::Personal),
            ("slack.exe", StyleBucket::Work),
            ("ms-teams.exe", StyleBucket::Work),
            ("OUTLOOK.EXE", StyleBucket::Email),
            ("olk.exe", StyleBucket::Email),
            ("notepad.exe", StyleBucket::Other),
        ] {
            assert_eq!(effective_bucket(&ctx(exe, None)), want, "{exe}");
        }
        for (host, want) in [
            ("web.whatsapp.com", StyleBucket::Personal),
            ("instagram.com", StyleBucket::Personal),
            ("messages.google.com", StyleBucket::Personal),
            ("app.slack.com", StyleBucket::Work),
            ("mail.google.com", StyleBucket::Email),
            ("docs.google.com", StyleBucket::Other),
        ] {
            assert_eq!(
                effective_bucket(&ctx("chrome.exe", Some(host))),
                want,
                "{host}"
            );
        }
    }

    #[test]
    fn style_follows_the_bucket_and_the_d27_defaults() {
        let s = crate::prefs::default_styles();
        assert_eq!(style_for(&s, Some(&ctx("WhatsApp.exe", None))), "casual");
        assert_eq!(style_for(&s, Some(&ctx("slack.exe", None))), "casual");
        assert_eq!(style_for(&s, Some(&ctx("outlook.exe", None))), "formal");
        assert_eq!(
            style_for(&s, Some(&ctx("code.exe", None))),
            "formal",
            "AI app = other"
        );
        assert_eq!(style_for(&s, None), "formal", "no context = other");
    }

    #[test]
    fn user_bucket_overrides_exe_and_host() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("MyChat.exe".to_string(), StyleBucket::Work);
        m.insert("www.mail.example.com".to_string(), StyleBucket::Email);
        m.insert("slack".to_string(), StyleBucket::Personal);
        m.insert("code".to_string(), StyleBucket::Other);
        let mut c = ctx("mychat.exe", None);
        apply_bucket_override(&mut c, &m);
        assert_eq!(effective_bucket(&c), StyleBucket::Work);
        assert_eq!(c.app_kind, AppKind::WorkMessaging);
        let mut c = ctx("Slack.exe", None);
        apply_bucket_override(&mut c, &m);
        assert_eq!(effective_bucket(&c), StyleBucket::Personal);
        let mut c = ctx("chrome.exe", Some("mail.example.com"));
        apply_bucket_override(&mut c, &m);
        assert_eq!(effective_bucket(&c), StyleBucket::Email);
        let mut c = ctx("code.exe", None);
        apply_bucket_override(&mut c, &m);
        assert_eq!(c.app_kind, AppKind::Ai, "AI formatting kept");
        let mut c = ctx("notepad.exe", None);
        let before = c.clone();
        apply_bucket_override(&mut c, &m);
        assert_eq!(c, before);
        assert_eq!(bucket_key(" WWW.Foo.com "), "foo.com");
    }

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
    fn app_kinds_from_app_groups() {
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
        // Exact exe names (D81): variants are listed, popular apps covered.
        assert_eq!(app_kind("Code - Insiders.exe", None), AppKind::Ai);
        assert_eq!(app_kind("ollama app.exe", None), AppKind::Ai);
        assert_eq!(app_kind("kitty.exe", None), AppKind::Terminal);
        assert_eq!(app_kind("wezterm-gui.exe", None), AppKind::Terminal);
        assert_eq!(
            app_kind("MailClient.exe", None),
            AppKind::Email,
            "eM Client"
        );
        assert_eq!(app_kind("mailspring.exe", None), AppKind::Email);
        assert_eq!(
            app_kind("chrome.exe", Some("aistudio.google.com")),
            AppKind::Ai
        );
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
