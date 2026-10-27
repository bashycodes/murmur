//! Insights page store: `%APPDATA%\FlowClone\insights.json`.
//!
//! Per-local-day counts of committed dictations (words, recorded ms,
//! dictations, words per app; never text), kept apart from
//! `history.sqlite` so deleting History or a short retention window does
//! not reset the stats. The math is `flow_core::insights`; this module
//! only persists it and maps timestamps to the user's local date.
//! Nothing here leaves the machine.

use crate::history::History;
use flow_core::insights::{Aggregates, LocalDate, Summary, DEFAULT_TYPING_WPM};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "insights.json";

pub fn default_path(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join(FILE_NAME)
}

/// Local calendar date of Unix time `ts_ms` (Windows time-zone rules,
/// DST included, for that instant). Off Windows: UTC.
pub fn local_date(ts_ms: i64) -> LocalDate {
    LocalDate::from_unix_ms(ts_ms, utc_offset_min(ts_ms))
}

#[cfg(not(windows))]
fn utc_offset_min(_ts_ms: i64) -> i32 {
    0
}

#[cfg(windows)]
fn utc_offset_min(ts_ms: i64) -> i32 {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME: 100 ns ticks since 1601-01-01.
    const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;
    let ticks = ((ts_ms + EPOCH_DIFF_MS).max(0) as u64) * 10_000;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: plain out-parameters on the stack.
    let ok = unsafe {
        FileTimeToSystemTime(&ft, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
    };
    if !ok {
        return 0;
    }
    let mins = |s: &SYSTEMTIME| {
        LocalDate::from_ymd(s.wYear as i64, s.wMonth as u32, s.wDay as u32).0 * 1440
            + s.wHour as i64 * 60
            + s.wMinute as i64
    };
    (mins(&local) - mins(&utc)) as i32
}

#[derive(Default)]
pub struct InsightsStore {
    path: Option<PathBuf>,
    pub data: Aggregates,
}

impl InsightsStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load (a missing or unreadable file = empty, not yet backfilled).
    pub fn open(path: &Path) -> Self {
        let data = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        InsightsStore {
            path: Some(path.to_path_buf()),
            data,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string(&self.data).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }

    /// Count one committed dictation (`date` = its local day).
    pub fn record(&mut self, date: LocalDate, words: i64, duration_ms: i64, app: &str) {
        self.data
            .add(date, words.max(0) as u64, duration_ms.max(0) as u64, app);
    }

    /// First run only: fold the transcripts already in History in, so the
    /// page doesn't start at zero. Returns how many rows were counted.
    pub fn backfill_once(
        &mut self,
        history: &History,
        date_of: &dyn Fn(i64) -> LocalDate,
    ) -> rusqlite::Result<usize> {
        if self.data.backfilled {
            return Ok(0);
        }
        let rows = history.stats_rows()?;
        for r in &rows {
            self.record(date_of(r.ts), r.words, r.duration_ms, &r.app);
        }
        self.data.backfilled = true;
        Ok(rows.len())
    }

    pub fn reset(&mut self) {
        self.data.reset();
    }

    pub fn summary(&self, today: LocalDate) -> Summary {
        self.data.summary(today, DEFAULT_TYPING_WPM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{word_count, Entry};
    use crate::prefs::Retention;

    const DAY_MS: i64 = 86_400_000;

    fn utc(ts: i64) -> LocalDate {
        LocalDate::from_unix_ms(ts, 0)
    }

    fn entry(ts: i64, text: &str, ms: i64) -> Entry {
        Entry {
            ts,
            app: "notepad.exe".into(),
            raw: text.into(),
            pasted: Some(text.into()),
            duration_ms: ms,
            words: word_count(text),
            ..Entry::default()
        }
    }

    #[test]
    fn backfill_runs_once_and_survives_history_clear() {
        let h = History::open_in_memory().unwrap();
        let t0 = 20_000 * DAY_MS;
        h.record(&entry(t0, "one two three", 3000), Retention::Keep, t0)
            .unwrap();
        h.record(&entry(t0 + DAY_MS, "four five", 2000), Retention::Keep, t0)
            .unwrap();
        let mut s = InsightsStore::in_memory();
        assert_eq!(s.backfill_once(&h, &utc).unwrap(), 2);
        assert_eq!(s.backfill_once(&h, &utc).unwrap(), 0, "only once");
        h.clear().unwrap();
        let sum = s.summary(utc(t0 + DAY_MS));
        assert_eq!(sum.total_words, 5, "stats outlive History");
        assert_eq!(sum.dictations, 2);
        assert_eq!(sum.current_streak, 2);
    }

    #[test]
    fn reset_clears_and_does_not_backfill_again() {
        let h = History::open_in_memory().unwrap();
        h.record(&entry(5, "a b", 1000), Retention::Keep, 5)
            .unwrap();
        let mut s = InsightsStore::in_memory();
        s.backfill_once(&h, &utc).unwrap();
        s.reset();
        assert_eq!(s.backfill_once(&h, &utc).unwrap(), 0);
        assert_eq!(s.summary(utc(5)).total_words, 0);
    }

    #[test]
    fn persists_across_open() {
        let d = std::env::temp_dir().join(format!("flow-insights-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE_NAME);
        let mut s = InsightsStore::open(&p);
        assert!(!s.data.backfilled, "missing file = first run");
        s.record(utc(0), 7, 4200, "code.exe");
        s.data.backfilled = true;
        s.save().unwrap();
        let back = InsightsStore::open(&p);
        assert_eq!(back.data, s.data);
        assert_eq!(back.summary(utc(0)).top_apps[0].app, "code.exe");
        // A corrupt file reads as empty instead of failing the app.
        std::fs::write(&p, "{nope").unwrap();
        assert_eq!(InsightsStore::open(&p).data, Aggregates::default());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn default_path_is_in_the_app_dir() {
        let p = default_path(Some("X"));
        assert!(p.ends_with(Path::new(crate::prefs::APP_DIR).join(FILE_NAME)));
    }
}
