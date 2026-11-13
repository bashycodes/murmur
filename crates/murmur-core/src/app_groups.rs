//! App groups: which apps get special handling (style bucket, insert
//! checks, bubble rules), by app id and web host. One source,
//! `data/app-groups.json`, for murmur-core on both platforms, the Windows
//! app and (through a parity test) the Android `PolicyLists.kt`. Its
//! `about` lines document the method and the public sources (D81).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Apps {
    exact: Vec<String>,
    families: Vec<String>,
    parts: Vec<String>,
    words: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Hosts {
    suffixes: Vec<String>,
    prefixes: Vec<String>,
    parts: Vec<String>,
}

/// One group: app-id rules plus web-host rules.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Group {
    apps: Apps,
    hosts: Hosts,
}

impl Group {
    /// `id` (package name or exe name without `.exe`) is in the group:
    /// exact, a family (`f` or `f.*`), a dot-separated part, or a word
    /// anywhere. Case-insensitive.
    pub fn app(&self, id: &str) -> bool {
        let id = id.trim().to_lowercase();
        if id.is_empty() {
            return false;
        }
        let a = &self.apps;
        a.exact.contains(&id)
            || a.families.iter().any(|f| {
                id == *f
                    || id
                        .strip_prefix(f.as_str())
                        .is_some_and(|r| r.starts_with('.'))
            })
            || id.split('.').any(|p| a.parts.iter().any(|x| x == p))
            || a.words.iter().any(|w| id.contains(w.as_str()))
    }

    /// `host` (any case, `www.` ignored) equals or is a subdomain of a
    /// suffix, starts with a prefix, or has a part as a whole dot-separated
    /// label other than the last (`slack.example.org`; not `slackware.…`,
    /// not a top-level domain).
    pub fn host(&self, host: &str) -> bool {
        let h = host.trim().to_lowercase();
        let h = h.strip_prefix("www.").unwrap_or(&h);
        if h.is_empty() {
            return false;
        }
        self.hosts
            .suffixes
            .iter()
            .any(|s| h == s || h.ends_with(&format!(".{s}")))
            || self
                .hosts
                .prefixes
                .iter()
                .any(|p| h.starts_with(p.as_str()))
            || h.rsplit_once('.').is_some_and(|(labels, _tld)| {
                labels
                    .split('.')
                    .any(|l| self.hosts.parts.iter().any(|x| x == l))
            })
    }

    /// [`Group::app`] or [`Group::host`].
    pub fn matches(&self, id: &str, host: &str) -> bool {
        self.app(id) || self.host(host)
    }
}

#[derive(Deserialize)]
struct Data {
    android: BTreeMap<String, Group>,
    windows: BTreeMap<String, Group>,
    compose_placeholders: Vec<String>,
}

fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| {
        let mut d: Data = serde_json::from_str(RAW).expect("data/app-groups.json is valid");
        // Lowercase on load, as PolicyLists.kt does, so a mixed-case entry
        // still matches the lowercased lookup on both platforms.
        for g in d.android.values_mut().chain(d.windows.values_mut()) {
            let (a, h) = (&mut g.apps, &mut g.hosts);
            for list in [
                &mut a.exact,
                &mut a.families,
                &mut a.parts,
                &mut a.words,
                &mut h.suffixes,
                &mut h.prefixes,
                &mut h.parts,
            ] {
                lower(list);
            }
        }
        lower(&mut d.compose_placeholders);
        d
    })
}

const RAW: &str = include_str!("../data/app-groups.json");

fn lower(list: &mut [String]) {
    for s in list {
        *s = s.trim().to_lowercase();
    }
}

/// An Android group by name (`email`, `restricted`, `browser`, …).
/// Panics on an unknown name (a typo is a bug, caught by tests).
pub fn android(name: &str) -> &'static Group {
    data()
        .android
        .get(name)
        .unwrap_or_else(|| panic!("no android app group {name}"))
}

/// A Windows group by name (`work`, `messaging`, `email`, `ai`, …).
pub fn windows(name: &str) -> &'static Group {
    data()
        .windows
        .get(name)
        .unwrap_or_else(|| panic!("no windows app group {name}"))
}

/// The localized "Message" placeholder, normalized (lowercase, no
/// trailing dots), for `compose_placeholder_apps`.
pub fn compose_placeholders() -> &'static [String] {
    &data().compose_placeholders
}

/// Windows group names, in the order the Windows app ranks them.
pub const WINDOWS_GROUPS: &[&str] = &[
    "work",
    "messaging",
    "email",
    "ai",
    "terminal",
    "document",
    "murmur",
];

/// Lowercase exact app ids of a Windows group (for the Settings table).
pub fn windows_exact(name: &str) -> &'static [String] {
    &windows(name).apps.exact
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_is_lowercase_and_every_named_group_exists() {
        // The raw file itself is lowercase (ids, hosts, placeholders), so
        // the Kotlin and Rust readers see the same strings either way.
        let raw: Data = serde_json::from_str(RAW).unwrap();
        for g in raw.android.values().chain(raw.windows.values()) {
            let (a, h) = (&g.apps, &g.hosts);
            for s in a
                .exact
                .iter()
                .chain(&a.families)
                .chain(&a.parts)
                .chain(&a.words)
                .chain(&h.suffixes)
                .chain(&h.prefixes)
                .chain(&h.parts)
                .chain(&raw.compose_placeholders)
            {
                assert_eq!(*s, s.trim().to_lowercase(), "{s}");
            }
        }
        for n in WINDOWS_GROUPS {
            windows(n);
        }
        for n in [
            "restart_prone",
            "known_null",
            "system_search",
            "in_call",
            "system_ui",
            "browser",
            "compose_placeholder_apps",
            "restricted",
            "email",
            "work",
            "personal",
            "work_chat",
            "ai",
            "terminal",
        ] {
            android(n);
        }
    }

    #[test]
    fn restart_prone_apps_are_known_null() {
        for p in &android("restart_prone").apps.exact {
            assert!(android("known_null").app(p), "{p}");
        }
    }

    #[test]
    fn host_parts_are_windows_only() {
        // PolicyLists.kt reads android hosts without `parts`; an android
        // entry there would match in Rust only.
        for (n, g) in &data().android {
            assert!(
                g.hosts.parts.is_empty(),
                "android group {n} has hosts.parts"
            );
        }
    }

    #[test]
    fn host_part_is_a_whole_label_before_the_tld() {
        let g = windows("work");
        assert!(g.host("slack.example.org"));
        assert!(g.host("www.Team.Slack.Example.org"));
        assert!(!g.host("slackware.example.org"));
        assert!(!g.host("example-slack.test"));
        assert!(!g.host("example.slack"));
    }

    #[test]
    fn family_needs_a_dot() {
        assert!(android("compose_placeholder_apps").app("com.whatsapp.w4b"));
        assert!(!android("compose_placeholder_apps").app("com.whatsappx"));
    }
}
