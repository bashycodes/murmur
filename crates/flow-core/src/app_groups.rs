//! App groups: which apps get special handling (style bucket, insert
//! checks, bubble rules), by app id and web host. One source,
//! `data/app-groups.json`, for flow-core on both platforms, the Windows
//! app and (through a parity test) the Android `PolicyLists.kt`. Its
//! `about` lines document the method and the public sources (D80).

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
    /// suffix, or starts with a prefix.
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
        serde_json::from_str(include_str!("../data/app-groups.json"))
            .expect("data/app-groups.json is valid")
    })
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
    "flow",
    "tagging_ide",
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
        for g in data().android.values().chain(data().windows.values()) {
            let a = &g.apps;
            for s in a
                .exact
                .iter()
                .chain(&a.families)
                .chain(&a.parts)
                .chain(&a.words)
            {
                assert_eq!(*s, s.to_lowercase(), "{s}");
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
    fn family_needs_a_dot() {
        assert!(android("compose_placeholder_apps").app("com.whatsapp.w4b"));
        assert!(!android("compose_placeholder_apps").app("com.whatsappx"));
    }
}
