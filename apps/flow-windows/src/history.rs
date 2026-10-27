//! Local transcript history: `%APPDATA%\FlowClone\history.sqlite`.
//!
//! Schema per windows-research 10-clone-spec §2 (Wispr's 28-table
//! `flow.sqlite` reduced to what the clone needs):
//! `history(id, ts, app, url, raw, formatted, pasted, edited, duration, words, lang)`.
//! **Audio is never stored** — there is no column for it.
//!
//! Retention pref (`keep` / `day` = 24 h / `never`) is enforced on every
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
}

impl From<Entry> for Item {
    fn from(entry: Entry) -> Item {
        let final_text = entry.best_text().to_string();
        let ai_edited = entry
            .formatted
            .as_deref()
            .is_some_and(|f| f.trim() != entry.raw.trim());
        Item {
            entry,
            final_text,
            ai_edited,
        }
    }
}

const COLUMNS: &str = "id, ts, app, url, raw, formatted, pasted, edited, duration, words, lang";

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
             CREATE INDEX IF NOT EXISTS history_ts ON history(ts);",
        )?;
        Ok(History { conn })
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
            self.scrub();
        }
        Ok(n)
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
            "INSERT INTO history (ts, app, url, raw, formatted, pasted, edited, duration, words, lang)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                e.lang
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
               OR edited LIKE ?2 ESCAPE '\\' OR app LIKE ?2 ESCAPE '\\')
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
        self.scrub();
        Ok(n > 0)
    }

    pub fn clear(&self) -> rusqlite::Result<usize> {
        let n = self.conn.execute("DELETE FROM history", [])?;
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
    fn never_stores_nothing_and_wipes_existing() {
        let h = History::open_in_memory().unwrap();
        h.record(&entry(1, "kept"), Retention::Keep, 1).unwrap();
        assert_eq!(h.record(&entry(2, "x"), Retention::Never, 2).unwrap(), None);
        assert_eq!(h.count().unwrap(), 0);
        assert!(h.latest(Retention::Never, 2).unwrap().is_none());
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
                "lang"
            ]
        );
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
