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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
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
    /// The user's later correction (learn-from-edits; not captured yet).
    pub edited: Option<String>,
    /// Dictation length in ms of audio.
    pub duration_ms: i64,
    pub words: i64,
    pub lang: Option<String>,
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
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
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
        match cutoff(r, now) {
            None => Ok(0),
            Some(c) => self.conn.execute("DELETE FROM history WHERE ts < ?1", [c]),
        }
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
                "SELECT ts, app, url, raw, formatted, pasted, edited, duration, words, lang
                 FROM history WHERE ts >= ?1 ORDER BY ts DESC, id DESC LIMIT 1",
                [min_ts],
                |row| {
                    Ok(Entry {
                        ts: row.get(0)?,
                        app: row.get(1)?,
                        url: row.get(2)?,
                        raw: row.get(3)?,
                        formatted: row.get(4)?,
                        pasted: row.get(5)?,
                        edited: row.get(6)?,
                        duration_ms: row.get(7)?,
                        words: row.get(8)?,
                        lang: row.get(9)?,
                    })
                },
            )
            .optional()
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
