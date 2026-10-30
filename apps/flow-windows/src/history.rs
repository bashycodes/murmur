//! Local transcript history: `%APPDATA%\FlowClone\history.sqlite`.
//!
//! Schema per windows-research 10-clone-spec §2 (Wispr's 28-table
//! `flow.sqlite` reduced to what the clone needs):
//! `history(id, ts, app, url, raw, formatted, pasted, edited, duration, words, lang)`,
//! plus `kind` / `original` for Command Mode rows, `route` (where the
//! speech was turned into text) and `transform` (the transform applied),
//! all added in place to older databases.
//!
//! Retry (0.11.0, DECISIONS D35): the 16 kHz mono audio of the newest
//! dictations only (`retry_audio_keep`, default 20; never a password
//! field's, never a command's) sits in `history_audio`, so a row can be
//! re-transcribed with the settings active now. Each retry adds a row to
//! `history_versions` (version 0 = the original); the `history` row always
//! holds the newest version, so copy / paste again / paste last / stats
//! read it unchanged. Both go with their row (delete, retention, clear).
//!
//! Retention pref (`keep` / `days_30` / `days_7` / `day` = 24 h / `never`)
//! is enforced on every
//! write and on open, so switching to `never` wipes existing rows.

use crate::prefs::Retention;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "history.sqlite";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Entry {
    /// Row id (0 for an entry not yet stored).
    pub id: i64,
    /// Unix epoch milliseconds.
    pub ts: i64,
    /// Process name (`slack.exe`).
    pub app: String,
    pub url: Option<String>,
    /// ASR text before cleanup/post-processing.
    pub raw: String,
    /// LLM-formatted text (only when the core returned FORMATTED).
    pub formatted: Option<String>,
    /// What was inserted; `None` when the paste failed (text still kept so
    /// "paste last transcript" can recover it).
    pub pasted: Option<String>,
    /// The field text the user left after correcting the paste
    /// (learn-from-edits, `learn.rs`).
    pub edited: Option<String>,
    /// Dictation length in ms of audio.
    pub duration_ms: i64,
    pub words: i64,
    pub lang: Option<String>,
    /// A Command Mode row: `raw` = the spoken instruction, `formatted` /
    /// `pasted` = the AI result, `original` = the selection it replaced.
    pub command: bool,
    /// Command Mode: the selected text before the rewrite (`None` = the
    /// result was generated at the cursor).
    pub original: Option<String>,
    /// Where the speech was turned into text: `pc` / `server` / `cloud`
    /// ([`route_label`]); `None` = unknown (rows from before the column, or
    /// a result without route info).
    pub route: Option<String>,
    /// Name of the transform that produced the pasted text: a transform
    /// shortcut row (`command`, `original` = the selection) or a dictation
    /// passed through auto-apply (`original` = the dictation before it).
    pub transform: Option<String>,
}

/// History / Insights name of a router engine (`RouteInfo.engine`):
/// `local` → `pc`, `remote` → `server`, `cloud` → `cloud`.
pub fn route_label(engine: &str) -> Option<&'static str> {
    match engine {
        "local" => Some("pc"),
        "remote" => Some("server"),
        "cloud" => Some("cloud"),
        _ => None,
    }
}

/// A row's counts only (Insights backfill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsRow {
    pub ts: i64,
    pub app: String,
    pub duration_ms: i64,
    pub words: i64,
}

/// What the settings History tab shows for one row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Item {
    #[serde(flatten)]
    pub entry: Entry,
    /// What was (or would have been) inserted.
    pub final_text: String,
    /// The LLM changed the wording: "undo AI edit" has something to undo.
    pub ai_edited: bool,
    /// The recording is still kept: the row can be retried.
    pub has_audio: bool,
    /// Every version once the row was retried (0 = the original, the last
    /// = this row's text); empty = never retried.
    pub versions: Vec<Version>,
}

/// One transcription of a row's recording (`history_versions`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Version {
    /// 0 = the original dictation, then one per retry.
    pub n: i64,
    /// When this version was made (Unix ms).
    pub ts: i64,
    pub raw: String,
    pub formatted: Option<String>,
    pub pasted: Option<String>,
    pub words: i64,
    pub lang: Option<String>,
    pub route: Option<String>,
    pub transform: Option<String>,
    pub original: Option<String>,
    /// What the version shows (its `best_text`).
    pub text: String,
}

/// The outcome of a retry, written over the row ([`History::apply_retry`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Retried {
    pub raw: String,
    /// The AI cleanup's text (`None` = the transcript was used as is).
    pub formatted: Option<String>,
    /// The auto-apply transform that ran on it: (name, its result).
    pub transform: Option<(String, String)>,
    pub lang: Option<String>,
    pub route: Option<String>,
}

/// A kept recording (`history_audio`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    /// 16 kHz mono PCM.
    pub pcm: Vec<i16>,
    /// The built-in style bucket of the app at dictation time (from its
    /// window title too, which is not kept); `None` = unknown.
    pub bucket: Option<String>,
}

impl From<Entry> for Item {
    fn from(entry: Entry) -> Item {
        let final_text = entry.best_text().to_string();
        // A command's result is all AI: there is no transcript to undo to.
        let ai_edited = !entry.command
            && entry
                .formatted
                .as_deref()
                .is_some_and(|f| f.trim() != entry.raw.trim());
        Item {
            entry,
            final_text,
            ai_edited,
            has_audio: false,
            versions: Vec::new(),
        }
    }
}

/// 16-bit little-endian bytes of `pcm` (the `history_audio.pcm` blob).
fn pcm_bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn pcm_from_bytes(b: &[u8]) -> Vec<i16> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

const COLUMNS: &str =
    "id, ts, app, url, raw, formatted, pasted, edited, duration, words, lang, kind, original, route, transform";

/// `history.kind` of a Command Mode row (`''` = a dictation).
const KIND_COMMAND: &str = "command";

fn row_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        ts: row.get(1)?,
        app: row.get(2)?,
        url: row.get(3)?,
        raw: row.get(4)?,
        formatted: row.get(5)?,
        pasted: row.get(6)?,
        edited: row.get(7)?,
        duration_ms: row.get(8)?,
        words: row.get(9)?,
        lang: row.get(10)?,
        command: row.get::<_, String>(11)? == KIND_COMMAND,
        original: row.get(12)?,
        route: row.get(13)?,
        transform: row.get(14)?,
    })
}

/// `LIKE` pattern matching `q` literally anywhere.
fn like_pattern(q: &str) -> String {
    let mut p = String::from("%");
    for c in q.trim().chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

impl Entry {
    /// Text "paste last transcript" should insert.
    pub fn best_text(&self) -> &str {
        self.pasted
            .as_deref()
            .or(self.formatted.as_deref())
            .unwrap_or(&self.raw)
    }
}

pub struct History {
    conn: Connection,
}

pub fn default_path(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join(FILE_NAME)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Oldest timestamp that survives `r` at `now`; `None` = keep everything.
pub fn cutoff(r: Retention, now: i64) -> Option<i64> {
    match r {
        Retention::Keep => None,
        Retention::Days30 => Some(now - 30 * DAY_MS),
        Retention::Days7 => Some(now - 7 * DAY_MS),
        Retention::Day => Some(now - DAY_MS),
        Retention::Never => Some(i64::MAX),
    }
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<History> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<History> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<History> {
        // secure_delete: deleted rows are overwritten with zeros, not left
        // readable in free pages (review S17).
        conn.execute_batch(
            "PRAGMA secure_delete=ON;
             PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS history (
               id        INTEGER PRIMARY KEY AUTOINCREMENT,
               ts        INTEGER NOT NULL,
               app       TEXT NOT NULL DEFAULT '',
               url       TEXT,
               raw       TEXT NOT NULL DEFAULT '',
               formatted TEXT,
               pasted    TEXT,
               edited    TEXT,
               duration  INTEGER NOT NULL DEFAULT 0,
               words     INTEGER NOT NULL DEFAULT 0,
               lang      TEXT
             );
             CREATE INDEX IF NOT EXISTS history_ts ON history(ts);
             CREATE TABLE IF NOT EXISTS history_audio (
               history_id INTEGER PRIMARY KEY,
               rate       INTEGER NOT NULL DEFAULT 16000,
               pcm        BLOB NOT NULL,
               bucket     TEXT
             );
             CREATE TABLE IF NOT EXISTS history_versions (
               id         INTEGER PRIMARY KEY AUTOINCREMENT,
               history_id INTEGER NOT NULL,
               n          INTEGER NOT NULL,
               ts         INTEGER NOT NULL,
               raw        TEXT NOT NULL DEFAULT '',
               formatted  TEXT,
               pasted     TEXT,
               words      INTEGER NOT NULL DEFAULT 0,
               lang       TEXT,
               route      TEXT,
               transform  TEXT,
               original   TEXT,
               UNIQUE (history_id, n)
             );",
        )?;
        // Columns added after the first schema (Command Mode, route), added
        // to databases created before them.
        for (col, decl) in [
            ("kind", "TEXT NOT NULL DEFAULT ''"),
            ("original", "TEXT"),
            ("route", "TEXT"),
            ("transform", "TEXT"),
        ] {
            let has = conn
                .prepare("SELECT 1 FROM pragma_table_info('history') WHERE name = ?1")?
                .exists([col])?;
            if !has {
                conn.execute_batch(&format!("ALTER TABLE history ADD COLUMN {col} {decl};"))?;
            }
        }
        let h = History { conn };
        h.drop_orphans()?;
        Ok(h)
    }

    /// Delete rows older than the retention window.
    pub fn prune(&self, r: Retention, now: i64) -> rusqlite::Result<usize> {
        let n = match cutoff(r, now) {
            None => 0,
            Some(c) => self
                .conn
                .execute("DELETE FROM history WHERE ts < ?1", [c])?,
        };
        if n > 0 {
            self.drop_orphans()?;
            self.scrub();
        }
        Ok(n)
    }

    /// Recordings and versions whose row is gone go too.
    fn drop_orphans(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "DELETE FROM history_audio WHERE history_id NOT IN (SELECT id FROM history);
             DELETE FROM history_versions WHERE history_id NOT IN (SELECT id FROM history);",
        )
    }

    /// Keep the recordings of the newest `keep` rows only (0 = none).
    pub fn prune_audio(&self, keep: u32) -> rusqlite::Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM history_audio WHERE history_id NOT IN
               (SELECT id FROM history ORDER BY ts DESC, id DESC LIMIT ?1)",
            [i64::from(keep)],
        )?;
        if n > 0 {
            self.scrub();
        }
        Ok(n)
    }

    /// Keep row `id`'s recording (16 kHz mono), then prune to the newest
    /// `keep`. Nothing is stored when `keep` is 0 or the row is gone.
    pub fn record_audio(
        &self,
        id: i64,
        pcm: &[i16],
        bucket: Option<&str>,
        keep: u32,
    ) -> rusqlite::Result<bool> {
        if keep == 0 || pcm.is_empty() || self.get(id)?.is_none() {
            self.prune_audio(keep)?;
            return Ok(false);
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO history_audio (history_id, rate, pcm, bucket)
             VALUES (?1, 16000, ?2, ?3)",
            params![id, pcm_bytes(pcm), bucket],
        )?;
        self.prune_audio(keep)?;
        self.has_audio(id)
    }

    pub fn has_audio(&self, id: i64) -> rusqlite::Result<bool> {
        self.conn
            .prepare_cached("SELECT 1 FROM history_audio WHERE history_id = ?1")?
            .exists([id])
    }

    /// Row `id`'s kept recording.
    pub fn audio(&self, id: i64) -> rusqlite::Result<Option<Recording>> {
        self.conn
            .query_row(
                "SELECT pcm, bucket FROM history_audio WHERE history_id = ?1",
                [id],
                |r| {
                    Ok(Recording {
                        pcm: pcm_from_bytes(&r.get::<_, Vec<u8>>(0)?),
                        bucket: r.get(1)?,
                    })
                },
            )
            .optional()
    }

    /// Every version of row `id`, oldest (0) first; empty = never retried.
    pub fn versions(&self, id: i64) -> rusqlite::Result<Vec<Version>> {
        let mut st = self.conn.prepare_cached(
            "SELECT n, ts, raw, formatted, pasted, words, lang, route, transform, original
             FROM history_versions WHERE history_id = ?1 ORDER BY n",
        )?;
        let rows = st.query_map([id], |r| {
            let mut v = Version {
                n: r.get(0)?,
                ts: r.get(1)?,
                raw: r.get(2)?,
                formatted: r.get(3)?,
                pasted: r.get(4)?,
                words: r.get(5)?,
                lang: r.get(6)?,
                route: r.get(7)?,
                transform: r.get(8)?,
                original: r.get(9)?,
                text: String::new(),
            };
            v.text = v
                .pasted
                .clone()
                .or(v.formatted.clone())
                .unwrap_or(v.raw.clone());
            Ok(v)
        })?;
        rows.collect()
    }

    /// The settings History list's view of a row (retry state included).
    pub fn item(&self, e: Entry) -> rusqlite::Result<Item> {
        let id = e.id;
        let mut item = Item::from(e);
        item.has_audio = !item.entry.command && self.has_audio(id)?;
        item.versions = self.versions(id)?;
        Ok(item)
    }

    /// A retry of row `id` finished: the first one saves the original as
    /// version 0; the new text becomes the next version and the row's own
    /// text (the "pasted" text too when the original was pasted, so paste
    /// last / copy read the newest; nothing is pasted by a retry). The
    /// user's correction belonged to the old text and is cleared. Returns
    /// the new version's number and the row before the retry.
    pub fn apply_retry(
        &mut self,
        id: i64,
        r: &Retried,
        now: i64,
    ) -> rusqlite::Result<Option<(i64, Entry)>> {
        let Some(old) = self.get(id)? else {
            return Ok(None);
        };
        let final_text = r
            .transform
            .as_ref()
            .map(|(_, t)| t.clone())
            .or(r.formatted.clone())
            .unwrap_or(r.raw.clone());
        // A not-pasted original stays "not pasted", unless a transform ran
        // (its result is only reachable through `pasted`).
        let pasted = old.pasted.is_some() || r.transform.is_some();
        let new = Entry {
            raw: r.raw.clone(),
            formatted: r.formatted.clone(),
            pasted: pasted.then(|| final_text.clone()),
            edited: None,
            words: word_count(r.formatted.as_deref().unwrap_or(&r.raw)),
            lang: r.lang.clone(),
            route: r.route.clone(),
            transform: r.transform.as_ref().map(|(n, _)| n.clone()),
            original: r
                .transform
                .as_ref()
                .map(|_| r.formatted.clone().unwrap_or(r.raw.clone())),
            ..old.clone()
        };
        let tx = self.conn.transaction()?;
        let insert = |e: &Entry, n: i64, ts: i64| {
            tx.execute(
                "INSERT INTO history_versions
                   (history_id, n, ts, raw, formatted, pasted, words, lang, route, transform, original)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id, n, ts, e.raw, e.formatted, e.pasted, e.words, e.lang, e.route,
                    e.transform, e.original
                ],
            )
        };
        let last: Option<i64> = tx.query_row(
            "SELECT MAX(n) FROM history_versions WHERE history_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let n = match last {
            Some(n) => n + 1,
            None => {
                insert(&old, 0, old.ts)?;
                1
            }
        };
        insert(&new, n, now)?;
        tx.execute(
            "UPDATE history SET raw = ?2, formatted = ?3, pasted = ?4, edited = NULL,
               words = ?5, lang = ?6, route = ?7, transform = ?8, original = ?9
             WHERE id = ?1",
            params![
                id,
                new.raw,
                new.formatted,
                new.pasted,
                new.words,
                new.lang,
                new.route,
                new.transform,
                new.original
            ],
        )?;
        tx.commit()?;
        Ok(Some((n, old)))
    }

    /// After a delete: fold the WAL into the database and truncate it, so
    /// removed transcripts do not linger in `history.sqlite-wal` (S17).
    /// Best effort (a concurrent reader can keep frames for a moment).
    fn scrub(&self) {
        let _ = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }

    /// Store `e` if retention allows; returns the row id when stored.
    pub fn record(&self, e: &Entry, r: Retention, now: i64) -> rusqlite::Result<Option<i64>> {
        self.prune(r, now)?;
        if r == Retention::Never {
            return Ok(None);
        }
        self.conn.execute(
            "INSERT INTO history
               (ts, app, url, raw, formatted, pasted, edited, duration, words, lang, kind, original, route, transform)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                e.ts,
                e.app,
                e.url,
                e.raw,
                e.formatted,
                e.pasted,
                e.edited,
                e.duration_ms,
                e.words,
                e.lang,
                if e.command { KIND_COMMAND } else { "" },
                e.original,
                e.route,
                e.transform
            ],
        )?;
        Ok(Some(self.conn.last_insert_rowid()))
    }

    pub fn latest(&self, r: Retention, now: i64) -> rusqlite::Result<Option<Entry>> {
        let min_ts = cutoff(r, now).unwrap_or(i64::MIN);
        self.conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM history WHERE ts >= ?1 ORDER BY ts DESC, id DESC LIMIT 1"
                ),
                [min_ts],
                row_entry,
            )
            .optional()
    }

    /// The newest row a transform produced ("view changes").
    pub fn latest_transform(&self, r: Retention, now: i64) -> rusqlite::Result<Option<Entry>> {
        let min_ts = cutoff(r, now).unwrap_or(i64::MIN);
        self.conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM history WHERE ts >= ?1 AND transform IS NOT NULL
                     ORDER BY ts DESC, id DESC LIMIT 1"
                ),
                [min_ts],
                row_entry,
            )
            .optional()
    }

    /// Newest first; `query` matches any text column or the app name
    /// (case-insensitive for ASCII, literally: `%`/`_` are not wildcards).
    pub fn list(
        &self,
        query: &str,
        limit: usize,
        r: Retention,
        now: i64,
    ) -> rusqlite::Result<Vec<Entry>> {
        let min_ts = cutoff(r, now).unwrap_or(i64::MIN);
        let mut st = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM history
             WHERE ts >= ?1 AND (?2 = '%%' OR raw LIKE ?2 ESCAPE '\\'
               OR formatted LIKE ?2 ESCAPE '\\' OR pasted LIKE ?2 ESCAPE '\\'
               OR edited LIKE ?2 ESCAPE '\\' OR app LIKE ?2 ESCAPE '\\'
               OR original LIKE ?2 ESCAPE '\\')
             ORDER BY ts DESC, id DESC LIMIT ?3"
        ))?;
        let rows = st.query_map(
            params![min_ts, like_pattern(query), limit.min(10_000) as i64],
            row_entry,
        )?;
        rows.collect()
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM history WHERE id = ?1"),
                [id],
                row_entry,
            )
            .optional()
    }

    /// True when a row was removed.
    pub fn delete(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM history WHERE id = ?1", [id])?;
        self.drop_orphans()?;
        self.scrub();
        Ok(n > 0)
    }

    pub fn clear(&self) -> rusqlite::Result<usize> {
        let n = self.conn.execute("DELETE FROM history", [])?;
        self.drop_orphans()?;
        // A full clear also returns the pages to the OS.
        let _ = self.conn.execute_batch("VACUUM;");
        self.scrub();
        Ok(n)
    }

    /// Store the user's correction of row `id` (learn-from-edits).
    pub fn set_edited(&self, id: i64, edited: &str) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "UPDATE history SET edited = ?2 WHERE id = ?1",
            params![id, edited],
        )? > 0)
    }

    /// Every row's counts (no text): the Insights backfill.
    pub fn stats_rows(&self) -> rusqlite::Result<Vec<StatsRow>> {
        let mut st = self
            .conn
            .prepare("SELECT ts, app, duration, words FROM history ORDER BY ts")?;
        let rows = st.query_map([], |r| {
            Ok(StatsRow {
                ts: r.get(0)?,
                app: r.get(1)?,
                duration_ms: r.get(2)?,
                words: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    pub fn count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
    }
}

pub fn word_count(s: &str) -> i64 {
    s.split_whitespace().count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: i64, text: &str) -> Entry {
        Entry {
            ts,
            app: "notepad.exe".into(),
            raw: text.into(),
            pasted: Some(text.into()),
            words: word_count(text),
            ..Entry::default()
        }
    }

    /// Bytes of the database and its WAL on disk.
    fn disk_bytes(path: &std::path::Path) -> Vec<u8> {
        let mut all = std::fs::read(path).unwrap_or_default();
        let wal = path.with_extension("sqlite-wal");
        all.extend(std::fs::read(wal).unwrap_or_default());
        all
    }

    fn contains(hay: &[u8], needle: &str) -> bool {
        hay.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn deleted_transcripts_leave_no_trace_on_disk() {
        // review S17: secure_delete + WAL truncate.
        let dir = std::env::temp_dir().join(format!("flow-hist-scrub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.sqlite");
        let h = History::open(&path).unwrap();
        for (i, t) in ["SECRET-ONE-alpha", "SECRET-TWO-bravo", "keep-this-row"]
            .iter()
            .enumerate()
        {
            h.record(&entry(i as i64 + 1, t), Retention::Keep, 10)
                .unwrap();
        }
        let first = h.list("SECRET-ONE", 10, Retention::Keep, 10).unwrap()[0].id;
        assert!(h.delete(first).unwrap());
        assert!(!contains(&disk_bytes(&path), "SECRET-ONE-alpha"), "delete");
        // Retention "never" prunes everything.
        h.prune(Retention::Never, 10).unwrap();
        h.clear().unwrap();
        let bytes = disk_bytes(&path);
        for t in ["SECRET-TWO-bravo", "keep-this-row"] {
            assert!(!contains(&bytes, t), "{t} still on disk after clear");
        }
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_stores_and_returns_latest() {
        let h = History::open_in_memory().unwrap();
        h.record(&entry(1, "first"), Retention::Keep, 10).unwrap();
        h.record(&entry(2, "second one"), Retention::Keep, 10)
            .unwrap();
        let l = h.latest(Retention::Keep, 10).unwrap().unwrap();
        assert_eq!(l.best_text(), "second one");
        assert_eq!(l.words, 2);
        assert_eq!(h.count().unwrap(), 2);
    }

    #[test]
    fn day_retention_prunes_older_than_24h() {
        let h = History::open_in_memory().unwrap();
        let now = 10 * DAY_MS;
        h.record(&entry(now - DAY_MS - 1, "old"), Retention::Keep, now)
            .unwrap();
        h.record(&entry(now - 5, "new"), Retention::Day, now)
            .unwrap();
        assert_eq!(h.count().unwrap(), 1);
        assert_eq!(
            h.latest(Retention::Day, now).unwrap().unwrap().best_text(),
            "new"
        );
        assert!(h
            .latest(Retention::Day, now + 2 * DAY_MS)
            .unwrap()
            .is_none());
    }

    #[test]
    fn week_and_month_retention_prune_by_age() {
        let h = History::open_in_memory().unwrap();
        let now = 100 * DAY_MS;
        for age in [40, 20, 3] {
            h.record(&entry(now - age * DAY_MS, "x"), Retention::Keep, now)
                .unwrap();
        }
        h.prune(Retention::Days30, now).unwrap();
        assert_eq!(h.count().unwrap(), 2);
        assert_eq!(h.list("", 10, Retention::Days7, now).unwrap().len(), 1);
        h.prune(Retention::Days7, now).unwrap();
        assert_eq!(h.count().unwrap(), 1);
        for (r, j) in [
            (Retention::Keep, "keep"),
            (Retention::Days30, "days_30"),
            (Retention::Days7, "days_7"),
            (Retention::Day, "day"),
            (Retention::Never, "never"),
        ] {
            assert_eq!(serde_json::to_value(r).unwrap(), j);
        }
    }

    #[test]
    fn never_stores_nothing_and_wipes_existing() {
        let h = History::open_in_memory().unwrap();
        h.record(&entry(1, "kept"), Retention::Keep, 1).unwrap();
        assert_eq!(h.record(&entry(2, "x"), Retention::Never, 2).unwrap(), None);
        assert_eq!(h.count().unwrap(), 0);
        assert!(h.latest(Retention::Never, 2).unwrap().is_none());
    }

    #[test]
    fn command_rows_keep_instruction_original_and_result() {
        let h = History::open_in_memory().unwrap();
        let e = Entry {
            ts: 5,
            raw: "make it formal".into(),
            formatted: Some("I am unable to attend.".into()),
            pasted: Some("I am unable to attend.".into()),
            command: true,
            original: Some("cant come".into()),
            ..Entry::default()
        };
        let id = h.record(&e, Retention::Keep, 10).unwrap().unwrap();
        let got = h.get(id).unwrap().unwrap();
        assert_eq!(got, Entry { id, ..e });
        assert_eq!(got.best_text(), "I am unable to attend.");
        let item = Item::from(got);
        assert!(!item.ai_edited, "nothing to undo to");
        assert_eq!(h.list("cant", 10, Retention::Keep, 10).unwrap().len(), 1);
        let plain = h.record(&entry(6, "hello"), Retention::Keep, 10).unwrap();
        let plain = h.get(plain.unwrap()).unwrap().unwrap();
        assert!(!plain.command && plain.original.is_none());
    }

    #[test]
    fn a_database_from_before_command_mode_gains_its_columns() {
        let dir = std::env::temp_dir().join(format!("flow-hist-mig-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.sqlite");
        let _ = std::fs::remove_file(&path);
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE history (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                   app TEXT NOT NULL DEFAULT '', url TEXT, raw TEXT NOT NULL DEFAULT '',
                   formatted TEXT, pasted TEXT, edited TEXT,
                   duration INTEGER NOT NULL DEFAULT 0, words INTEGER NOT NULL DEFAULT 0,
                   lang TEXT);
                 INSERT INTO history (ts, raw, pasted) VALUES (1, 'old words', 'Old words.');",
            )
            .unwrap();
        }
        let h = History::open(&path).unwrap();
        let old = h.latest(Retention::Keep, 2).unwrap().unwrap();
        assert_eq!(old.best_text(), "Old words.");
        assert!(!old.command && old.original.is_none());
        assert_eq!(old.route, None, "old rows: route unknown");
        assert_eq!(old.transform, None);
        drop(h);
        // Opening again (columns present) is a no-op.
        assert!(History::open(&path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transform_rows_are_stored_and_the_latest_is_found() {
        let h = History::open_in_memory().unwrap();
        assert_eq!(h.latest_transform(Retention::Keep, 10).unwrap(), None);
        let e = Entry {
            ts: 2,
            raw: "polish instruction".into(),
            pasted: Some("Polished.".into()),
            command: true,
            original: Some("polishd".into()),
            transform: Some("Polish".into()),
            ..Entry::default()
        };
        let id = h.record(&e, Retention::Keep, 10).unwrap().unwrap();
        h.record(&entry(3, "plain dictation later"), Retention::Keep, 10)
            .unwrap();
        let got = h.latest_transform(Retention::Keep, 10).unwrap().unwrap();
        assert_eq!(got, Entry { id, ..e });
        let j = serde_json::to_value(Item::from(got)).unwrap();
        assert_eq!(j["transform"], "Polish");
        assert_eq!(
            h.latest(Retention::Keep, 10).unwrap().unwrap().transform,
            None
        );
    }

    #[test]
    fn route_is_stored_and_listed() {
        let h = History::open_in_memory().unwrap();
        let e = Entry {
            route: route_label("remote").map(String::from),
            ..entry(1, "via server")
        };
        let id = h.record(&e, Retention::Keep, 1).unwrap().unwrap();
        assert_eq!(h.get(id).unwrap().unwrap().route.as_deref(), Some("server"));
        let j = serde_json::to_value(Item::from(h.get(id).unwrap().unwrap())).unwrap();
        assert_eq!(j["route"], "server");
        assert_eq!(route_label("local"), Some("pc"));
        assert_eq!(route_label("cloud"), Some("cloud"));
        assert_eq!(route_label("mock"), None);
    }

    #[test]
    fn failed_paste_keeps_text_for_paste_last() {
        let e = Entry {
            raw: "raw words".into(),
            formatted: Some("Formatted words.".into()),
            pasted: None,
            ..Entry::default()
        };
        assert_eq!(e.best_text(), "Formatted words.");
    }

    #[test]
    fn list_searches_newest_first_and_escapes_wildcards() {
        let h = History::open_in_memory().unwrap();
        h.record(&entry(1, "alpha one"), Retention::Keep, 10)
            .unwrap();
        h.record(&entry(2, "beta 100% sure"), Retention::Keep, 10)
            .unwrap();
        h.record(&entry(3, "Alpha two"), Retention::Keep, 10)
            .unwrap();
        let all = h.list("", 10, Retention::Keep, 10).unwrap();
        assert_eq!(
            all.iter().map(|e| e.raw.as_str()).collect::<Vec<_>>(),
            ["Alpha two", "beta 100% sure", "alpha one"]
        );
        assert!(all.iter().all(|e| e.id > 0));
        let a = h.list("ALPHA", 10, Retention::Keep, 10).unwrap();
        assert_eq!(a.len(), 2, "case-insensitive");
        assert_eq!(h.list("%", 10, Retention::Keep, 10).unwrap().len(), 1);
        assert_eq!(h.list("_", 10, Retention::Keep, 10).unwrap().len(), 0);
        assert_eq!(h.list("notepad", 10, Retention::Keep, 10).unwrap().len(), 3);
        assert_eq!(h.list("", 1, Retention::Keep, 10).unwrap().len(), 1);
    }

    #[test]
    fn get_delete_clear_and_set_edited() {
        let h = History::open_in_memory().unwrap();
        let id = h
            .record(&entry(1, "teh plan"), Retention::Keep, 1)
            .unwrap()
            .unwrap();
        let other = h
            .record(&entry(2, "other"), Retention::Keep, 2)
            .unwrap()
            .unwrap();
        assert!(h.set_edited(id, "the plan").unwrap());
        assert_eq!(
            h.get(id).unwrap().unwrap().edited.as_deref(),
            Some("the plan")
        );
        assert!(h.list("the plan", 5, Retention::Keep, 2).unwrap().len() == 1);
        assert!(h.delete(id).unwrap());
        assert!(!h.delete(id).unwrap());
        assert!(h.get(id).unwrap().is_none());
        assert!(h.get(other).unwrap().is_some());
        assert_eq!(h.clear().unwrap(), 1);
        assert_eq!(h.count().unwrap(), 0);
    }

    #[test]
    fn item_flags_ai_edits_for_undo() {
        let mut e = entry(1, "um send it");
        e.formatted = Some("Send it.".into());
        e.pasted = Some("Send it.".into());
        let i = Item::from(e.clone());
        assert!(i.ai_edited);
        assert_eq!(i.final_text, "Send it.");
        e.formatted = None;
        assert!(!Item::from(e).ai_edited, "RAW_TRANSCRIPT has no AI edit");
        let j = serde_json::to_value(Item::from(entry(1, "x"))).unwrap();
        assert_eq!(j["raw"], "x", "entry fields flattened for the UI");
        assert_eq!(j["final_text"], "x");
    }

    #[test]
    fn schema_has_no_audio_column() {
        let h = History::open_in_memory().unwrap();
        let mut st = h.conn.prepare("PRAGMA table_info(history)").unwrap();
        let cols: Vec<String> = st
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            cols,
            [
                "id",
                "ts",
                "app",
                "url",
                "raw",
                "formatted",
                "pasted",
                "edited",
                "duration",
                "words",
                "lang",
                "kind",
                "original",
                "route",
                "transform"
            ]
        );
    }

    fn pcm(n: usize) -> Vec<i16> {
        (0..n).map(|i| (i as i16).wrapping_mul(37)).collect()
    }

    #[test]
    fn audio_is_kept_for_the_newest_rows_only() {
        let h = History::open_in_memory().unwrap();
        let ids: Vec<i64> = (1..=4)
            .map(|t| {
                let id = h
                    .record(&entry(t, "x"), Retention::Keep, 10)
                    .unwrap()
                    .unwrap();
                assert!(h.record_audio(id, &pcm(320), Some("work"), 2).unwrap());
                id
            })
            .collect();
        let kept: Vec<bool> = ids.iter().map(|&i| h.has_audio(i).unwrap()).collect();
        assert_eq!(kept, [false, false, true, true], "newest 2");
        let rec = h.audio(ids[3]).unwrap().unwrap();
        assert_eq!(rec.pcm, pcm(320), "PCM round-trips");
        assert_eq!(rec.bucket.as_deref(), Some("work"));
        // Lowering the setting prunes; 0 keeps none and stores none.
        h.prune_audio(1).unwrap();
        assert!(!h.has_audio(ids[2]).unwrap() && h.has_audio(ids[3]).unwrap());
        let id = h
            .record(&entry(9, "y"), Retention::Keep, 10)
            .unwrap()
            .unwrap();
        assert!(!h.record_audio(id, &pcm(10), None, 0).unwrap());
        assert_eq!(h.audio(ids[3]).unwrap(), None, "0 = keep none");
        assert!(!h.record_audio(999, &pcm(10), None, 5).unwrap(), "no row");
    }

    #[test]
    fn audio_and_versions_go_with_their_row() {
        let mut h = History::open_in_memory().unwrap();
        let now = 100 * DAY_MS;
        let old = h
            .record(&entry(now - 10 * DAY_MS, "old"), Retention::Keep, now)
            .unwrap()
            .unwrap();
        let a = h
            .record(&entry(now - 2, "a"), Retention::Keep, now)
            .unwrap()
            .unwrap();
        let b = h
            .record(&entry(now - 1, "b"), Retention::Keep, now)
            .unwrap()
            .unwrap();
        for id in [old, a, b] {
            h.record_audio(id, &pcm(16), None, 20).unwrap();
            h.apply_retry(
                id,
                &Retried {
                    raw: "again".into(),
                    ..Retried::default()
                },
                now,
            )
            .unwrap();
        }
        let count = |h: &History, t: &str| -> i64 {
            h.conn
                .query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(
            (count(&h, "history_audio"), count(&h, "history_versions")),
            (3, 6)
        );
        h.prune(Retention::Days7, now).unwrap();
        assert!(
            !h.has_audio(old).unwrap() && h.versions(old).unwrap().is_empty(),
            "expired"
        );
        assert!(h.delete(a).unwrap());
        assert_eq!(h.audio(a).unwrap(), None, "deleted");
        assert!(h.versions(a).unwrap().is_empty());
        assert_eq!(
            (count(&h, "history_audio"), count(&h, "history_versions")),
            (1, 2)
        );
        h.clear().unwrap();
        assert_eq!(
            (count(&h, "history_audio"), count(&h, "history_versions")),
            (0, 0)
        );
        assert!(
            h.apply_retry(b, &Retried::default(), now)
                .unwrap()
                .is_none(),
            "gone"
        );
    }

    #[test]
    fn deleted_audio_leaves_no_trace_on_disk() {
        let dir = std::env::temp_dir().join(format!("flow-hist-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.sqlite");
        let h = History::open(&path).unwrap();
        let id = h
            .record(&entry(1, "x"), Retention::Keep, 10)
            .unwrap()
            .unwrap();
        // A recognisable sample pattern ("AUDIOMARK" as PCM bytes).
        let mark: Vec<i16> = b"AUDIOMARKAUDIOMARK"
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        h.record_audio(id, &mark, None, 20).unwrap();
        assert!(h.delete(id).unwrap());
        assert!(!contains(&disk_bytes(&path), "AUDIOMARKAUDIOMARK"));
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retries_add_versions_and_the_row_shows_the_newest() {
        let mut h = History::open_in_memory().unwrap();
        let e = Entry {
            raw: "um send it".into(),
            formatted: Some("Send it.".into()),
            pasted: Some("Send it.".into()),
            edited: Some("Send it now.".into()),
            route: Some("pc".into()),
            lang: Some("en".into()),
            duration_ms: 1200,
            ..entry(5, "")
        };
        let id = h.record(&e, Retention::Keep, 10).unwrap().unwrap();
        assert!(h.versions(id).unwrap().is_empty(), "never retried");
        let r1 = Retried {
            raw: "um send it".into(),
            formatted: Some("Send it, please.".into()),
            route: Some("server".into()),
            lang: Some("en".into()),
            ..Retried::default()
        };
        let (n, before) = h.apply_retry(id, &r1, 20).unwrap().unwrap();
        assert_eq!((n, before.words), (1, 0));
        let row = h.get(id).unwrap().unwrap();
        assert_eq!(row.best_text(), "Send it, please.");
        assert_eq!(row.edited, None, "the correction was of the old text");
        assert_eq!(
            (row.route.as_deref(), row.words, row.ts),
            (Some("server"), 3, 5)
        );
        assert_eq!(row.duration_ms, 1200, "same recording");
        // A transform that is on now runs too.
        let r2 = Retried {
            raw: "send it".into(),
            formatted: None,
            transform: Some(("Polish".into(), "Please send it.".into())),
            ..Retried::default()
        };
        assert_eq!(h.apply_retry(id, &r2, 30).unwrap().unwrap().0, 2);
        let row = h.get(id).unwrap().unwrap();
        assert_eq!(row.best_text(), "Please send it.");
        assert_eq!(row.original.as_deref(), Some("send it"));
        let v = h.versions(id).unwrap();
        assert_eq!(
            v.iter().map(|v| (v.n, v.text.as_str())).collect::<Vec<_>>(),
            [
                (0, "Send it."),
                (1, "Send it, please."),
                (2, "Please send it.")
            ]
        );
        assert_eq!((v[0].ts, v[1].ts), (5, 20));
        assert_eq!(v[0].route.as_deref(), Some("pc"));
        let item = h.item(row).unwrap();
        let j = serde_json::to_value(&item).unwrap();
        assert_eq!(j["final_text"], "Please send it.");
        assert_eq!(j["versions"].as_array().unwrap().len(), 3);
        assert_eq!(j["has_audio"], false);
        // A not-pasted original stays not pasted.
        let np = h
            .record(
                &Entry {
                    pasted: None,
                    ..entry(6, "hi")
                },
                Retention::Keep,
                10,
            )
            .unwrap()
            .unwrap();
        let r3 = Retried {
            raw: "hi there".into(),
            ..Retried::default()
        };
        h.apply_retry(np, &r3, 40).unwrap();
        let row = h.get(np).unwrap().unwrap();
        assert_eq!((row.pasted.as_deref(), row.best_text()), (None, "hi there"));
    }

    #[test]
    fn a_database_from_before_retry_gains_its_tables() {
        let dir = std::env::temp_dir().join(format!("flow-hist-mig-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.sqlite");
        {
            // The 0.10 schema: history only.
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE history (
                   id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL,
                   app TEXT NOT NULL DEFAULT '', url TEXT, raw TEXT NOT NULL DEFAULT '',
                   formatted TEXT, pasted TEXT, edited TEXT,
                   duration INTEGER NOT NULL DEFAULT 0, words INTEGER NOT NULL DEFAULT 0,
                   lang TEXT, kind TEXT NOT NULL DEFAULT '', original TEXT, route TEXT,
                   transform TEXT);
                 INSERT INTO history (ts, raw, pasted) VALUES (1, 'old words', 'Old words.');",
            )
            .unwrap();
        }
        let mut h = History::open(&path).unwrap();
        let old = h.latest(Retention::Keep, 2).unwrap().unwrap();
        let item = h.item(old.clone()).unwrap();
        assert!(
            !item.has_audio && item.versions.is_empty(),
            "old rows: no audio"
        );
        h.apply_retry(
            old.id,
            &Retried {
                raw: "old words".into(),
                ..Retried::default()
            },
            3,
        )
        .unwrap();
        drop(h);
        let h = History::open(&path).unwrap();
        assert_eq!(h.versions(old.id).unwrap().len(), 2, "persisted");
        drop(h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_rows_offer_no_retry() {
        let h = History::open_in_memory().unwrap();
        let id = h
            .record(
                &Entry {
                    command: true,
                    ..entry(1, "x")
                },
                Retention::Keep,
                1,
            )
            .unwrap()
            .unwrap();
        h.record_audio(id, &pcm(8), None, 20).unwrap();
        let e = h.get(id).unwrap().unwrap();
        assert!(!h.item(e).unwrap().has_audio);
    }

    #[test]
    fn file_db_persists_across_open() {
        let d = std::env::temp_dir().join(format!("flow-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE_NAME);
        History::open(&p)
            .unwrap()
            .record(&entry(5, "persisted"), Retention::Keep, 5)
            .unwrap();
        let h = History::open(&p).unwrap();
        assert_eq!(
            h.latest(Retention::Keep, 5).unwrap().unwrap().best_text(),
            "persisted"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
