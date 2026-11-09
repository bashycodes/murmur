//! Named keys (D38): many bearer secrets instead of one shared token.
//!
//! A key is a secret handed to one person or device, with a name, a stable
//! opaque id, an admin flag and usage counters. The server stores only the
//! SHA-256 of each secret; the secret itself is printed once by
//! `flow-server keys add` and never written anywhere.
//!
//! Two files in the server's state directory, both mode 0600 and written
//! atomically (temp file + rename):
//!
//! * `keys.json`: the key records. Written by the `flow-server keys` CLI
//!   and by the server (admin API, D39; `token_file` import). Every
//!   read-modify-write holds an exclusive lock on `keys.json.lock` and
//!   re-reads the file under it, so two writers never lose each other's
//!   change. A running server re-reads it when it changes (one `stat` per
//!   authentication), so `keys add` / `keys revoke` take effect on the
//!   next connection without a restart.
//! * `key-usage.json`: per-key counters (last used, last connected,
//!   client, dictations, audio). Written only by the server, batched ([`KeyStore::flush`]).
//!   Never text or audio (D17).
//!
//! Interface: [`KeyStore::authenticate`] (secret → [`KeyRef`]),
//! [`KeyStore::add`], [`KeyStore::revoke`], [`KeyStore::record_client`] /
//! [`KeyStore::record_dictation`] / [`KeyStore::record_clean`], [`KeyStore::list`], [`KeyStore::flush`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const KEYS_FILE: &str = "keys.json";
pub const USAGE_FILE: &str = "key-usage.json";
/// Lock file serialising writers of `keys.json` (CLI and server, D39).
pub const LOCK_FILE: &str = "keys.json.lock";
/// The name the imported `token_file` secret gets.
pub const OWNER: &str = "owner";
/// `Key::origin` of the key imported from `token_file`.
const ORIGIN_TOKEN_FILE: &str = "token_file";
const FORMAT_VERSION: u32 = 1;

/// One key record as stored in `keys.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Key {
    /// Stable, opaque (`k-` + 16 random hex digits); never reused.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub admin: bool,
    /// SHA-256 of the secret, lowercase hex.
    pub hash: String,
    /// Unix seconds.
    pub created: u64,
    /// Unix seconds; a revoked key is kept (its id and usage stay) but
    /// never authenticates again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<u64>,
    /// `"token_file"` for the key imported from the old single token: its
    /// hash follows that file when it is regenerated (`gen-token`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl Key {
    pub fn active(&self) -> bool {
        self.revoked.is_none()
    }
}

/// Usage counters of one key (`key-usage.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Usage {
    /// Unix seconds of the last real use: a finished dictation, a
    /// text-only cleanup or a model download. Admin page visits and
    /// connection checks don't count.
    pub last_used: Option<u64>,
    /// Unix seconds of the last authenticated request of any kind. Files
    /// written before the split call it `last_seen`; `last_used` is then
    /// unknown until the key is used again.
    #[serde(alias = "last_seen")]
    pub last_connected: Option<u64>,
    /// `hello.client` of the last connection (e.g. `flow-android 0.12.0`).
    pub client: Option<String>,
    pub dictations: u64,
    pub audio_ms: u64,
    /// Model library downloads (D40): requests that sent bytes, and the
    /// bytes sent (a resumed download counts again, with its own bytes).
    pub downloads: u64,
    pub download_bytes: u64,
}

/// Who a request authenticated as. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRef {
    pub id: String,
    pub name: String,
    pub admin: bool,
}

impl From<&Key> for KeyRef {
    fn from(k: &Key) -> Self {
        Self {
            id: k.id.clone(),
            name: k.name.clone(),
            admin: k.admin,
        }
    }
}

/// Why a key operation failed. The admin API maps each to a status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// Bad key name (400).
    BadName(String),
    /// An active key already has this name (409).
    NameTaken(String),
    /// No active key with this name or id (404).
    NotFound(String),
    /// Revoking it would leave no active admin key (409); the CLI can
    /// override with `--force`.
    LastAdmin(String),
    /// Reading or writing the key files failed (500).
    Io(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::BadName(m)
            | KeyError::NameTaken(m)
            | KeyError::NotFound(m)
            | KeyError::LastAdmin(m)
            | KeyError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for KeyError {}

impl From<KeyError> for String {
    fn from(e: KeyError) -> Self {
        e.to_string()
    }
}

/// A key with its usage, for listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub key: Key,
    pub usage: Usage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct KeysFile {
    version: u32,
    /// Set once the `token_file` secret has been imported, so revoking
    /// `owner` is not undone by the next start.
    #[serde(default)]
    token_file_imported: bool,
    #[serde(default)]
    keys: Vec<Key>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageFile {
    version: u32,
    #[serde(default)]
    usage: BTreeMap<String, Usage>,
}

/// Identity of the keys file on disk; a rename always changes the inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    mtime: Option<SystemTime>,
    ino: u64,
}

fn stamp(p: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(p).ok()?;
    #[cfg(unix)]
    let ino = std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(not(unix))]
    let ino = 0;
    Some(Stamp {
        len: m.len(),
        mtime: m.modified().ok(),
        ino,
    })
}

struct Inner {
    keys: KeysFile,
    stamp: Option<Stamp>,
    usage: BTreeMap<String, Usage>,
    dirty: bool,
}

/// The key store. File-backed ([`KeyStore::open`]) or in memory
/// ([`KeyStore::in_memory`], for in-process servers and tests).
pub struct KeyStore {
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl KeyStore {
    /// Open (creating if needed) the store in `dir`. `token` is the old
    /// single token (`token_file`): imported once as the admin key
    /// `owner`, and when the file is regenerated, the `owner` key follows.
    pub fn open(dir: &Path, token: Option<&str>) -> Result<Self, String> {
        create_dir(dir)?;
        let path = dir.join(KEYS_FILE);
        // Held through the import write below, so a concurrent writer's
        // change is read here, not overwritten.
        let _lock = match token {
            Some(_) => Some(lock_file(dir)?),
            None => None,
        };
        let existing = read_json::<KeysFile>(&path)?;
        let st = stamp(&path);
        let mut keys = existing.unwrap_or(KeysFile {
            version: FORMAT_VERSION,
            ..Default::default()
        });
        let mut changed = false;
        if let Some(t) = token {
            changed = import_token(&mut keys, t);
        }
        let usage = read_json::<UsageFile>(&dir.join(USAGE_FILE))?
            .map(|u| u.usage)
            .unwrap_or_default();
        let s = Self {
            dir: Some(dir.to_path_buf()),
            inner: Mutex::new(Inner {
                keys,
                stamp: st,
                usage,
                dirty: false,
            }),
        };
        if changed {
            let mut g = s.lock();
            s.save_keys(&mut g)?;
        }
        Ok(s)
    }

    /// A store that lives only in memory, with `token` as its `owner` key.
    pub fn in_memory(token: &str) -> Self {
        let mut keys = KeysFile {
            version: FORMAT_VERSION,
            ..Default::default()
        };
        import_token(&mut keys, token);
        Self {
            dir: None,
            inner: Mutex::new(Inner {
                keys,
                stamp: None,
                usage: BTreeMap::new(),
                dirty: false,
            }),
        }
    }

    /// `keys.json` (None for an in-memory store).
    pub fn keys_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(KEYS_FILE))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a read-modify-write of `keys.json`: under the writers' file
    /// lock, re-read the file (whatever its stamp says), apply `f`, save
    /// when `f` succeeds. In memory there is no file and no lock.
    fn write_locked<T>(
        &self,
        f: impl FnOnce(&mut KeysFile) -> Result<T, KeyError>,
    ) -> Result<T, KeyError> {
        let mut g = self.lock();
        let _lock = match &self.dir {
            Some(d) => {
                let l = lock_file(d).map_err(KeyError::Io)?;
                let path = d.join(KEYS_FILE);
                let st = stamp(&path);
                g.keys = read_json::<KeysFile>(&path)
                    .map_err(KeyError::Io)?
                    .unwrap_or_default();
                g.stamp = st;
                Some(l)
            }
            None => None,
        };
        let r = f(&mut g.keys)?;
        self.save_keys(&mut g).map_err(KeyError::Io)?;
        Ok(r)
    }

    /// Pick up changes another process (the CLI) made to `keys.json`.
    fn refresh(&self, g: &mut Inner) {
        let Some(path) = self.keys_path() else { return };
        let st = stamp(&path);
        if st == g.stamp {
            return;
        }
        match read_json::<KeysFile>(&path) {
            Ok(k) => {
                g.keys = k.unwrap_or_default();
                g.stamp = st;
                tracing::info!(
                    keys = g.keys.keys.iter().filter(|k| k.active()).count(),
                    "keys file reloaded"
                );
            }
            // Atomic writes make a torn file unlikely; keep the last good
            // set and retry on the next request.
            Err(e) => tracing::warn!("keys file: {e}"),
        }
    }

    /// The active key whose secret this is, if any; touches its
    /// `last_connected` (not `last_used`). Compares every key's hash in constant time.
    pub fn authenticate(&self, secret: &str) -> Option<KeyRef> {
        let h = sha256_hex(secret);
        let mut g = self.lock();
        self.refresh(&mut g);
        let mut hit: Option<KeyRef> = None;
        for k in g.keys.keys.iter().filter(|k| k.active()) {
            if ct_eq(h.as_bytes(), k.hash.as_bytes()) && hit.is_none() {
                hit = Some(k.into());
            }
        }
        if let Some(k) = &hit {
            g.usage.entry(k.id.clone()).or_default().last_connected = Some(now_s());
            g.dirty = true;
        }
        hit
    }

    /// The client string a key's connection sent in `hello`.
    pub fn record_client(&self, id: &str, client: &str) {
        let mut g = self.lock();
        let u = g.usage.entry(id.to_string()).or_default();
        let client: String = client.chars().take(128).collect();
        if u.client.as_deref() != Some(client.as_str()) {
            u.client = Some(client);
            g.dirty = true;
        }
    }

    /// One finished dictation of `audio_ms` milliseconds.
    pub fn record_dictation(&self, id: &str, audio_ms: u64) {
        let mut g = self.lock();
        let u = g.usage.entry(id.to_string()).or_default();
        u.dictations += 1;
        u.audio_ms = u.audio_ms.saturating_add(audio_ms);
        u.last_used = Some(now_s());
        g.dirty = true;
    }

    /// One successful text-only cleanup (D36): a use, though it counts
    /// no dictation (the client transcribed on its own).
    pub fn record_clean(&self, id: &str) {
        let mut g = self.lock();
        g.usage.entry(id.to_string()).or_default().last_used = Some(now_s());
        g.dirty = true;
    }

    /// One model-library download request that sent `bytes` (D40).
    pub fn record_download(&self, id: &str, bytes: u64) {
        let mut g = self.lock();
        let u = g.usage.entry(id.to_string()).or_default();
        u.downloads += 1;
        u.download_bytes = u.download_bytes.saturating_add(bytes);
        u.last_used = Some(now_s());
        g.dirty = true;
    }

    /// Create a key. Returns it and its secret, which is never stored.
    pub fn add(&self, name: &str, admin: bool) -> Result<(KeyRef, String), KeyError> {
        check_name(name)?;
        self.write_locked(|keys| {
            if keys.keys.iter().any(|k| k.active() && k.name == name) {
                return Err(KeyError::NameTaken(format!(
                    "a key named {name:?} already exists"
                )));
            }
            let secret = crate::config::gen_token();
            let key = Key {
                id: new_id(keys),
                name: name.into(),
                admin,
                hash: sha256_hex(&secret),
                created: now_s(),
                revoked: None,
                origin: None,
            };
            let r = KeyRef::from(&key);
            keys.keys.push(key);
            Ok((r, secret))
        })
    }

    /// Revoke the active key with this name or id. Open connections keep
    /// running; the next connection with it gets 401. Refuses to revoke
    /// the last active admin key ([`KeyError::LastAdmin`]) unless `force`.
    pub fn revoke(&self, name_or_id: &str, force: bool) -> Result<KeyRef, KeyError> {
        // A name match wins over an id match (names of active keys are
        // unique; ids always are).
        let find = |keys: &[Key]| {
            let pos = |f: &dyn Fn(&Key) -> bool| keys.iter().position(|k| k.active() && f(k));
            pos(&|k| k.name == name_or_id).or_else(|| pos(&|k| k.id == name_or_id))
        };
        self.revoke_where(force, find, || {
            format!("no active key named or with id {name_or_id:?}")
        })
    }

    /// [`KeyStore::revoke`] by id only (the admin API).
    pub fn revoke_id(&self, id: &str, force: bool) -> Result<KeyRef, KeyError> {
        let find = |keys: &[Key]| keys.iter().position(|k| k.active() && k.id == id);
        self.revoke_where(force, find, || format!("no active key with id {id:?}"))
    }

    fn revoke_where(
        &self,
        force: bool,
        find: impl FnOnce(&[Key]) -> Option<usize>,
        not_found: impl FnOnce() -> String,
    ) -> Result<KeyRef, KeyError> {
        self.write_locked(|keys| {
            let i = find(&keys.keys).ok_or_else(|| KeyError::NotFound(not_found()))?;
            let k = &keys.keys[i];
            let admins = keys.keys.iter().filter(|k| k.active() && k.admin).count();
            if k.admin && admins <= 1 && !force {
                return Err(KeyError::LastAdmin(format!(
                    "{:?} is the last admin key: add another admin key first",
                    k.name
                )));
            }
            keys.keys[i].revoked = Some(now_s());
            Ok(KeyRef::from(&keys.keys[i]))
        })
    }

    /// Every key (revoked ones too), oldest first, with its usage.
    pub fn list(&self) -> Vec<KeyInfo> {
        let mut g = self.lock();
        self.refresh(&mut g);
        g.keys
            .keys
            .iter()
            .map(|k| KeyInfo {
                key: k.clone(),
                usage: g.usage.get(&k.id).cloned().unwrap_or_default(),
            })
            .collect()
    }

    /// Active keys.
    pub fn active(&self) -> usize {
        let mut g = self.lock();
        self.refresh(&mut g);
        g.keys.keys.iter().filter(|k| k.active()).count()
    }

    /// Write the usage counters if they changed (no-op in memory).
    pub fn flush(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let mut g = self.lock();
        if !g.dirty {
            return Ok(());
        }
        let f = UsageFile {
            version: FORMAT_VERSION,
            usage: g.usage.clone(),
        };
        write_json(&dir.join(USAGE_FILE), &f)?;
        g.dirty = false;
        Ok(())
    }

    fn save_keys(&self, g: &mut Inner) -> Result<(), String> {
        let Some(path) = self.keys_path() else {
            return Ok(());
        };
        g.keys.version = FORMAT_VERSION;
        write_json(&path, &g.keys)?;
        g.stamp = stamp(&path);
        Ok(())
    }
}

/// `flow-server keys list`: one row per key, revoked ones last-marked.
pub fn table(keys: &[KeyInfo]) -> String {
    let when = |t: Option<u64>| match t {
        Some(t) => {
            let s = crate::capture::ts(UNIX_EPOCH + std::time::Duration::from_secs(t));
            format!("{} {}", &s[..10], &s[11..16])
        }
        None => "-".into(),
    };
    let mut rows = vec![[
        "NAME",
        "ID",
        "ADMIN",
        "CREATED (UTC)",
        "LAST USED (UTC)",
        "LAST CONNECTED (UTC)",
        "CLIENT",
        "DICTATIONS",
        "AUDIO MIN",
        "DOWNLOADS",
        "STATUS",
    ]
    .map(String::from)];
    for k in keys {
        let u = &k.usage;
        rows.push([
            k.key.name.clone(),
            k.key.id.clone(),
            if k.key.admin { "yes" } else { "no" }.into(),
            when(Some(k.key.created)),
            when(u.last_used),
            when(u.last_connected),
            u.client.clone().unwrap_or_else(|| "-".into()),
            u.dictations.to_string(),
            format!("{:.1}", u.audio_ms as f64 / 60_000.0),
            match u.downloads {
                0 => "0".into(),
                n => format!("{n} ({:.1} MB)", u.download_bytes as f64 / 1e6),
            },
            match k.key.revoked {
                Some(t) => format!("revoked {}", when(Some(t))),
                None => "active".into(),
            },
        ]);
    }
    let mut w = [0usize; 11];
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let mut out = String::new();
    for r in &rows {
        let line: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<width$}", width = w[i]))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// Import `token` as the admin key `owner` (once), or move the imported
/// key to it after `gen-token` regenerated the file. Returns whether
/// anything changed.
fn import_token(keys: &mut KeysFile, token: &str) -> bool {
    let h = sha256_hex(token);
    if keys.token_file_imported {
        return match keys
            .keys
            .iter_mut()
            .find(|k| k.active() && k.origin.as_deref() == Some(ORIGIN_TOKEN_FILE))
        {
            Some(k) if k.hash != h => {
                k.hash = h;
                tracing::info!(key = %k.id, "token_file changed: owner key updated");
                true
            }
            _ => false,
        };
    }
    keys.token_file_imported = true;
    if keys.keys.iter().any(|k| k.active() && k.name == OWNER) {
        tracing::warn!("token_file not imported: a key named {OWNER:?} already exists");
        return true;
    }
    let key = Key {
        id: new_id(keys),
        name: OWNER.into(),
        admin: true,
        hash: h,
        created: now_s(),
        revoked: None,
        origin: Some(ORIGIN_TOKEN_FILE.into()),
    };
    tracing::info!(key = %key.id, "token_file imported as the admin key {OWNER:?}");
    keys.keys.push(key);
    true
}

fn check_name(name: &str) -> Result<(), KeyError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(KeyError::BadName(format!(
            "key name {name:?}: 1-64 characters of A-Z a-z 0-9 . _ -"
        )))
    }
}

fn new_id(keys: &KeysFile) -> String {
    loop {
        let mut b = [0u8; 8];
        getrandom::getrandom(&mut b).expect("OS randomness");
        let id = format!("k-{}", hex(&b));
        if !keys.keys.iter().any(|k| k.id == id) {
            return id;
        }
    }
}

pub fn sha256_hex(secret: &str) -> String {
    hex(&Sha256::digest(secret.as_bytes()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-time equality (length is not secret: hashes are fixed-size).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn create_dir(dir: &Path) -> Result<(), String> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
        .map_err(|e| format!("state dir {}: {e}", dir.display()))
}

/// Take the writers' exclusive lock on `keys.json` (blocks until free;
/// released when the returned file drops). The lock file is empty and
/// never removed: removing it would let two writers lock different files.
fn lock_file(dir: &Path) -> Result<std::fs::File, String> {
    let p = dir.join(LOCK_FILE);
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let f = o.open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    #[cfg(unix)]
    {
        // Same hand-over as write_json: a root CLI must not leave a lock
        // file the service user cannot open.
        use std::os::unix::fs::MetadataExt;
        if let (Ok(d), Ok(m)) = (std::fs::metadata(dir), f.metadata()) {
            if (d.uid(), d.gid()) != (m.uid(), m.gid()) {
                let _ = std::os::unix::fs::chown(&p, Some(d.uid()), Some(d.gid()));
            }
        }
    }
    f.lock().map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(f)
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<Option<T>, String> {
    match std::fs::read(p) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| format!("{}: {e}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

/// Write `v` to `p` atomically, mode 0600. When the CLI runs as root on a
/// state dir owned by the service user, the file is handed to that user so
/// the server can still read it.
pub(crate) fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<(), String> {
    use std::io::Write;
    let dir = p.parent().unwrap_or(Path::new("."));
    let mut rnd = [0u8; 4];
    getrandom::getrandom(&mut rnd).expect("OS randomness");
    let tmp = dir.join(format!(
        ".{}.{}-{}.tmp",
        p.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        hex(&rnd)
    ));
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let body = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    let r = (|| {
        let mut f = o.open(&tmp)?;
        f.write_all(&body)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let (d, m) = (std::fs::metadata(dir)?, f.metadata()?);
            if (d.uid(), d.gid()) != (m.uid(), m.gid()) {
                // Only root may do this; anyone else keeps their own file.
                let _ = std::os::unix::fs::chown(&tmp, Some(d.uid()), Some(d.gid()));
            }
        }
        std::fs::rename(&tmp, p)
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(err(e));
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "old-single-token-0123456789abcdef";

    fn mode(p: &Path) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777
        }
        #[cfg(not(unix))]
        {
            let _ = p;
            0o600
        }
    }

    #[test]
    fn token_migrates_to_admin_owner_once() {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let k = s.authenticate(TOKEN).expect("old token still works");
        assert_eq!((k.name.as_str(), k.admin), (OWNER, true));
        assert!(k.id.starts_with("k-") && k.id.len() == 18, "{}", k.id);
        // Reopening keeps the same key (stable id), no duplicate.
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.authenticate(TOKEN).unwrap().id, k.id);
        // A revoked owner is not re-imported on the next start.
        assert!(matches!(
            s.revoke(OWNER, false),
            Err(KeyError::LastAdmin(_))
        ));
        s.revoke(OWNER, true).unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        assert!(s.authenticate(TOKEN).is_none());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn regenerated_token_file_moves_the_owner_key() {
        let d = tempfile::tempdir().unwrap();
        let id = KeyStore::open(d.path(), Some(TOKEN))
            .unwrap()
            .authenticate(TOKEN)
            .unwrap()
            .id;
        let new = "regenerated-token-0123456789abcdef";
        let s = KeyStore::open(d.path(), Some(new)).unwrap();
        assert!(s.authenticate(TOKEN).is_none());
        assert_eq!(s.authenticate(new).unwrap().id, id);
    }

    #[test]
    fn only_hashes_on_disk_mode_0600() {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let (k, secret) = s.add("phone", false).unwrap();
        assert_eq!(secret.len(), 43);
        s.record_client(&k.id, "flow-android 9.9.9");
        s.flush().unwrap();
        for f in [KEYS_FILE, USAGE_FILE] {
            let p = d.path().join(f);
            let body = std::fs::read_to_string(&p).unwrap();
            assert!(
                !body.contains(&secret) && !body.contains(TOKEN),
                "{f}: {body}"
            );
            assert_eq!(mode(&p), 0o600, "{f}");
        }
        let body = std::fs::read_to_string(d.path().join(KEYS_FILE)).unwrap();
        assert!(body.contains(&sha256_hex(&secret)));
        assert!(body.contains(&sha256_hex(TOKEN)));
        // No temp files left behind (the writers' lock file stays).
        let mut names: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [USAGE_FILE, KEYS_FILE, LOCK_FILE]);
        assert_eq!(mode(&d.path().join(LOCK_FILE)), 0o600);
    }

    #[test]
    fn add_revoke_and_names() {
        let s = KeyStore::in_memory(TOKEN);
        let (k, secret) = s.add("laptop", true).unwrap();
        assert!(k.admin);
        assert_eq!(s.authenticate(&secret).unwrap(), k);
        assert!(s.add("laptop", false).is_err(), "names are unique");
        assert!(s.add("bad name", false).is_err());
        assert!(s.add("", false).is_err());
        assert!(s.authenticate("laptop").is_none());
        assert!(s.authenticate(&secret[..42]).is_none());
        // Revoke by id; the name is free again, the new key gets a new id.
        assert_eq!(s.revoke(&k.id, false).unwrap().id, k.id);
        assert!(s.authenticate(&secret).is_none());
        assert!(s.revoke(&k.id, false).is_err(), "already revoked");
        let (k2, _) = s.add("laptop", false).unwrap();
        assert_ne!(k2.id, k.id);
        assert_eq!(s.list().len(), 3);
        assert_eq!(s.active(), 2);
    }

    #[test]
    fn running_store_sees_cli_changes() {
        let d = tempfile::tempdir().unwrap();
        let server = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        assert!(server.authenticate(TOKEN).is_some());
        // The CLI is another process with its own store on the same dir.
        let cli = KeyStore::open(d.path(), None).unwrap();
        let (_, secret) = cli.add("tablet", false).unwrap();
        assert_eq!(server.authenticate(&secret).unwrap().name, "tablet");
        KeyStore::open(d.path(), None)
            .unwrap()
            .revoke("tablet", false)
            .unwrap();
        assert!(server.authenticate(&secret).is_none());
        assert!(server.authenticate(TOKEN).is_some());
    }

    #[test]
    fn usage_counts_and_survives_reopen() {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let id = s.authenticate(TOKEN).unwrap().id;
        s.record_client(&id, "flow-windows 1.0.0");
        s.record_dictation(&id, 1500);
        s.record_dictation(&id, 2500);
        s.record_download(&id, 1000);
        s.record_download(&id, 500);
        s.flush().unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let u = &s.list()[0].usage;
        assert_eq!((u.dictations, u.audio_ms), (2, 4000));
        assert_eq!((u.downloads, u.download_bytes), (2, 1500));
        assert_eq!(u.client.as_deref(), Some("flow-windows 1.0.0"));
        assert!(u.last_used.is_some());
        assert!(u.last_connected.is_some());
    }

    #[test]
    fn authenticate_connects_but_does_not_use() {
        let s = KeyStore::in_memory(TOKEN);
        let id = s.authenticate(TOKEN).unwrap().id;
        let u = s.list()[0].usage.clone();
        assert!(u.last_connected.is_some());
        assert_eq!(u.last_used, None, "an auth alone is not a use");
        s.record_dictation(&id, 1000);
        assert!(s.list()[0].usage.last_used.is_some());
    }

    #[test]
    fn old_usage_file_last_seen_becomes_last_connected() {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let id = s.list()[0].key.id.clone();
        drop(s);
        // key-usage.json as written before the split: only `last_seen`.
        std::fs::write(
            d.path().join(USAGE_FILE),
            format!(
                r#"{{"version":1,"usage":{{"{id}":{{"last_seen":1700000500,"client":"flow-android 1.0.0","dictations":3,"audio_ms":4000}}}}}}"#
            ),
        )
        .unwrap();
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let u = &s.list()[0].usage;
        assert_eq!(u.last_connected, Some(1_700_000_500));
        assert_eq!(u.last_used, None);
        assert_eq!((u.dictations, u.audio_ms), (3, 4000));
        // Rewritten in the new shape: no `last_seen` left behind.
        s.record_client(&id, "flow-android 1.1.0");
        s.flush().unwrap();
        let raw = std::fs::read_to_string(d.path().join(USAGE_FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["usage"][&id]["last_connected"], 1_700_000_500, "{raw}");
        assert!(!raw.contains("last_seen"), "{raw}");
    }

    #[test]
    fn table_shows_usage_never_hashes() {
        let s = KeyStore::in_memory(TOKEN);
        let id = s.authenticate(TOKEN).unwrap().id;
        s.record_client(&id, "flow-android 1.0.0");
        s.record_dictation(&id, 90_000);
        s.record_download(&id, 2_500_000);
        let (k, _) = s.add("old", false).unwrap();
        s.revoke(&k.id, false).unwrap();
        let t = table(&s.list());
        assert_eq!(t.lines().count(), 3, "{t}");
        let owner = t.lines().nth(1).unwrap();
        for want in [
            OWNER,
            id.as_str(),
            "yes",
            "flow-android 1.0.0",
            " 1 ",
            "1.5",
            "1 (2.5 MB)",
            "active",
        ] {
            assert!(owner.contains(want), "{want}: {t}");
        }
        assert!(t.lines().nth(2).unwrap().contains("revoked 20"), "{t}");
        assert!(!t.contains(&sha256_hex(TOKEN)), "{t}");
    }

    #[test]
    fn no_token_and_no_file_means_no_keys() {
        let d = tempfile::tempdir().unwrap();
        let s = KeyStore::open(d.path(), None).unwrap();
        assert_eq!(s.active(), 0);
        assert!(!d.path().join(KEYS_FILE).exists(), "nothing to write");
    }

    #[test]
    fn last_admin_guard() {
        let s = KeyStore::in_memory(TOKEN);
        let (phone, _) = s.add("phone", false).unwrap();
        assert!(matches!(
            s.revoke(OWNER, false),
            Err(KeyError::LastAdmin(_))
        ));
        let (laptop, _) = s.add("laptop", true).unwrap();
        // Another admin remains: the owner may go (even revoking itself).
        s.revoke(OWNER, false).unwrap();
        assert!(matches!(
            s.revoke_id(&laptop.id, false),
            Err(KeyError::LastAdmin(_))
        ));
        // Non-admin keys are never guarded.
        s.revoke_id(&phone.id, false).unwrap();
        assert!(matches!(
            s.revoke_id(&phone.id, false),
            Err(KeyError::NotFound(_))
        ));
        assert!(
            matches!(s.revoke_id("phone", false), Err(KeyError::NotFound(_))),
            "id only"
        );
        s.revoke_id(&laptop.id, true).unwrap();
        assert_eq!(s.active(), 0);
    }

    #[test]
    fn concurrent_writers_lose_nothing() {
        // Several stores on one dir (the server and CLI runs), each adding
        // keys from its own thread: every add must survive.
        let d = tempfile::tempdir().unwrap();
        KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let dir = d.path().to_path_buf();
                std::thread::spawn(move || {
                    let s = KeyStore::open(&dir, None).unwrap();
                    for i in 0..10 {
                        s.add(&format!("k{t}-{i}"), false).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let s = KeyStore::open(d.path(), Some(TOKEN)).unwrap();
        assert_eq!(s.active(), 41);
        // A store whose cached copy is stale still writes on top of the
        // file, not over it.
        let stale = KeyStore::open(d.path(), None).unwrap();
        stale.list();
        KeyStore::open(d.path(), None)
            .unwrap()
            .add("other", false)
            .unwrap();
        stale.add("mine", false).unwrap();
        let names: Vec<String> = s.list().into_iter().map(|k| k.key.name).collect();
        assert!(names.contains(&"other".into()) && names.contains(&"mine".into()));
    }
}
