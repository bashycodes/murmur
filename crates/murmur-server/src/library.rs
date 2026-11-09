//! Model library (D40): serve owner-tagged model files to clients.
//!
//! The owner copies model folders into the library folder (`[library] dir`,
//! default `<state dir>/library`) and tags each with a `model.toml`
//! (`docs/server-advanced.md` §10). One folder = one model; its name is the
//! model id. Untagged or invalid folders are skipped with a reason
//! (`flow-server library list`, and the log when the set changes).
//!
//! Interface: [`Library::rescan`] (blocking: stat every file, hash what
//! changed, swap the new [`Index`] in), [`Library::index`] (the current
//! index, never blocks on hashing), and the HTTP routes ([`routes`]):
//! `GET /v1/models` and `GET /v1/models/{id}/files/{name}` with `Range`.
//!
//! Safety: a download is looked up by `(id, name)` in the index; the path
//! served is the one the scan resolved, never one built from the request.
//! Every path is canonicalised and must stay inside the library (symlinks
//! inside it are followed, ones leading out are refused), and is checked
//! again when opened.
//!
//! SHA-256 per file is cached in `<state dir>/library-hashes.json` keyed by
//! the file's real path and reused while its size and mtime are unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use flow_protocol::library::{
    catalogue, is_safe_name, LibraryFile, LibraryModel, ModelKind, ModelList,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppState;

/// The tag file in each model folder.
pub const TAG_FILE: &str = "model.toml";
/// The hash cache, in the state dir.
pub const HASH_CACHE_FILE: &str = "library-hashes.json";
/// Subfolders below a model folder.
const MAX_DEPTH: usize = 4;
const MAX_FILES: usize = 512;
const CHUNK: usize = 256 * 1024;
const CACHE_VERSION: u32 = 1;

/// `[library]` in the config.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LibraryConfig {
    /// The library folder. Default: `<state dir>/library`.
    pub dir: Option<PathBuf>,
    /// Seconds between rescans (stat only; files are hashed again only
    /// when their size or mtime changed). 0 = scan once at start.
    pub rescan_s: u64,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            dir: None,
            rescan_s: 60,
        }
    }
}

/// `model.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tag {
    kind: ModelKind,
    title: String,
    language: Option<String>,
    catalogue_id: Option<String>,
    engine: Option<String>,
}

/// A file's size and mtime: the cache entry is valid while these match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    size: u64,
    mtime_s: u64,
    mtime_ns: u32,
}

fn stamp(m: &std::fs::Metadata) -> Stamp {
    let t = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .unwrap_or_default();
    Stamp {
        size: m.len(),
        mtime_s: t.as_secs(),
        mtime_ns: t.subsec_nanos(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheEntry {
    #[serde(flatten)]
    stamp: Stamp,
    sha256: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    #[serde(default)]
    files: BTreeMap<String, CacheEntry>,
}

/// One served model: its public listing plus the resolved paths.
#[derive(Debug, Clone)]
pub struct Model {
    pub info: LibraryModel,
    /// Real paths, parallel to `info.files`.
    paths: Vec<PathBuf>,
    stamps: Vec<Stamp>,
}

/// A folder that is not served, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub folder: String,
    pub reason: String,
}

/// The result of one scan.
#[derive(Debug, Default)]
pub struct Index {
    pub models: Vec<Model>,
    pub skipped: Vec<Skipped>,
    /// The library folder does not exist (or is unreadable): `Some(why)`.
    pub missing: Option<String>,
    /// Files whose SHA-256 this scan computed (cache misses).
    pub hashed: usize,
    /// The library's real path (downloads must stay inside it).
    root: Option<PathBuf>,
}

impl Index {
    /// The `GET /v1/models` body.
    pub fn list(&self) -> ModelList {
        ModelList {
            models: self.models.iter().map(|m| m.info.clone()).collect(),
        }
    }

    fn file(&self, id: &str, name: &str) -> Option<(PathBuf, Stamp, String)> {
        let m = self.models.iter().find(|m| m.info.id == id)?;
        let i = m.info.files.iter().position(|f| f.name == name)?;
        Some((
            m.paths[i].clone(),
            m.stamps[i],
            m.info.files[i].sha256.clone(),
        ))
    }

    /// Same models, files, hashes and skips (for "log only on change").
    fn same(&self, o: &Index) -> bool {
        self.missing == o.missing
            && self.skipped == o.skipped
            && self.models.len() == o.models.len()
            && self
                .models
                .iter()
                .zip(&o.models)
                .all(|(a, b)| a.info == b.info)
    }
}

/// The library: its folder, the hash cache and the current index.
pub struct Library {
    dir: PathBuf,
    cache_path: Option<PathBuf>,
    cache: Mutex<BTreeMap<String, CacheEntry>>,
    index: RwLock<Arc<Index>>,
}

impl Library {
    /// A library over `dir`, caching hashes in `cache_path` (None: memory
    /// only). Nothing is scanned until [`Library::rescan`].
    pub fn new(dir: PathBuf, cache_path: Option<PathBuf>) -> Self {
        let cache = match &cache_path {
            Some(p) => match crate::keys::read_json::<CacheFile>(p) {
                Ok(Some(c)) if c.version == CACHE_VERSION => c.files,
                Ok(_) => BTreeMap::new(),
                Err(e) => {
                    tracing::warn!("library hash cache (rebuilt): {e}");
                    BTreeMap::new()
                }
            },
            None => BTreeMap::new(),
        };
        Self {
            dir,
            cache_path,
            cache: Mutex::new(cache),
            index: RwLock::new(Arc::new(Index::default())),
        }
    }

    /// From the config: `[library] dir`, else `<state dir>/library`; the
    /// cache lives in the state dir. None without either.
    pub fn from_config(cfg: &crate::Config) -> Option<Self> {
        let state = cfg.keys_dir();
        let dir = cfg
            .library
            .dir
            .clone()
            .or_else(|| state.as_ref().map(|s| s.join("library")))?;
        Some(Self::new(dir, state.map(|s| s.join(HASH_CACHE_FILE))))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The current index (cheap; never waits for a scan).
    pub fn index(&self) -> Arc<Index> {
        self.index.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Scan the folder now (blocking: run it off the async threads), swap
    /// the result in and save the hash cache if it changed. Logs the
    /// models and skipped folders when they differ from the last scan.
    pub fn rescan(&self) -> Arc<Index> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let (idx, changed) = scan(&self.dir, &mut cache);
        if changed {
            if let Some(p) = &self.cache_path {
                let f = CacheFile {
                    version: CACHE_VERSION,
                    files: cache.clone(),
                };
                let r = p
                    .parent()
                    .map_or(Ok(()), crate::keys::create_dir)
                    .and_then(|_| crate::keys::write_json(p, &f));
                if let Err(e) = r {
                    tracing::warn!("library hash cache: {e}");
                }
            }
        }
        drop(cache);
        let idx = Arc::new(idx);
        let old = std::mem::replace(
            &mut *self.index.write().unwrap_or_else(|e| e.into_inner()),
            idx.clone(),
        );
        if !old.same(&idx) {
            log(&self.dir, &idx);
        }
        idx
    }
}

fn log(dir: &Path, idx: &Index) {
    if let Some(why) = &idx.missing {
        tracing::info!(dir = %dir.display(), "model library: no models served ({why})");
        return;
    }
    let ids: Vec<&str> = idx.models.iter().map(|m| m.info.id.as_str()).collect();
    tracing::info!(
        dir = %dir.display(),
        models = ids.len(),
        hashed = idx.hashed,
        "model library: serving [{}]",
        ids.join(", ")
    );
    for s in &idx.skipped {
        tracing::warn!(folder = %s.folder, "model library: skipped: {}", s.reason);
    }
}

/// Canonicalise `p`; it must lie inside `root` (and not be `root`).
fn resolve(root: &Path, p: &Path) -> Result<(PathBuf, std::fs::Metadata), String> {
    let c = std::fs::canonicalize(p).map_err(|e| e.to_string())?;
    if c == root || !c.starts_with(root) {
        return Err("a symlink that leads out of the library".into());
    }
    let m = std::fs::metadata(&c).map_err(|e| e.to_string())?;
    Ok((c, m))
}

fn sorted_dir(dir: &Path) -> Result<Vec<std::fs::DirEntry>, String> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    v.sort_by_key(|e| e.file_name());
    Ok(v)
}

/// Scan `dir`. Returns the index and whether `cache` changed.
fn scan(dir: &Path, cache: &mut BTreeMap<String, CacheEntry>) -> (Index, bool) {
    let mut idx = Index::default();
    let entries = match std::fs::canonicalize(dir) {
        Ok(r) if r.is_dir() => sorted_dir(&r).map(|e| (r, e)),
        Ok(_) => Err("not a folder".into()),
        Err(e) => Err(e.to_string()),
    };
    let (root, entries) = match entries {
        Ok(x) => x,
        Err(e) => {
            idx.missing = Some(e);
            return (idx, false);
        }
    };
    let mut changed = false;
    let mut seen = BTreeSet::new();
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let mut s = Scan {
            root: &root,
            cache,
            seen: &mut seen,
            changed: &mut changed,
            hashed: &mut idx.hashed,
        };
        match s.model(&e.path(), &name) {
            Ok(m) => idx.models.push(m),
            Err(reason) => idx.skipped.push(Skipped {
                folder: name,
                reason,
            }),
        }
    }
    let before = cache.len();
    cache.retain(|k, _| seen.contains(k));
    changed |= cache.len() != before;
    idx.root = Some(root);
    (idx, changed)
}

struct Scan<'a> {
    root: &'a Path,
    cache: &'a mut BTreeMap<String, CacheEntry>,
    seen: &'a mut BTreeSet<String>,
    changed: &'a mut bool,
    hashed: &'a mut usize,
}

impl Scan<'_> {
    fn model(&mut self, path: &Path, id: &str) -> Result<Model, String> {
        let (dir, md) = resolve(self.root, path)?;
        if !md.is_dir() {
            return Err(format!(
                "not a folder (put a single-file model in its own folder with a {TAG_FILE})"
            ));
        }
        if !is_safe_name(id) {
            return Err(
                "the folder name is the model id: use only A-Z a-z 0-9 . _ - (at most 128)".into(),
            );
        }
        let tag_path = dir.join(TAG_FILE);
        if std::fs::symlink_metadata(&tag_path).is_err() {
            return Err(format!("untagged: no {TAG_FILE}"));
        }
        let (tag_path, _) =
            resolve(self.root, &tag_path).map_err(|e| format!("{TAG_FILE}: {e}"))?;
        let text = std::fs::read_to_string(&tag_path).map_err(|e| format!("{TAG_FILE}: {e}"))?;
        let tag = parse_tag(&text)?;
        let mut found = Vec::new();
        self.walk(&dir, "", 0, &mut found)?;
        if found.is_empty() {
            return Err(format!("no model files next to {TAG_FILE}"));
        }
        let (mut files, mut paths, mut stamps) = (Vec::new(), Vec::new(), Vec::new());
        for (name, p, md) in found {
            let st = stamp(&md);
            let sha = self.sha256(&p, st).map_err(|e| format!("{name}: {e}"))?;
            files.push(LibraryFile {
                name,
                size: st.size,
                sha256: sha,
            });
            paths.push(p);
            stamps.push(st);
        }
        Ok(Model {
            info: LibraryModel {
                id: id.into(),
                title: tag.title,
                kind: tag.kind,
                language: tag.language,
                // A catalogue copy needs no `engine`: the catalogue knows it.
                engine: tag.engine.or_else(|| {
                    let c = catalogue(tag.catalogue_id.as_deref()?)?;
                    Some(c.engine.into())
                }),
                catalogue_id: tag.catalogue_id,
                files,
            },
            paths,
            stamps,
        })
    }

    fn walk(
        &mut self,
        dir: &Path,
        prefix: &str,
        depth: usize,
        out: &mut Vec<(String, PathBuf, std::fs::Metadata)>,
    ) -> Result<(), String> {
        for e in sorted_dir(dir)? {
            let os = e.file_name();
            let n = os.to_str().ok_or("a file name that is not UTF-8")?;
            if n.starts_with('.') || (depth == 0 && n == TAG_FILE) {
                continue;
            }
            let rel = if prefix.is_empty() {
                n.to_string()
            } else {
                format!("{prefix}/{n}")
            };
            if !is_safe_name(n) {
                return Err(format!(
                    "{rel:?}: file names may use only A-Z a-z 0-9 . _ -"
                ));
            }
            let (p, md) = resolve(self.root, &e.path()).map_err(|r| format!("{rel}: {r}"))?;
            if md.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    return Err(format!("{rel}: folders nested deeper than {MAX_DEPTH}"));
                }
                self.walk(&p, &rel, depth + 1, out)?;
            } else if md.is_file() {
                out.push((rel, p, md));
                if out.len() > MAX_FILES {
                    return Err(format!("more than {MAX_FILES} files"));
                }
            }
        }
        Ok(())
    }

    fn sha256(&mut self, p: &Path, st: Stamp) -> std::io::Result<String> {
        let key = p.to_string_lossy().into_owned();
        self.seen.insert(key.clone());
        if let Some(c) = self.cache.get(&key).filter(|c| c.stamp == st) {
            return Ok(c.sha256.clone());
        }
        let sha = sha256_file(p)?;
        *self.hashed += 1;
        *self.changed = true;
        self.cache.insert(
            key,
            CacheEntry {
                stamp: st,
                sha256: sha.clone(),
            },
        );
        Ok(sha)
    }
}

fn parse_tag(text: &str) -> Result<Tag, String> {
    let t: Tag = toml::from_str(text).map_err(|e| format!("{TAG_FILE}: {}", e.message()))?;
    let title = t.title.trim();
    if title.is_empty() || title.chars().count() > 200 || title.chars().any(char::is_control) {
        return Err(format!(
            "{TAG_FILE}: title must be 1-200 printable characters"
        ));
    }
    for (field, v) in [
        ("language", &t.language),
        ("catalogue_id", &t.catalogue_id),
        ("engine", &t.engine),
    ] {
        if let Some(v) = v.as_deref().filter(|v| !is_safe_name(v)) {
            return Err(format!(
                "{TAG_FILE}: {field} {v:?} may use only A-Z a-z 0-9 . _ -"
            ));
        }
    }
    Ok(Tag {
        title: title.into(),
        ..t
    })
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Rescan every `interval` until `kill` (the first scan runs at once).
pub(crate) fn spawn_rescan(
    lib: Arc<Library>,
    interval: Duration,
    mut kill: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        loop {
            let l = lib.clone();
            let _ = tokio::task::spawn_blocking(move || l.rescan()).await;
            if interval.is_zero() {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = kill.changed() => return,
            }
        }
    });
}

// ---- HTTP ----

pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(flow_protocol::library::PATH_MODELS, get(list_handler))
        .route("/v1/models/{id}/files/{*name}", get(file_handler))
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

async fn list_handler(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if crate::authorize(&state, &headers).is_err() {
        return unauthorized();
    }
    let list = state
        .library
        .as_ref()
        .map(|l| l.index().list())
        .unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&list).unwrap(),
    )
        .into_response()
}

/// A request's `Range` against a file of `size` bytes (RFC 9110 §14).
#[derive(Debug, PartialEq, Eq)]
pub enum ByteRange {
    Full,
    /// Inclusive.
    Part(u64, u64),
    Unsatisfiable,
}

/// One `bytes=` range; anything else (several ranges, other units, bad
/// syntax) is ignored and the whole file is sent, as the RFC allows.
pub fn parse_range(h: Option<&str>, size: u64) -> ByteRange {
    let Some(spec) = h.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return ByteRange::Full;
    };
    let Some((a, b)) = spec.split_once('-').filter(|_| !spec.contains(',')) else {
        return ByteRange::Full;
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        return match b.parse::<u64>() {
            Ok(0) => ByteRange::Unsatisfiable,
            Ok(_) if size == 0 => ByteRange::Unsatisfiable,
            Ok(n) => ByteRange::Part(size - n.min(size), size - 1),
            Err(_) => ByteRange::Full,
        };
    }
    let Ok(start) = a.parse::<u64>() else {
        return ByteRange::Full;
    };
    let end = if b.is_empty() {
        u64::MAX
    } else {
        match b.parse::<u64>() {
            Ok(e) if e >= start => e,
            _ => return ByteRange::Full,
        }
    };
    if start >= size {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Part(start, end.min(size - 1))
}

/// Counts what a download sent; records it on the key when dropped
/// (finished or cut off).
struct Counted {
    key: Option<(Arc<crate::keys::KeyStore>, String)>,
    sent: u64,
}

impl Drop for Counted {
    fn drop(&mut self) {
        if let (Some((k, id)), true) = (&self.key, self.sent > 0) {
            k.record_download(id, self.sent);
        }
    }
}

async fn file_handler(
    State(state): State<Arc<AppState>>,
    UrlPath((id, name)): UrlPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Ok(key) = crate::authorize(&state, &headers) else {
        return unauthorized();
    };
    let not_found = || (StatusCode::NOT_FOUND, "no such model file").into_response();
    let Some(lib) = &state.library else {
        return not_found();
    };
    let idx = lib.index();
    let (Some((path, st, sha)), Some(root)) = (idx.file(&id, &name), idx.root.clone()) else {
        return not_found();
    };
    // Re-check at open time: still the same real path inside the library,
    // still the bytes the listed hash describes.
    let p = path.clone();
    let opened = tokio::task::spawn_blocking(move || {
        let c = std::fs::canonicalize(&p).ok()?;
        if c != p || !c.starts_with(&root) {
            return None;
        }
        let f = std::fs::File::open(&c).ok()?;
        let now = stamp(&f.metadata().ok()?);
        Some((f, now))
    })
    .await
    .ok()
    .flatten();
    let Some((file, now)) = opened else {
        return not_found();
    };
    if now != st {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "60")],
            "file changed since the last library scan; retry after the next rescan",
        )
            .into_response();
    }
    let size = st.size;
    let etag = format!("\"{sha}\"");
    let range_h = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    // If-Range with another validator: the client's partial copy is of
    // other bytes; send the whole file.
    let if_range_ok = headers
        .get(header::IF_RANGE)
        .is_none_or(|v| v.as_bytes() == etag.as_bytes());
    let range = if if_range_ok {
        parse_range(range_h, size)
    } else {
        ByteRange::Full
    };
    let (status, start, len, content_range) = match range {
        ByteRange::Full => (StatusCode::OK, 0, size, None),
        ByteRange::Part(a, b) => (
            StatusCode::PARTIAL_CONTENT,
            a,
            b - a + 1,
            Some(format!("bytes {a}-{b}/{size}")),
        ),
        ByteRange::Unsatisfiable => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
                "",
            )
                .into_response()
        }
    };
    let mut file = tokio::fs::File::from_std(file);
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "seek failed").into_response();
        }
    }
    let counted = Counted {
        key: key.and_then(|k| state.keys.clone().map(|s| (s, k.id))),
        sent: 0,
    };
    let stream =
        futures_util::stream::unfold((file, len, counted), |(mut f, rem, mut c)| async move {
            use tokio::io::AsyncReadExt;
            if rem == 0 {
                return None;
            }
            let mut buf = vec![0u8; rem.min(CHUNK as u64) as usize];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    c.sent += n as u64;
                    Some((
                        Ok::<_, std::io::Error>(Bytes::from(buf)),
                        (f, rem - n as u64, c),
                    ))
                }
                Err(e) => Some((Err(e), (f, 0, c))),
            }
        });
    let mut r = Response::new(Body::from_stream(stream));
    *r.status_mut() = status;
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    if let Some(cr) = content_range.and_then(|c| HeaderValue::from_str(&c).ok()) {
        h.insert(header::CONTENT_RANGE, cr);
    }
    r
}

/// `flow-server library list`: the served models, then the skipped folders.
pub fn table(dir: &Path, idx: &Index) -> String {
    let mut out = format!("library: {}\n", dir.display());
    if let Some(why) = &idx.missing {
        out.push_str(&format!("no models served: {why}\n"));
        return out;
    }
    let mut rows = vec![[
        "ID",
        "KIND",
        "LANG",
        "ENGINE",
        "CATALOGUE",
        "FILES",
        "SIZE MB",
        "TITLE",
    ]
    .map(String::from)];
    for m in &idx.models {
        let i = &m.info;
        let dash = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
        rows.push([
            i.id.clone(),
            i.kind.as_str().into(),
            dash(&i.language),
            dash(&i.engine),
            dash(&i.catalogue_id),
            i.files.len().to_string(),
            format!("{:.1}", i.size() as f64 / 1e6),
            i.title.clone(),
        ]);
    }
    let mut w = [0usize; 8];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    for r in &rows {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<width$}", width = w[i]))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    for s in &idx.skipped {
        out.push_str(&format!("skipped {}: {}\n", s.folder, s.reason));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn hex_of(b: &[u8]) -> String {
        Sha256::digest(b)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    const WHISPER: &str = "kind = \"stt\"\ntitle = \"Whisper base\"\ncatalogue_id = \"whisper-base\"\nengine = \"whisper\"\n";

    #[test]
    fn tagged_folders_are_listed_with_hashes() {
        let d = tempfile::tempdir().unwrap();
        put(&d.path().join("whisper-base/model.toml"), WHISPER);
        put(&d.path().join("whisper-base/ggml-base.bin"), "weights");
        put(
            &d.path().join("zip-he/model.toml"),
            "kind = \"hebrew\"\ntitle = \"Zipformer Hebrew\"\nlanguage = \"he\"\nengine = \"sherpa_online\"\n",
        );
        put(&d.path().join("zip-he/tokens.txt"), "a b");
        put(&d.path().join("zip-he/sub/encoder.onnx"), "enc");
        put(&d.path().join("zip-he/.DS_Store"), "junk");
        let (idx, changed) = scan(d.path(), &mut BTreeMap::new());
        assert!(changed);
        assert!(idx.skipped.is_empty(), "{:?}", idx.skipped);
        let l = idx.list();
        assert_eq!(l.models.len(), 2);
        let w = &l.models[0];
        assert_eq!(
            (
                w.id.as_str(),
                w.kind,
                w.catalogue_id.as_deref(),
                w.engine.as_deref()
            ),
            (
                "whisper-base",
                ModelKind::Stt,
                Some("whisper-base"),
                Some("whisper")
            )
        );
        assert_eq!(w.files.len(), 1, "model.toml is not a model file");
        assert_eq!(w.files[0].sha256, hex_of(b"weights"));
        assert_eq!(w.files[0].size, 7);
        let z = &l.models[1];
        assert_eq!(
            (z.kind, z.language.as_deref()),
            (ModelKind::Hebrew, Some("he"))
        );
        let names: Vec<&str> = z.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["sub/encoder.onnx", "tokens.txt"]);
    }

    #[test]
    fn a_catalogue_id_alone_gives_the_engine() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(
            &r.join("pk/model.toml"),
            "kind = \"stt\"\ntitle = \"P\"\ncatalogue_id = \"parakeet-unified-en-0.6b-int8\"\n",
        );
        put(&r.join("pk/tokens.txt"), "a");
        // The tag's own engine wins; an unknown id infers nothing.
        put(
            &r.join("own/model.toml"),
            "kind = \"stt\"\ntitle = \"O\"\ncatalogue_id = \"whisper-base\"\nengine = \"custom\"\n",
        );
        put(&r.join("own/a.bin"), "a");
        put(
            &r.join("unknown/model.toml"),
            "kind = \"stt\"\ntitle = \"U\"\ncatalogue_id = \"not-in-the-catalogue\"\n",
        );
        put(&r.join("unknown/a.bin"), "a");
        let (idx, _) = scan(r, &mut BTreeMap::new());
        let engine: BTreeMap<&str, Option<&str>> = idx
            .models
            .iter()
            .map(|m| (m.info.id.as_str(), m.info.engine.as_deref()))
            .collect();
        assert_eq!(engine["pk"], Some("parakeet"));
        assert_eq!(engine["own"], Some("custom"));
        assert_eq!(engine["unknown"], None);
    }

    #[test]
    fn untagged_and_invalid_folders_are_skipped_with_reasons() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        put(&r.join("untagged/a.bin"), "x");
        put(
            &r.join("badkind/model.toml"),
            "kind = \"tts\"\ntitle = \"t\"\n",
        );
        put(&r.join("badkind/a.bin"), "x");
        put(
            &r.join("typo/model.toml"),
            "kind = \"llm\"\ntitle = \"t\"\nlangauge = \"en\"\n",
        );
        put(&r.join("typo/a.gguf"), "x");
        put(
            &r.join("notitle/model.toml"),
            "kind = \"vad\"\ntitle = \" \"\n",
        );
        put(&r.join("notitle/a.onnx"), "x");
        put(
            &r.join("empty/model.toml"),
            "kind = \"lid\"\ntitle = \"t\"\n",
        );
        put(&r.join("bad name/model.toml"), WHISPER);
        put(&r.join("badfile/model.toml"), WHISPER);
        put(&r.join("badfile/a b.bin"), "x");
        put(&r.join("loose.gguf"), "x");
        put(&r.join(".hidden/model.toml"), WHISPER);
        let (idx, _) = scan(r, &mut BTreeMap::new());
        assert!(idx.models.is_empty());
        let why: BTreeMap<&str, &str> = idx
            .skipped
            .iter()
            .map(|s| (s.folder.as_str(), s.reason.as_str()))
            .collect();
        assert_eq!(why.len(), 8, "{why:?}");
        assert!(why["untagged"].contains("untagged"), "{why:?}");
        assert!(why["badkind"].contains("tts"), "{why:?}");
        assert!(why["typo"].contains("langauge"), "{why:?}");
        assert!(why["notitle"].contains("title"), "{why:?}");
        assert!(why["empty"].contains("no model files"), "{why:?}");
        assert!(why["bad name"].contains("model id"), "{why:?}");
        assert!(why["badfile"].contains("a b.bin"), "{why:?}");
        assert!(why["loose.gguf"].contains("own folder"), "{why:?}");
        let t = table(r, &idx);
        assert!(t.contains("skipped untagged: untagged"), "{t}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_library_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        put(&d.path().join("secret.txt"), "secret");
        put(&d.path().join("outside/model.toml"), WHISPER);
        put(&d.path().join("outside/a.bin"), "x");
        put(&lib.join("leak/model.toml"), WHISPER);
        std::os::unix::fs::symlink(d.path().join("secret.txt"), lib.join("leak/a.bin")).unwrap();
        std::os::unix::fs::symlink(d.path().join("outside"), lib.join("outdir")).unwrap();
        // A symlink inside the library is fine.
        put(&lib.join("real/model.toml"), WHISPER);
        put(&lib.join("real/w.bin"), "w");
        std::os::unix::fs::symlink(lib.join("real"), lib.join("alias")).unwrap();
        let (idx, _) = scan(&lib, &mut BTreeMap::new());
        let ids: Vec<&str> = idx.models.iter().map(|m| m.info.id.as_str()).collect();
        assert_eq!(ids, ["alias", "real"]);
        let skipped: Vec<&str> = idx.skipped.iter().map(|s| s.folder.as_str()).collect();
        assert_eq!(skipped, ["leak", "outdir"]);
        assert!(idx
            .skipped
            .iter()
            .all(|s| s.reason.contains("out of the library")));
    }

    #[test]
    fn hash_cache_is_reused_until_size_or_mtime_change() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        let cache = d.path().join("state").join(HASH_CACHE_FILE);
        put(&lib.join("m/model.toml"), WHISPER);
        put(&lib.join("m/a.bin"), "one");
        put(&lib.join("m/b.bin"), "two");
        let l = Library::new(lib.clone(), Some(cache.clone()));
        assert_eq!(l.rescan().hashed, 2);
        assert!(cache.is_file());
        assert_eq!(l.rescan().hashed, 0, "unchanged files are not read again");
        // A fresh process reuses the saved cache.
        let l = Library::new(lib.clone(), Some(cache.clone()));
        assert_eq!(l.rescan().hashed, 0);
        // Same size, new content and mtime: hashed again.
        let f = std::fs::File::options()
            .write(true)
            .open(lib.join("m/a.bin"))
            .unwrap();
        std::io::Write::write_all(&mut &f, b"ONE").unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
            .unwrap();
        drop(f);
        let idx = l.rescan();
        assert_eq!(idx.hashed, 1);
        assert_eq!(idx.models[0].info.files[0].sha256, hex_of(b"ONE"));
        // Removed files leave the cache.
        std::fs::remove_file(lib.join("m/b.bin")).unwrap();
        l.rescan();
        let saved: CacheFile = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
        assert_eq!(saved.files.len(), 1);
    }

    #[test]
    fn missing_library_serves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::new(d.path().join("nope"), None);
        let idx = l.rescan();
        assert!(idx.missing.is_some() && idx.models.is_empty());
        assert!(table(l.dir(), &idx).contains("no models served"));
    }

    #[test]
    fn ranges() {
        use ByteRange::*;
        let p = |h: &str| parse_range(Some(h), 100);
        assert_eq!(parse_range(None, 100), Full);
        assert_eq!(p("bytes=0-9"), Part(0, 9));
        assert_eq!(p("bytes=90-"), Part(90, 99));
        assert_eq!(p("bytes=90-500"), Part(90, 99));
        assert_eq!(p("bytes=-10"), Part(90, 99));
        assert_eq!(p("bytes=-500"), Part(0, 99));
        assert_eq!(p("bytes=100-"), Unsatisfiable);
        assert_eq!(p("bytes=-0"), Unsatisfiable);
        assert_eq!(p("bytes=0-1,5-6"), Full);
        assert_eq!(p("bytes=9-1"), Full);
        assert_eq!(p("items=0-1"), Full);
        assert_eq!(p("bytes=x-1"), Full);
        assert_eq!(parse_range(Some("bytes=0-"), 0), Unsatisfiable);
    }
}
