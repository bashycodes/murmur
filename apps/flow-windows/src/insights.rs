//! Insights page store: `%APPDATA%\FlowClone\insights.json`.
//!
//! Per-local-day counts of committed dictations (words, recorded ms,
//! dictations, words per app; never text), kept apart from
//! `history.sqlite` so deleting History or a short retention window does
//! not reset the stats. The math is `flow_core::insights`; this module
//! only persists it and maps timestamps to the user's local date.
//! Nothing here leaves the machine.
//!
//! The desktop redesign's extra figures (where speech ran, fixes made, an
//! 18-week heatmap) are Windows-only [`Extras`] stored in the same file
//! next to the shared aggregate, so `flow_core::insights::Summary` (and
//! its golden vectors shared with Android) stays unchanged.

use crate::core_api::DictEntry;
use crate::history::History;
use flow_core::insights::{Aggregates, LocalDate, Summary, DEFAULT_TYPING_WPM};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Weeks in [`View::heatmap`].
pub const HEATMAP_WEEKS: usize = 18;
/// Days in [`View::routes_7d`] (today and the 6 before it).
const ROUTE_DAYS: i64 = 7;

/// Fixes counted on one day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fixes {
    /// Dictations whose text the AI cleanup changed.
    #[serde(default)]
    pub ai: u64,
    /// Dictionary replacements applied (see [`dictionary_fixes`]).
    #[serde(default)]
    pub dictionary: u64,
}

/// Windows-only per-day counts, keyed by local `YYYY-MM-DD` like
/// [`Aggregates::days`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extras {
    /// Day → route (`pc` / `server` / `cloud`) → dictations.
    #[serde(default)]
    pub routes: BTreeMap<String, BTreeMap<String, u64>>,
    /// Day → fixes.
    #[serde(default)]
    pub fixes: BTreeMap<String, Fixes>,
}

/// `insights.json`: the shared aggregate plus [`Extras`] at the top level
/// (a file from before the extras loads with them empty).
#[derive(Serialize, Deserialize)]
struct FileShape<A, E> {
    #[serde(flatten)]
    data: A,
    #[serde(flatten)]
    extra: E,
}

/// Dictations per route over the last 7 days. `unknown` = dictations with
/// no route recorded (before the route was counted, or a result without
/// router info).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RouteShare {
    pub pc: u64,
    pub server: u64,
    pub cloud: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FixTotals {
    pub ai: u64,
    pub dictionary: u64,
    pub total: u64,
}

/// One heatmap cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeatDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// `None` = after today (the rest of the current week).
    pub words: Option<u64>,
}

/// What `insights_get` returns: the shared [`Summary`] fields (flattened)
/// plus the redesign's extras.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct View {
    #[serde(flatten)]
    pub summary: Summary,
    /// [`HEATMAP_WEEKS`] × 7 days, oldest first, Monday-first weeks; the
    /// last week is the one holding today. Chunk by 7 for columns.
    pub heatmap: Vec<HeatDay>,
    pub routes_7d: RouteShare,
    /// All-time fixes (since they started being counted).
    pub fixes: FixTotals,
}

/// Monday of `d`'s week (1970-01-01 was a Thursday).
fn week_start(d: LocalDate) -> LocalDate {
    d.add_days(-(d.0 + 3).rem_euclid(7))
}

/// Dictionary replacements the output shows: for each non-snippet entry
/// with a replacement, how many more whole-word, case-insensitive
/// occurrences of the replacement `out` has than the raw ASR text `asr`.
/// Vocabulary-only words (no replacement) bias the recogniser and cannot
/// be told apart from a correct hearing, so they are not counted.
pub fn dictionary_fixes(asr: &str, out: &str, dict: &[DictEntry]) -> u64 {
    let (asr, out) = (asr.to_lowercase(), out.to_lowercase());
    dict.iter()
        .filter(|e| !e.is_snippet)
        .filter_map(|e| e.replacement.as_deref())
        .map(|r| r.trim().to_lowercase())
        .filter(|r| !r.is_empty())
        .map(|r| word_hits(&out, &r).saturating_sub(word_hits(&asr, &r)))
        .sum()
}

/// Occurrences of `needle` in `hay` not touching a letter or digit.
fn word_hits(hay: &str, needle: &str) -> u64 {
    let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    hay.match_indices(needle)
        .filter(|&(i, m)| {
            edge(hay[..i].chars().next_back()) && edge(hay[i + m.len()..].chars().next())
        })
        .count() as u64
}

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
    pub extra: Extras,
}

impl InsightsStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load (a missing or unreadable file = empty, not yet backfilled).
    pub fn open(path: &Path) -> Self {
        let f: FileShape<Aggregates, Extras> = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(FileShape {
                data: Aggregates::default(),
                extra: Extras::default(),
            });
        InsightsStore {
            path: Some(path.to_path_buf()),
            data: f.data,
            extra: f.extra,
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
        let json = serde_json::to_string(&FileShape {
            data: &self.data,
            extra: &self.extra,
        })
        .map_err(std::io::Error::other)?;
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

    /// The extras of one committed dictation: its route (`None` = unknown,
    /// not counted) and the fixes it made.
    pub fn record_extras(&mut self, date: LocalDate, route: Option<&str>, fixes: Fixes) {
        let key = date.key();
        if let Some(r) = route {
            *self
                .extra
                .routes
                .entry(key.clone())
                .or_default()
                .entry(r.to_string())
                .or_default() += 1;
        }
        if fixes != Fixes::default() {
            let f = self.extra.fixes.entry(key).or_default();
            f.ai += fixes.ai;
            f.dictionary += fixes.dictionary;
        }
    }

    pub fn reset(&mut self) {
        self.data.reset();
        self.extra = Extras::default();
    }

    pub fn summary(&self, today: LocalDate) -> Summary {
        self.data.summary(today, DEFAULT_TYPING_WPM)
    }

    /// The Insights page: [`Summary`] + heatmap, route share, fixes.
    pub fn view(&self, today: LocalDate) -> View {
        let words = |d: LocalDate| self.data.days.get(&d.key()).map_or(0, |s| s.words);
        let first = week_start(today).add_days(-7 * (HEATMAP_WEEKS as i64 - 1));
        let heatmap = (0..7 * HEATMAP_WEEKS as i64)
            .map(|i| {
                let d = first.add_days(i);
                HeatDay {
                    date: d.key(),
                    words: (d <= today).then(|| words(d)),
                }
            })
            .collect();
        let mut routes_7d = RouteShare::default();
        let mut dictations = 0;
        for back in 0..ROUTE_DAYS {
            let key = today.add_days(-back).key();
            dictations += self.data.days.get(&key).map_or(0, |s| s.count);
            for (r, n) in self.extra.routes.get(&key).into_iter().flatten() {
                match r.as_str() {
                    "pc" => routes_7d.pc += n,
                    "server" => routes_7d.server += n,
                    "cloud" => routes_7d.cloud += n,
                    _ => {}
                }
            }
        }
        let known = routes_7d.pc + routes_7d.server + routes_7d.cloud;
        routes_7d.unknown = dictations.saturating_sub(known);
        let (ai, dictionary) = self
            .extra
            .fixes
            .values()
            .fold((0, 0), |(a, d), f| (a + f.ai, d + f.dictionary));
        View {
            summary: self.summary(today),
            heatmap,
            routes_7d,
            fixes: FixTotals {
                ai,
                dictionary,
                total: ai + dictionary,
            },
        }
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
    fn view_heatmap_is_18_monday_first_weeks_ending_this_week() {
        let today = LocalDate::parse("2026-10-30").unwrap(); // a Friday
        let mut s = InsightsStore::in_memory();
        s.record(today, 9, 1000, "a.exe");
        s.record(today.add_days(-125), 4, 1000, "a.exe");
        let v = s.view(today);
        assert_eq!(v.heatmap.len(), 18 * 7);
        assert_eq!(v.heatmap[0].date, "2026-06-29", "a Monday");
        let last = &v.heatmap[18 * 7 - 1];
        assert_eq!(last.date, "2026-11-01");
        assert_eq!(last.words, None, "after today");
        let t = v.heatmap.iter().find(|d| d.date == "2026-10-30").unwrap();
        assert_eq!(t.words, Some(9));
        assert_eq!(v.heatmap[0].words, Some(0));
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(j["total_words"], 13, "summary fields flattened");
        assert!(j["heatmap"].is_array() && j["routes_7d"].is_object());
    }

    #[test]
    fn view_route_share_counts_the_last_7_days() {
        let today = LocalDate::parse("2026-10-30").unwrap();
        let mut s = InsightsStore::in_memory();
        for (back, route) in [
            (0, Some("pc")),
            (1, Some("server")),
            (2, Some("server")),
            (3, None),
            (7, Some("cloud")),
        ] {
            let d = today.add_days(-back);
            s.record(d, 2, 1000, "a.exe");
            s.record_extras(d, route, Fixes::default());
        }
        let r = s.view(today).routes_7d;
        assert_eq!(
            r,
            RouteShare {
                pc: 1,
                server: 2,
                cloud: 0,
                unknown: 1
            }
        );
    }

    #[test]
    fn fixes_add_up_persist_and_reset() {
        let d = std::env::temp_dir().join(format!("flow-insights-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE_NAME);
        // A file from before the extras.
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            &p,
            r#"{"backfilled":true,"days":{"2026-10-30":{"words":3,"ms":1,"count":1}}}"#,
        )
        .unwrap();
        let mut s = InsightsStore::open(&p);
        assert_eq!(s.extra, Extras::default());
        assert_eq!(s.data.days.len(), 1, "shared aggregate still read");
        let day = utc(0);
        s.record_extras(
            day,
            Some("pc"),
            Fixes {
                ai: 1,
                dictionary: 2,
            },
        );
        s.record_extras(
            day,
            None,
            Fixes {
                ai: 1,
                dictionary: 0,
            },
        );
        s.save().unwrap();
        let back = InsightsStore::open(&p);
        assert_eq!(back.extra, s.extra);
        assert_eq!(back.data, s.data);
        assert_eq!(
            back.view(day).fixes,
            FixTotals {
                ai: 2,
                dictionary: 2,
                total: 4
            }
        );
        s.reset();
        assert_eq!(s.view(day).fixes.total, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dictionary_fixes_count_replacements_the_asr_did_not_have() {
        let dict = [
            DictEntry {
                word: "gh".into(),
                replacement: Some("GitHub".into()),
                is_snippet: false,
            },
            DictEntry {
                word: "Kubernetes".into(),
                replacement: None,
                is_snippet: false,
            },
            DictEntry {
                word: "addr".into(),
                replacement: Some("1 Main St".into()),
                is_snippet: true,
            },
        ];
        assert_eq!(
            dictionary_fixes("push to gh and gh", "Push to GitHub and GitHub.", &dict),
            2
        );
        assert_eq!(
            dictionary_fixes("github is up", "GitHub is up.", &dict),
            0,
            "heard right"
        );
        assert_eq!(
            dictionary_fixes("x", "GitHubby", &dict),
            0,
            "whole words only"
        );
        assert_eq!(
            dictionary_fixes("addr", "1 Main St", &dict),
            0,
            "snippets are not fixes"
        );
    }

    #[test]
    fn default_path_is_in_the_app_dir() {
        let p = default_path(Some("X"));
        assert!(p.ends_with(Path::new(crate::prefs::APP_DIR).join(FILE_NAME)));
    }
}
