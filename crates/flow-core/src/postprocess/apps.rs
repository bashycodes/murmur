//! App category lookup (spec §9) and the apps that manage their own
//! spacing (spec §5.7).

use crate::config::{AppCategory, FieldContext, Platform};

struct Row {
    category: AppCategory,
    /// Windows process names, lower-case, without `.exe`.
    windows: &'static [&'static str],
    /// Android package names, exact.
    android: &'static [&'static str],
    /// Hosts; each also matches its subdomains.
    hosts: &'static [&'static str],
}

const TABLE: &[Row] = &[
    Row {
        category: AppCategory::Email,
        windows: &[
            "outlook",
            "olk",
            "thunderbird",
            "betterbird",
            "mailbird",
            "mailspring",
            "hxoutlook",
        ],
        android: &[
            "com.google.android.gm",
            "com.microsoft.office.outlook",
            "ch.protonmail.android",
            "com.samsung.android.email.provider",
            "com.fsck.k9",
            "net.thunderbird.android",
            "com.yahoo.mobile.client.android.mail",
        ],
        hosts: &[
            "mail.google.com",
            "outlook.live.com",
            "outlook.office.com",
            "outlook.office365.com",
            "mail.proton.me",
            "mail.yahoo.com",
        ],
    },
    Row {
        category: AppCategory::WorkChat,
        windows: &["slack", "ms-teams", "teams", "zoom", "mattermost"],
        android: &[
            "com.slack",
            "com.microsoft.teams",
            "us.zoom.videomeetings",
            "com.mattermost.rn",
        ],
        hosts: &["app.slack.com", "teams.microsoft.com", "teams.live.com"],
    },
    Row {
        category: AppCategory::PersonalChat,
        windows: &[
            "whatsapp",
            "whatsapp.root",
            "telegram",
            "discord",
            "signal",
            "messenger",
            "viber",
            "line",
            "skype",
            "element",
        ],
        android: &[
            "com.whatsapp",
            "com.whatsapp.w4b",
            "org.telegram.messenger",
            "org.thoughtcrime.securesms",
            "com.discord",
            "com.facebook.orca",
            "com.instagram.android",
            "com.viber.voip",
            "jp.naver.line.android",
            "com.google.android.apps.messaging",
            "com.samsung.android.messaging",
            "im.vector.app",
        ],
        hosts: &[
            "web.whatsapp.com",
            "web.telegram.org",
            "discord.com",
            "messenger.com",
        ],
    },
    Row {
        category: AppCategory::AiPrompt,
        windows: &[
            "chatgpt",
            "claude",
            "perplexity",
            "cursor",
            "windsurf",
            "code",
        ],
        android: &[
            "com.openai.chatgpt",
            "com.anthropic.claude",
            "ai.perplexity.app.android",
            "com.google.android.apps.bard",
        ],
        hosts: &[
            "chatgpt.com",
            "chat.openai.com",
            "claude.ai",
            "gemini.google.com",
            "perplexity.ai",
        ],
    },
    Row {
        category: AppCategory::Terminal,
        windows: &[
            "windowsterminal",
            "wt",
            "cmd",
            "powershell",
            "pwsh",
            "conhost",
            "alacritty",
            "wezterm-gui",
            "mintty",
            "putty",
        ],
        android: &[
            "com.termux",
            "org.connectbot",
            "com.server.auditor.ssh.client",
        ],
        hosts: &[],
    },
    Row {
        category: AppCategory::Document,
        windows: &[
            "winword", "notion", "obsidian", "onenote", "soffice", "swriter",
        ],
        android: &[
            "com.google.android.apps.docs.editors.docs",
            "com.microsoft.office.word",
            "notion.id",
            "md.obsidian",
        ],
        hosts: &["docs.google.com", "notion.so", "notion.site"],
    },
];

/// The category post-processing and AI cleanup use: the shell's own
/// category when it sent one, else this module's app and host lists.
pub fn effective_category(platform: Platform, ctx: &FieldContext) -> AppCategory {
    if ctx.app_category != AppCategory::Other {
        return ctx.app_category;
    }
    let by_app = match platform {
        Platform::Windows => {
            let id = windows_id(&ctx.app_id);
            TABLE.iter().find(|r| r.windows.contains(&id.as_str()))
        }
        Platform::Android => {
            let id = ctx.app_id.trim();
            TABLE.iter().find(|r| r.android.contains(&id))
        }
    };
    if let Some(r) = by_app {
        return r.category;
    }
    let host = normalise_host(&ctx.url_host);
    TABLE
        .iter()
        .find(|r| r.hosts.iter().any(|h| host_matches(&host, h)))
        .map_or(AppCategory::Other, |r| r.category)
}

/// Fields whose app handles spacing and case itself: the dictation is
/// inserted trimmed, without cursor fitting or a trailing space.
pub(super) fn manages_own_spacing(platform: Platform, ctx: &FieldContext) -> bool {
    let host = normalise_host(&ctx.url_host);
    if ["notion.so", "notion.site"]
        .iter()
        .any(|h| host_matches(&host, h))
    {
        return true;
    }
    match platform {
        Platform::Windows => matches!(
            windows_id(&ctx.app_id).as_str(),
            "notion" | "flow-windows" | "murmur" | "murmur"
        ),
        Platform::Android => {
            let id = ctx.app_id.trim();
            id == "notion.id" || id == "dev.flowclone" || id.starts_with("dev.flowclone.")
        }
    }
}

/// A Windows process name, lower-case, without a directory or `.exe`.
fn windows_id(app_id: &str) -> String {
    let id = app_id.trim();
    let base = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let lower = base.to_lowercase();
    match lower.strip_suffix(".exe") {
        Some(s) => s.to_string(),
        None => lower,
    }
}

fn normalise_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_lowercase()
}

/// `host` is `domain` or one of its subdomains.
fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppCategory::*;

    fn win(id: &str) -> AppCategory {
        effective_category(
            Platform::Windows,
            &FieldContext {
                app_id: id.into(),
                ..Default::default()
            },
        )
    }

    fn android(id: &str) -> AppCategory {
        effective_category(
            Platform::Android,
            &FieldContext {
                app_id: id.into(),
                ..Default::default()
            },
        )
    }

    fn host(p: Platform, h: &str) -> AppCategory {
        effective_category(
            p,
            &FieldContext {
                app_id: "chrome.exe".into(),
                url_host: h.into(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn windows_app_ids() {
        assert_eq!(win("slack.exe"), WorkChat);
        assert_eq!(win("OUTLOOK.EXE"), Email);
        assert_eq!(win("olk.exe"), Email);
        assert_eq!(win("thunderbird"), Email);
        assert_eq!(win("ms-teams.exe"), WorkChat);
        assert_eq!(win("Teams.exe"), WorkChat);
        assert_eq!(win("WhatsApp.exe"), PersonalChat);
        assert_eq!(win("Telegram.exe"), PersonalChat);
        assert_eq!(win("Discord.exe"), PersonalChat);
        assert_eq!(win("Signal.exe"), PersonalChat);
        assert_eq!(win("WindowsTerminal.exe"), Terminal);
        assert_eq!(win("pwsh.exe"), Terminal);
        assert_eq!(win("ChatGPT.exe"), AiPrompt);
        assert_eq!(win("Claude.exe"), AiPrompt);
        assert_eq!(win("Cursor.exe"), AiPrompt);
        assert_eq!(win("Code.exe"), AiPrompt);
        assert_eq!(win("WINWORD.EXE"), Document);
        assert_eq!(win("Notion.exe"), Document);
        assert_eq!(win("linkedin.exe"), Other);
        assert_eq!(win("zentrix.exe"), Other);
        // Android packages are not looked up on Windows.
        assert_eq!(win("com.whatsapp"), Other);
    }

    #[test]
    fn android_packages() {
        assert_eq!(android("com.whatsapp"), PersonalChat);
        assert_eq!(android("com.slack"), WorkChat);
        assert_eq!(android("com.google.android.gm"), Email);
        assert_eq!(android("com.instagram.android"), PersonalChat);
        assert_eq!(android("com.linkedin.android"), Other);
        assert_eq!(android("com.termux"), Terminal);
        assert_eq!(
            android("com.google.android.apps.docs.editors.docs"),
            Document
        );
        // Exact package names; Windows ids are not looked up on Android.
        assert_eq!(android("COM.WHATSAPP"), Other);
        assert_eq!(android("slack"), Other);
    }

    #[test]
    fn hosts_and_subdomains() {
        for p in [Platform::Windows, Platform::Android] {
            assert_eq!(host(p, "mail.google.com"), Email);
            assert_eq!(host(p, "app.slack.com"), WorkChat);
            assert_eq!(host(p, "web.whatsapp.com"), PersonalChat);
            assert_eq!(host(p, "claude.ai"), AiPrompt);
            assert_eq!(host(p, "docs.google.com"), Document);
            assert_eq!(host(p, "www.notion.so"), Document);
            assert_eq!(host(p, "ptb.discord.com"), PersonalChat);
            assert_eq!(host(p, "notdiscord.com"), Other);
            assert_eq!(host(p, "www.linkedin.com"), Other);
        }
    }

    #[test]
    fn shell_category_wins() {
        let ctx = FieldContext {
            app_id: "slack.exe".into(),
            app_category: PersonalChat,
            ..Default::default()
        };
        assert_eq!(effective_category(Platform::Windows, &ctx), PersonalChat);
    }

    #[test]
    fn own_spacing_apps() {
        let own = |p, id: &str, h: &str| {
            manages_own_spacing(
                p,
                &FieldContext {
                    app_id: id.into(),
                    url_host: h.into(),
                    ..Default::default()
                },
            )
        };
        assert!(own(Platform::Windows, "Notion.exe", ""));
        assert!(own(Platform::Windows, "flow-windows.exe", ""));
        assert!(own(Platform::Windows, "Murmur.exe", ""));
        assert!(own(Platform::Windows, "murmur.exe", ""));
        assert!(own(Platform::Windows, "chrome.exe", "notion.so"));
        assert!(own(Platform::Windows, "chrome.exe", "www.notion.so"));
        assert!(own(
            Platform::Android,
            "com.android.chrome",
            "zentrix.notion.site"
        ));
        assert!(own(Platform::Android, "notion.id", ""));
        assert!(own(Platform::Android, "dev.flowclone", ""));
        assert!(own(Platform::Android, "dev.flowclone.hybrid", ""));
        assert!(!own(Platform::Android, "dev.flowclonex", ""));
        assert!(!own(Platform::Windows, "slack.exe", ""));
        assert!(!own(Platform::Windows, "chrome.exe", "notion.com"));
    }
}
