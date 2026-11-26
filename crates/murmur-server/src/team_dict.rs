//! The team dictionary (`murmur_protocol::dictionary`): one shared word
//! list per server that every client downloads and merges under its own
//! dictionary. Any active key reads it (`GET /v1/dictionary`, with an ETag);
//! admin keys add, replace and remove entries (`/v1/admin/dictionary`).
//!
//! Stored as `team-dictionary.json` next to `keys.json` in the state dir
//! (mode 0600, written atomically), or only in memory when the key store
//! is. Only the server writes it, under the store's lock. A file that does
//! not parse is logged as an error and served as an empty list, and every
//! edit is refused (500 `store_corrupt`) so the file is never overwritten:
//! the admin repairs or removes it and restarts the server.
//! The words are the admin's, not dictation text, but the log still gets
//! only counts.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use murmur_core::postprocess::word_key;
use murmur_protocol::admin::code;
use murmur_protocol::dictionary::{
    check_entry, sanitize, RemoveTeamEntry, TeamDictionary, TeamEntry, CODE_STORE_CORRUPT,
    CODE_TOO_MANY, MAX_EDIT_BODY_BYTES, MAX_ENTRIES, PATH_ADMIN_DICTIONARY, PATH_DICTIONARY,
};

use crate::AppState;

pub const TEAM_DICTIONARY_FILE: &str = "team-dictionary.json";

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// Not a term-like entry (the message says why).
    Bad(String),
    /// The list already holds [`MAX_ENTRIES`].
    TooMany,
    /// No entry with that word.
    NotFound(String),
    /// The file could not be written; nothing changed.
    Io(String),
    /// The file did not parse at start: edits would overwrite it.
    Corrupt,
}

struct Current {
    list: TeamDictionary,
    etag: String,
}

/// The team list and its ETag. File-backed or in memory.
pub struct TeamDictStore {
    path: Option<PathBuf>,
    /// The file did not parse: serve empty, refuse every edit.
    corrupt: bool,
    inner: Mutex<Current>,
}

fn etag_of(list: &TeamDictionary) -> String {
    let body = serde_json::to_string(list).expect("team dictionary serialises");
    format!("\"{}\"", &crate::keys::sha256_hex(&body)[..16])
}

fn current(list: TeamDictionary) -> Current {
    Current {
        etag: etag_of(&list),
        list,
    }
}

impl TeamDictStore {
    /// The list in `dir` (`None` = memory only). A missing file is an
    /// empty list; an unreadable one is logged, served empty and never
    /// overwritten ([`EditError::Corrupt`]).
    pub fn open(dir: Option<&Path>) -> Self {
        let path = dir.map(|d| d.join(TEAM_DICTIONARY_FILE));
        let mut corrupt = false;
        let list = match path
            .as_deref()
            .map(crate::keys::read_json::<TeamDictionary>)
        {
            Some(Ok(Some(l))) => TeamDictionary {
                entries: sanitize(&l.entries),
            },
            Some(Err(e)) => {
                tracing::error!(
                    "team dictionary: {e}; serving an empty list and refusing edits until the file is repaired or removed"
                );
                corrupt = true;
                TeamDictionary::default()
            }
            _ => TeamDictionary::default(),
        };
        if !list.entries.is_empty() {
            tracing::info!(entries = list.entries.len(), "team dictionary");
        }
        Self {
            path,
            corrupt,
            inner: Mutex::new(current(list)),
        }
    }

    /// `team-dictionary.json` (None in memory).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Current> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The list and its ETag.
    pub fn get(&self) -> (TeamDictionary, String) {
        let g = self.lock();
        (g.list.clone(), g.etag.clone())
    }

    /// Save `list` (file first, so a failed write changes nothing).
    fn commit(&self, g: &mut Current, list: TeamDictionary) -> Result<(), EditError> {
        if self.corrupt {
            return Err(EditError::Corrupt);
        }
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                crate::keys::create_dir(d).map_err(EditError::Io)?;
            }
            crate::keys::write_json(p, &list).map_err(EditError::Io)?;
        }
        *g = current(list);
        Ok(())
    }

    /// Add `e`, or replace the entry with the same word (case and spacing
    /// ignored) in place.
    pub fn upsert(&self, e: &TeamEntry) -> Result<(TeamDictionary, String), EditError> {
        let e = check_entry(e).map_err(EditError::Bad)?;
        if self.corrupt {
            return Err(EditError::Corrupt);
        }
        let mut g = self.lock();
        let mut list = g.list.clone();
        let k = word_key(&e.word);
        match list.entries.iter().position(|x| word_key(&x.word) == k) {
            Some(i) if list.entries[i] == e => return Ok((g.list.clone(), g.etag.clone())),
            Some(i) => list.entries[i] = e,
            None if list.entries.len() >= MAX_ENTRIES => return Err(EditError::TooMany),
            None => list.entries.push(e),
        }
        self.commit(&mut g, list)?;
        Ok((g.list.clone(), g.etag.clone()))
    }

    /// Remove the entry with `word` (case and spacing ignored).
    pub fn remove(&self, word: &str) -> Result<(TeamDictionary, String), EditError> {
        if self.corrupt {
            return Err(EditError::Corrupt);
        }
        let k = word_key(word);
        let mut g = self.lock();
        let mut list = g.list.clone();
        let before = list.entries.len();
        list.entries.retain(|x| word_key(&x.word) != k);
        if k.is_empty() || list.entries.len() == before {
            return Err(EditError::NotFound(word.trim().to_string()));
        }
        self.commit(&mut g, list)?;
        Ok((g.list.clone(), g.etag.clone()))
    }
}

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(PATH_DICTIONARY, get(read))
        .route(PATH_ADMIN_DICTIONARY, post(add).delete(remove))
        .layer(DefaultBodyLimit::max(MAX_EDIT_BODY_BYTES))
}

/// The list with its ETag. Clients revalidate (`no-cache`), never reuse
/// it blindly.
fn listed(status: StatusCode, list: Option<&TeamDictionary>, etag: &str) -> Response {
    let mut r = match list {
        Some(l) => (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_string(l).expect("team dictionary serialises"),
        )
            .into_response(),
        None => status.into_response(),
    };
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Ok(v) = HeaderValue::from_str(etag) {
        h.insert(header::ETAG, v);
    }
    r
}

fn edit_error(e: EditError) -> Response {
    let (status, c, m) = match e {
        EditError::Bad(m) => (StatusCode::BAD_REQUEST, code::BAD_REQUEST, m),
        EditError::TooMany => (
            StatusCode::BAD_REQUEST,
            CODE_TOO_MANY,
            format!("the team dictionary is full ({MAX_ENTRIES} entries); remove one first"),
        ),
        EditError::NotFound(w) => (
            StatusCode::NOT_FOUND,
            code::NOT_FOUND,
            format!("\"{w}\" is not in the team dictionary"),
        ),
        EditError::Corrupt => {
            tracing::error!(
                "team dictionary: edit refused, the list file does not parse; repair or remove it and restart"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CODE_STORE_CORRUPT,
                "the server's team dictionary file is damaged; the server admin must repair or remove it".into(),
            )
        }
        EditError::Io(m) => {
            tracing::warn!("team dictionary: {m}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                code::SERVER_ERROR,
                "the server could not write its team dictionary".into(),
            )
        }
    };
    crate::admin::error(status, c, m)
}

/// The store, or the refusal (`auth_off` / 401 / 429 / 403, as the admin API).
fn store(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    need_admin: bool,
) -> Result<(crate::keys::KeyRef, Arc<TeamDictStore>), Box<Response>> {
    let (key, _) = crate::admin::caller(state, peer, headers, need_admin)?;
    // Auth on always opens the store (see `serve_with_pool`).
    let s = state.team_dict.clone().ok_or_else(|| {
        Box::new(crate::admin::error(
            StatusCode::NOT_FOUND,
            code::AUTH_OFF,
            "this server runs without keys (auth = \"none\")",
        ))
    })?;
    Ok((key, s))
}

/// `If-None-Match` names `etag` (or `*`).
fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == etag || t == "*")
}

async fn read(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    let (_, s) = match store(&state, peer, &headers, false) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let (list, etag) = s.get();
    if not_modified(&headers, &etag) {
        return listed(StatusCode::NOT_MODIFIED, None, &etag);
    }
    listed(StatusCode::OK, Some(&list), &etag)
}

async fn edit(
    me: crate::keys::KeyRef,
    s: Arc<TeamDictStore>,
    what: &'static str,
    f: impl FnOnce(&TeamDictStore) -> Result<(TeamDictionary, String), EditError> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(move || f(&s)).await {
        Ok(Ok((list, etag))) => {
            tracing::info!(by = %me.id, entries = list.entries.len(), "team dictionary: {what}");
            listed(StatusCode::OK, Some(&list), &etag)
        }
        Ok(Err(e)) => edit_error(e),
        Err(e) => edit_error(EditError::Io(e.to_string())),
    }
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Box<Response>> {
    serde_json::from_slice(body).map_err(|e| {
        Box::new(crate::admin::error(
            StatusCode::BAD_REQUEST,
            code::BAD_REQUEST,
            e.to_string(),
        ))
    })
}

async fn add(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Auth first: a stranger's bad body is a 401, not a 400.
    let ((me, s), e) = match store(&state, peer, &headers, true)
        .and_then(|x| Ok((x, parse::<TeamEntry>(&body)?)))
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    edit(me, s, "entry saved", move |s| s.upsert(&e)).await
}

async fn remove(
    State(state): State<Arc<AppState>>,
    Extension(ConnectInfo(peer)): Extension<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ((me, s), r) = match store(&state, peer, &headers, true)
        .and_then(|x| Ok((x, parse::<RemoveTeamEntry>(&body)?)))
    {
        Ok(x) => x,
        Err(r) => return *r,
    };
    edit(me, s, "entry removed", move |s| s.remove(&r.word)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(w: &str, r: Option<&str>) -> TeamEntry {
        TeamEntry {
            word: w.into(),
            replacement: r.map(Into::into),
        }
    }

    #[test]
    fn upsert_replaces_the_same_word_in_place_and_changes_the_etag() {
        let s = TeamDictStore::open(None);
        let (_, t0) = s.get();
        s.upsert(&e("Zephyr", None)).unwrap();
        let (_, t1) = s.upsert(&e("acme", Some("AcmeCorp"))).unwrap();
        assert_ne!(t0, t1);
        let (l, t2) = s.upsert(&e(" ZEPHYR ", Some("ZephyrCo"))).unwrap();
        assert_eq!(
            l.entries,
            [e("ZEPHYR", Some("ZephyrCo")), e("acme", Some("AcmeCorp"))]
        );
        assert_ne!(t1, t2);
        // The same entry again: nothing changes.
        let (_, t3) = s.upsert(&e("ZEPHYR", Some("ZephyrCo"))).unwrap();
        assert_eq!(t2, t3);
        assert!(matches!(s.upsert(&e("<b>", None)), Err(EditError::Bad(_))));
        assert_eq!(s.remove("nope"), Err(EditError::NotFound("nope".into())));
        let (l, _) = s.remove("Zephyr").unwrap();
        assert_eq!(l.entries, [e("acme", Some("AcmeCorp"))]);
    }

    #[test]
    fn the_list_is_capped() {
        let s = TeamDictStore::open(None);
        for i in 0..MAX_ENTRIES {
            s.upsert(&e(&format!("w{i}"), None)).unwrap();
        }
        assert_eq!(s.upsert(&e("one more", None)), Err(EditError::TooMany));
        // Replacing an existing word still works when full.
        assert!(s.upsert(&e("w0", Some("W0"))).is_ok());
    }

    #[test]
    fn persists_and_reopens_with_the_same_etag() {
        let d = tempfile::tempdir().unwrap();
        let s = TeamDictStore::open(Some(d.path()));
        let (_, etag) = s.upsert(&e("Quillon", None)).unwrap();
        let again = TeamDictStore::open(Some(d.path()));
        assert_eq!(
            again.get(),
            (
                TeamDictionary {
                    entries: vec![e("Quillon", None)]
                },
                etag
            )
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(s.path().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o777, 0o600);
        }
        // A broken file is served empty and never overwritten.
        let file = d.path().join(TEAM_DICTIONARY_FILE);
        std::fs::write(&file, "not json").unwrap();
        let broken = TeamDictStore::open(Some(d.path()));
        assert!(broken.get().0.entries.is_empty());
        assert_eq!(broken.upsert(&e("Velloria", None)), Err(EditError::Corrupt));
        assert_eq!(broken.remove("Quillon"), Err(EditError::Corrupt));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json");
    }

    #[test]
    fn if_none_match_lists_and_weak_tags() {
        let mut h = HeaderMap::new();
        assert!(!not_modified(&h, "\"abc\""));
        h.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"x\", W/\"abc\""),
        );
        assert!(not_modified(&h, "\"abc\""));
        assert!(!not_modified(&h, "\"abd\""));
    }
}
