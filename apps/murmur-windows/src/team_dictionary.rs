//! The server's team dictionary (`murmur_protocol::dictionary`): one shared
//! word list that every client of the saved murmur-server uses like its
//! own dictionary words, under them (a personal entry wins on the same
//! word; `Prefs::dictation_dictionary`). Settings → Dictionary shows it
//! read-only; Settings → Your server → Users & keys edits it with an admin
//! key.
//!
//! The list is fetched when the app starts and when Settings opens, with
//! the cached copy's ETag, and kept in `team-dictionary.json` next to
//! `prefs.json` (`murmur_remote::team_dict`, shared with Android). Offline,
//! the last copy is used. No server saved means no team list. The cache
//! holds the server's address and the list, never the key.

use std::path::{Path, PathBuf};

use murmur_protocol::admin::{code, AdminError};
use murmur_protocol::dictionary::TeamEntry;
use serde::Serialize;

use crate::core_api::DictEntry;
use crate::dev_settings::DevSettings;

/// The cache file, next to `prefs.json`.
pub const CACHE_FILE: &str = "team-dictionary.json";

/// What the Dictionary page and Users & keys show (`team_dictionary_*`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TeamView {
    /// A server and a token are saved: show the team section.
    pub configured: bool,
    /// The list in use (fresh, or the cached copy).
    pub entries: Vec<TeamEntry>,
    /// Why the server's list could not be read (the cached copy is shown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `team-dictionary.json` next to `prefs_path`.
pub fn cache_path(prefs_path: &Path) -> PathBuf {
    prefs_path.with_file_name(CACHE_FILE)
}

/// The entries as dictionary entries (`Prefs::team_dictionary`).
pub fn to_dict(entries: &[TeamEntry]) -> Vec<DictEntry> {
    entries
        .iter()
        .map(|e| DictEntry {
            word: e.word.clone(),
            replacement: e.replacement.clone(),
            is_snippet: false,
        })
        .collect()
}

/// Page text for a team dictionary error.
pub fn error_text(e: &AdminError) -> String {
    match e.code.as_str() {
        code::UNSUPPORTED => "this server has no team dictionary: update murmur-server".into(),
        code::AUTH_OFF => "this server runs without keys, so it has no team dictionary".into(),
        code::NOT_FOUND | code::BAD_REQUEST | murmur_protocol::dictionary::CODE_TOO_MANY
            if !e.message.is_empty() =>
        {
            e.message.clone()
        }
        _ => crate::server_admin::error_text(e),
    }
}

/// The cached list for the saved server, without the network (start-up).
pub fn cached(dev: &DevSettings, cache: &Path) -> TeamView {
    #[cfg(feature = "remote")]
    if let Ok(c) = crate::server_admin::client(dev) {
        return TeamView {
            configured: true,
            entries: murmur_remote::team_dict::TeamDictCache::new(cache).load(&c.server()),
            error: None,
        };
    }
    let _ = (dev, cache);
    TeamView::default()
}

/// Fetch the saved server's list if it changed (blocking, ≤ 15 s); offline,
/// the cached copy with the reason.
pub fn refresh(dev: &DevSettings, cache: &Path) -> TeamView {
    #[cfg(feature = "remote")]
    if let Ok(c) = crate::server_admin::client(dev) {
        let r = murmur_remote::team_dict::TeamDictCache::new(cache).refresh(&c);
        let error = r.error.filter(|e| {
            // A server without a team dictionary is not a problem to show.
            e.code != code::UNSUPPORTED && e.code != code::AUTH_OFF
        });
        return TeamView {
            configured: true,
            entries: r.entries,
            error: error.map(|e| format!("Using the saved copy: {}", error_text(&e))),
        };
    }
    let _ = (dev, cache);
    TeamView::default()
}

/// Add `word` (→ `replacement` when not blank), or replace the entry with
/// that word (admin key only). The new list, also cached.
pub fn add(
    dev: &DevSettings,
    cache: &Path,
    word: &str,
    replacement: &str,
) -> Result<TeamView, String> {
    let e = TeamEntry {
        word: word.into(),
        replacement: Some(replacement.to_string()).filter(|r| !r.trim().is_empty()),
    };
    murmur_protocol::dictionary::check_entry(&e)?;
    #[cfg(feature = "remote")]
    {
        edit(dev, cache, |c| c.add_team_entry(&e))
    }
    #[cfg(not(feature = "remote"))]
    {
        no_server(dev, cache)
    }
}

/// Remove `word` (admin key only). The new list, also cached.
pub fn remove(dev: &DevSettings, cache: &Path, word: &str) -> Result<TeamView, String> {
    #[cfg(feature = "remote")]
    {
        edit(dev, cache, |c| c.remove_team_entry(word))
    }
    #[cfg(not(feature = "remote"))]
    {
        let _ = word;
        no_server(dev, cache)
    }
}

#[cfg(feature = "remote")]
fn edit(
    dev: &DevSettings,
    cache: &Path,
    f: impl FnOnce(
        &murmur_remote::admin::AdminClient,
    )
        -> Result<(murmur_protocol::dictionary::TeamDictionary, Option<String>), AdminError>,
) -> Result<TeamView, String> {
    let c = crate::server_admin::client(dev)?;
    let answer = f(&c).map_err(|e| error_text(&e))?;
    Ok(TeamView {
        configured: true,
        entries: murmur_remote::team_dict::TeamDictCache::new(cache).edited(&c, &answer),
        error: None,
    })
}

#[cfg(not(feature = "remote"))]
fn no_server(dev: &DevSettings, cache: &Path) -> Result<TeamView, String> {
    let _ = (dev, cache);
    Err("this build has no server support".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_server_means_no_team_list() {
        let d = std::env::temp_dir().join("murmur-team-dictionary-test-none.json");
        let dev = DevSettings::default();
        assert_eq!(cached(&dev, &d), TeamView::default());
        assert_eq!(refresh(&dev, &d), TeamView::default());
        assert!(add(&dev, &d, "Quillon", "").is_err());
        // A bad entry is refused before any server is asked.
        let m = add(&dev, &d, "<b>x</b>", "").unwrap_err();
        assert!(m.contains("markup"), "{m}");
    }

    #[test]
    fn cache_sits_next_to_prefs_and_entries_are_never_snippets() {
        let p = Path::new("dir").join("prefs.json");
        assert_eq!(cache_path(&p), Path::new("dir").join(CACHE_FILE));
        let d = to_dict(&[TeamEntry {
            word: "acme".into(),
            replacement: Some("AcmeCorp".into()),
        }]);
        assert_eq!(d[0].replacement.as_deref(), Some("AcmeCorp"));
        assert!(!d[0].is_snippet);
    }

    #[test]
    fn errors_read_as_page_text() {
        let e = |c: &str, m: &str| AdminError::new(c, m);
        assert!(error_text(&e(code::UNSUPPORTED, "")).contains("no team dictionary"));
        assert_eq!(
            error_text(&e(
                murmur_protocol::dictionary::CODE_TOO_MANY,
                "the team dictionary is full"
            )),
            "the team dictionary is full"
        );
        assert_eq!(
            error_text(&e(code::FORBIDDEN, "")),
            "this app's key is not an admin key"
        );
    }

    /// The team list reaches the core like personal words: replacements,
    /// ASR vocabulary (hotwords, Whisper prompt) and cleanup terms all read
    /// the session's dictionary; the personal spelling wins.
    #[test]
    fn team_words_reach_the_core_session_under_personal_ones() {
        let p = crate::prefs::Prefs {
            dictionary: vec![DictEntry {
                word: "zephyr".into(),
                replacement: Some("Zephyr".into()),
                is_snippet: false,
            }],
            team_dictionary: team(),
            ..Default::default()
        };
        let cfg = crate::core_adapter::to_core_session(&p.session_config());
        assert_eq!(
            cfg.dictionary.vocabulary(),
            vec!["Zephyr", "AcmeCorp", "Quillon"]
        );
        let out = murmur_core::postprocess::apply(
            "zephyr met acme corp",
            "zephyr met acme corp",
            &cfg,
            &murmur_core::config::FieldContext::default(),
        );
        assert!(
            out.text.starts_with("Zephyr met AcmeCorp"),
            "{:?}",
            out.text
        );
    }

    fn team() -> Vec<DictEntry> {
        to_dict(&[
            TeamEntry {
                word: "Zephyr".into(),
                replacement: Some("ZephyrCo".into()),
            },
            TeamEntry {
                word: "acme corp".into(),
                replacement: Some("AcmeCorp".into()),
            },
            TeamEntry {
                word: "Quillon".into(),
                replacement: None,
            },
        ])
    }
}
