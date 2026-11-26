//! Client side of the team dictionary (`murmur_protocol::dictionary`):
//! fetch the server's list with `If-None-Match`, keep the last copy on disk,
//! and use that copy while the server can't be reached. Shared by
//! murmur-windows and, through murmur-ffi, Android, so both apps refresh,
//! cache and fall back the same way.
//!
//! The cache file is one JSON object `{server, etag, entries}`: the list of
//! the server it came from (`host:port`), never the key. A cache from
//! another server is ignored (switching servers drops the old team list).
//! What is read or downloaded passes `dictionary::sanitize` first.

use std::path::{Path, PathBuf};

use murmur_protocol::admin::{code, AdminError};
use murmur_protocol::dictionary::{sanitize, TeamDictionary, TeamEntry};
use serde::{Deserialize, Serialize};

use crate::admin::AdminClient;

/// The cache file's content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct CacheFile {
    server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    entries: Vec<TeamEntry>,
}

/// What [`TeamDictCache::refresh`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Refresh {
    /// The list to use now: fresh, or the cached copy.
    pub entries: Vec<TeamEntry>,
    /// `entries` differ from the cached copy.
    pub changed: bool,
    /// Why the server's list could not be read; `entries` is then the
    /// cached copy (or empty when the server has no team dictionary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AdminError>,
}

/// The on-disk copy of one server's team dictionary.
#[derive(Debug, Clone)]
pub struct TeamDictCache {
    path: PathBuf,
}

impl TeamDictCache {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self, server: &str) -> CacheFile {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|c| c.server == server)
            .map(|c| CacheFile {
                entries: sanitize(&c.entries),
                ..c
            })
            .unwrap_or_default()
    }

    /// The cached list of `server` (`host:port`); empty for none or
    /// another server's.
    pub fn load(&self, server: &str) -> Vec<TeamEntry> {
        self.read(server).entries
    }

    /// Save `list` as `server`'s current copy (after a fetch or an edit).
    /// Returns the sanitized entries.
    pub fn store(
        &self,
        server: &str,
        list: &TeamDictionary,
        etag: Option<String>,
    ) -> std::io::Result<Vec<TeamEntry>> {
        let c = CacheFile {
            server: server.into(),
            etag,
            entries: sanitize(&list.entries),
        };
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&c).expect("cache serialises"))?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(c.entries)
    }

    /// Forget the cached list (no server set any more).
    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Fetch `client`'s list if it changed since the cached copy.
    pub fn refresh(&self, client: &AdminClient) -> Refresh {
        self.refresh_with(&client.server(), |etag| client.team_dictionary(etag))
    }

    /// Cache the list an admin edit answered with (`AdminClient::add_team_entry`
    /// / `remove_team_entry`), so the editing app uses it at once. Returns the
    /// entries to use.
    pub fn edited(
        &self,
        client: &AdminClient,
        answer: &(TeamDictionary, Option<String>),
    ) -> Vec<TeamEntry> {
        let (list, etag) = answer;
        self.store(&client.server(), list, etag.clone())
            .unwrap_or_else(|_| sanitize(&list.entries))
    }

    /// [`Self::refresh`] with the fetch given (tests). `fetch` gets the
    /// cached ETag and answers like `AdminClient::team_dictionary`.
    pub fn refresh_with(
        &self,
        server: &str,
        fetch: impl FnOnce(Option<&str>) -> Result<Option<(TeamDictionary, Option<String>)>, AdminError>,
    ) -> Refresh {
        let cached = self.read(server);
        let etag = cached.etag.as_deref().filter(|_| self.path.is_file());
        match fetch(etag) {
            Ok(Some((list, etag))) => match self.store(server, &list, etag) {
                Ok(entries) => Refresh {
                    changed: entries != cached.entries,
                    entries,
                    error: None,
                },
                Err(e) => {
                    let entries = sanitize(&list.entries);
                    Refresh {
                        changed: entries != cached.entries,
                        entries,
                        error: Some(AdminError::new(
                            code::SERVER_ERROR,
                            format!("team dictionary cache: {e}"),
                        )),
                    }
                }
            },
            Ok(None) => Refresh {
                entries: cached.entries,
                changed: false,
                error: None,
            },
            // The server has no team dictionary (auth off, or older than
            // the feature): there is no team list.
            Err(e) if e.code == code::AUTH_OFF || e.code == code::UNSUPPORTED => {
                self.clear();
                Refresh {
                    changed: !cached.entries.is_empty(),
                    entries: Vec::new(),
                    error: Some(e),
                }
            }
            // Offline, throttled, a revoked key: keep using the last copy.
            Err(e) => Refresh {
                entries: cached.entries,
                changed: false,
                error: Some(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: &str = "192.0.2.1:7313";

    fn e(w: &str) -> TeamEntry {
        TeamEntry {
            word: w.into(),
            replacement: None,
        }
    }

    fn list(words: &[&str]) -> TeamDictionary {
        TeamDictionary {
            entries: words.iter().map(|w| e(w)).collect(),
        }
    }

    fn cache() -> (tempfile::TempDir, TeamDictCache) {
        let d = tempfile::tempdir().unwrap();
        let c = TeamDictCache::new(d.path().join("cache").join("team-dictionary.json"));
        (d, c)
    }

    #[test]
    fn first_fetch_caches_and_the_next_sends_the_etag() {
        let (_d, c) = cache();
        let r = c.refresh_with(SERVER, |etag| {
            assert_eq!(etag, None);
            Ok(Some((list(&["Quillon"]), Some("\"t1\"".into()))))
        });
        assert_eq!(
            (r.entries.clone(), r.changed, r.error),
            (vec![e("Quillon")], true, None)
        );
        // 304: the cached copy, unchanged.
        let r = c.refresh_with(SERVER, |etag| {
            assert_eq!(etag, Some("\"t1\""));
            Ok(None)
        });
        assert_eq!((r.entries, r.changed), (vec![e("Quillon")], false));
        // A new list replaces it.
        let r = c.refresh_with(SERVER, |_| {
            Ok(Some((list(&["Brixwell"]), Some("\"t2\"".into()))))
        });
        assert_eq!((r.entries, r.changed), (vec![e("Brixwell")], true));
        assert_eq!(c.load(SERVER), vec![e("Brixwell")]);
    }

    #[test]
    fn offline_uses_the_cache_and_no_team_list_clears_it() {
        let (_d, c) = cache();
        c.store(SERVER, &list(&["Velloria"]), Some("\"t\"".into()))
            .unwrap();
        let r = c.refresh_with(SERVER, |_| {
            Err(AdminError::new(code::CONNECT_FAILED, "can't reach"))
        });
        assert_eq!(r.entries, vec![e("Velloria")]);
        assert_eq!(r.error.unwrap().code, code::CONNECT_FAILED);
        let r = c.refresh_with(SERVER, |_| Err(AdminError::new(code::UNAUTHORIZED, "")));
        assert_eq!(
            r.entries,
            vec![e("Velloria")],
            "a revoked key keeps the copy"
        );
        // An old server (no route) or auth off: no team list at all.
        let r = c.refresh_with(SERVER, |_| Err(AdminError::new(code::UNSUPPORTED, "")));
        assert_eq!((r.entries.is_empty(), r.changed), (true, true));
        assert!(c.load(SERVER).is_empty());
    }

    #[test]
    fn another_servers_cache_is_ignored_and_bad_entries_dropped() {
        let (_d, c) = cache();
        c.store(SERVER, &list(&["Velloria"]), Some("\"t\"".into()))
            .unwrap();
        assert!(c.load("198.51.100.7:7313").is_empty());
        // No ETag is sent for another server's copy.
        let r = c.refresh_with("198.51.100.7:7313", |etag| {
            assert_eq!(etag, None);
            Ok(Some((list(&["Orvane", "<b>x</b>", "orvane"]), None)))
        });
        assert_eq!(r.entries, vec![e("Orvane")]);
        // A broken file is an empty cache.
        std::fs::write(c.path(), "not json").unwrap();
        assert!(c.load(SERVER).is_empty());
    }
}
