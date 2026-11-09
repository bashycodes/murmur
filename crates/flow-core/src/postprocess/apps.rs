//! App category lookup (spec §9) and the Notion host (spec §5.7).

use crate::config::{AppCategory, FieldContext, Platform};

struct Row {
    category: AppCategory,
    /// Windows process names, lower-case, without `.exe`.
    windows: &'static [&'static str],
    /// Android package names, exact.
    android: &'static [&'static str],
    /// Hosts matched exactly on Windows.
    windows_hosts: &'static [&'static str],
    /// Hosts matched with their subdomains on Android.
    android_hosts: &'static [&'static str],
}

const EMAIL_HOSTS: &[&str] = &[
    "mail.google.com",
    "outlook.live.com",
    "outlook.office.com",
    "outlook.office365.com",
    "mail.proton.me",
    "mail.yahoo.com",
];

const PERSONAL_HOSTS: &[&str] = &[
    "web.whatsapp.com",
    "web.telegram.org",
    "discord.com",
    "messenger.com",
];

const TABLE: &[Row] = &[
    Row {
        category: AppCategory::Email,
        windows: &["outlook", "thunderbird", "mailbird", "mailspring"],
        android: &[
            "com.google.android.gm",
            "com.microsoft.office.outlook",
            "ch.protonmail.android",
            "com.samsung.android.email.provider",
            "com.fsck.k9",
            "net.thunderbird.android",
            "com.yahoo.mobile.client.android.mail",
        ],
        windows_hosts: EMAIL_HOSTS,
        android_hosts: EMAIL_HOSTS,
    },
    Row {
        category: AppCategory::WorkChat,
        windows: &["slack"],
        android: &["com.microsoft.teams", "com.mattermost.rn"],
        windows_hosts: &["app.slack.com", "teams.microsoft.com"],
        android_hosts: &["slack.com", "teams.microsoft.com", "teams.live.com"],
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
        ],
        windows_hosts: PERSONAL_HOSTS,
        android_hosts: PERSONAL_HOSTS,
    },
    Row {
        category: AppCategory::AiPrompt,
        windows: &["chatgpt", "claude", "cursor", "windsurf", "code"],
        android: &[
            "com.openai.chatgpt",
            "com.anthropic.claude",
            "com.google.android.apps.bard",
        ],
        windows_hosts: &[
            "chatgpt.com",
            "chat.openai.com",
            "claude.ai",
            "gemini.google.com",
        ],
        android_hosts: &[
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
            "cmd",
            "powershell",
            "pwsh",
            "alacritty",
            "wezterm-gui",
        ],
        android: &["com.termux"],
        windows_hosts: &[],
        android_hosts: &[],
    },
];

/// The category post-processing and AI cleanup use: the shell's own
/// category when it sent one, else [`listed_category`].
pub fn effective_category(platform: Platform, ctx: &FieldContext) -> AppCategory {
    if ctx.app_category != AppCategory::Other {
        return ctx.app_category;
    }
    listed_category(platform, ctx)
}

/// The category of the app id or URL host in the app lists alone,
/// ignoring `ctx.app_category`.
pub(super) fn listed_category(platform: Platform, ctx: &FieldContext) -> AppCategory {
    let host = normalise_host(&ctx.url_host);
    let row = match platform {
        Platform::Windows => {
            let id = windows_id(&ctx.app_id);
            TABLE
                .iter()
                .find(|r| r.windows.contains(&id.as_str()))
                .or_else(|| {
                    TABLE
                        .iter()
                        .find(|r| r.windows_hosts.contains(&host.as_str()))
                })
        }
        Platform::Android => {
            let id = ctx.app_id.trim();
            TABLE.iter().find(|r| r.android.contains(&id)).or_else(|| {
                TABLE
                    .iter()
                    .find(|r| r.android_hosts.iter().any(|h| host_matches(&host, h)))
            })
        }
    };
    row.map_or(AppCategory::Other, |r| r.category)
}

/// The URL host is exactly `notion.so` (§5.7).
pub(super) fn is_notion_host(ctx: &FieldContext) -> bool {
    normalise_host(&ctx.url_host) == "notion.so"
}

/// A Windows process name, lower-case, without `.exe`.
fn windows_id(app_id: &str) -> String {
    let lower = app_id.trim().to_lowercase();
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
        let app_id = match p {
            Platform::Windows => "chrome.exe",
            Platform::Android => "com.android.chrome",
        };
        effective_category(
            p,
            &FieldContext {
                app_id: app_id.into(),
                url_host: h.into(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn windows_app_ids() {
        assert_eq!(win("slack.exe"), WorkChat);
        assert_eq!(win("OUTLOOK.EXE"), Email);
        assert_eq!(win("thunderbird"), Email);
        assert_eq!(win("Mailbird.exe"), Email);
        assert_eq!(win("mailspring.exe"), Email);
        assert_eq!(win("WhatsApp.exe"), PersonalChat);
        assert_eq!(win("WhatsApp.Root.exe"), PersonalChat);
        assert_eq!(win("Telegram.exe"), PersonalChat);
        assert_eq!(win("Discord.exe"), PersonalChat);
        assert_eq!(win("Signal.exe"), PersonalChat);
        assert_eq!(win("Messenger.exe"), PersonalChat);
        assert_eq!(win("WindowsTerminal.exe"), Terminal);
        assert_eq!(win("cmd.exe"), Terminal);
        assert_eq!(win("pwsh.exe"), Terminal);
        assert_eq!(win("wezterm-gui.exe"), Terminal);
        assert_eq!(win("ChatGPT.exe"), AiPrompt);
        assert_eq!(win("Claude.exe"), AiPrompt);
        assert_eq!(win("Cursor.exe"), AiPrompt);
        assert_eq!(win("Code.exe"), AiPrompt);
        // Not listed: Other.
        for id in [
            "olk.exe",
            "ms-teams.exe",
            "Teams.exe",
            "Zoom.exe",
            "mattermost.exe",
            "WINWORD.EXE",
            "Notion.exe",
            "Obsidian.exe",
            "notepad.exe",
            "chrome.exe",
            "zentrix.exe",
        ] {
            assert_eq!(win(id), Other, "{id}");
        }
        // Android packages are not looked up on Windows.
        assert_eq!(win("com.whatsapp"), Other);
    }

    #[test]
    fn android_packages() {
        assert_eq!(android("com.whatsapp"), PersonalChat);
        assert_eq!(android("com.microsoft.teams"), WorkChat);
        assert_eq!(android("com.mattermost.rn"), WorkChat);
        assert_eq!(android("com.google.android.gm"), Email);
        assert_eq!(android("com.instagram.android"), PersonalChat);
        assert_eq!(android("com.termux"), Terminal);
        assert_eq!(android("com.google.android.apps.bard"), AiPrompt);
        for id in [
            "com.slack",
            "us.zoom.videomeetings",
            "im.vector.app",
            "com.linkedin.android",
            "notion.id",
        ] {
            assert_eq!(android(id), Other, "{id}");
        }
        // Exact package names; Windows ids are not looked up on Android.
        assert_eq!(android("COM.WHATSAPP"), Other);
        assert_eq!(android("slack"), Other);
    }

    #[test]
    fn windows_hosts_are_exact() {
        let w = Platform::Windows;
        assert_eq!(host(w, "mail.google.com"), Email);
        assert_eq!(host(w, "app.slack.com"), WorkChat);
        assert_eq!(host(w, "teams.microsoft.com"), WorkChat);
        assert_eq!(host(w, "web.whatsapp.com"), PersonalChat);
        assert_eq!(host(w, "messenger.com"), PersonalChat);
        assert_eq!(host(w, "claude.ai"), AiPrompt);
        for h in [
            "www.messenger.com",
            "x.slack.com",
            "slack.com",
            "teams.live.com",
            "perplexity.ai",
            "docs.google.com",
            "notion.so",
            "",
        ] {
            assert_eq!(host(w, h), Other, "{h}");
        }
    }

    #[test]
    fn android_hosts_match_subdomains() {
        let a = Platform::Android;
        assert_eq!(host(a, "mail.google.com"), Email);
        assert_eq!(host(a, "slack.com"), WorkChat);
        assert_eq!(host(a, "app.slack.com"), WorkChat);
        assert_eq!(host(a, "x.slack.com"), WorkChat);
        assert_eq!(host(a, "teams.live.com"), WorkChat);
        assert_eq!(host(a, "www.messenger.com"), PersonalChat);
        assert_eq!(host(a, "ptb.discord.com"), PersonalChat);
        assert_eq!(host(a, "perplexity.ai"), AiPrompt);
        assert_eq!(host(a, "notdiscord.com"), Other);
        assert_eq!(host(a, "docs.google.com"), Other);
    }

    #[test]
    fn shell_category_wins_but_not_in_the_listed_one() {
        let ctx = FieldContext {
            app_id: "slack.exe".into(),
            app_category: PersonalChat,
            ..Default::default()
        };
        assert_eq!(effective_category(Platform::Windows, &ctx), PersonalChat);
        assert_eq!(listed_category(Platform::Windows, &ctx), WorkChat);
    }

    #[test]
    fn notion_host_is_exact() {
        let n = |h: &str| {
            is_notion_host(&FieldContext {
                url_host: h.into(),
                ..Default::default()
            })
        };
        assert!(n("notion.so"));
        assert!(!n("www.notion.so"));
        assert!(!n("zentrix.notion.site"));
        assert!(!n(""));
    }
}
