//! Usage insights: a small per-day aggregate of committed dictations and
//! the stats shown on the Insights page (words, speaking WPM, streaks,
//! estimated time saved vs typing, top apps).
//!
//! Pure and platform-free so every shell (Windows today, Android via FFI
//! later) shares the same math. The shell owns persistence and decides
//! what "today" is: days are **local** calendar dates ([`LocalDate`]),
//! converted from timestamps with the shell's UTC offset.
//!
//! The aggregate is kept apart from transcript history on purpose: it
//! holds counts only (no text), so deleting history or a short retention
//! window does not reset the user's stats.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Typing speed the "time saved" estimate compares against (words/minute).
pub const DEFAULT_TYPING_WPM: f64 = 40.0;
/// How many apps [`Summary::top_apps`] lists.
pub const TOP_APPS: usize = 5;
/// Days in [`Summary::last7`] (oldest first, ending today).
pub const CHART_DAYS: usize = 7;

const MS_PER_DAY: i64 = 86_400_000;

/// A local calendar date as days since 1970-01-01 (proleptic Gregorian).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDate(pub i64);

impl LocalDate {
    pub fn from_ymd(y: i64, m: u32, d: u32) -> LocalDate {
        // Howard Hinnant's days_from_civil.
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = m as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        LocalDate(era * 146_097 + doe - 719_468)
    }

    pub fn ymd(self) -> (i64, u32, u32) {
        let z = self.0 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
        (y, m, d)
    }

    /// The local date of Unix time `ts_ms` for a zone `offset_min` minutes
    /// ahead of UTC (e.g. +120 for CEST, -300 for EST).
    pub fn from_unix_ms(ts_ms: i64, offset_min: i32) -> LocalDate {
        LocalDate((ts_ms + offset_min as i64 * 60_000).div_euclid(MS_PER_DAY))
    }

    pub fn add_days(self, n: i64) -> LocalDate {
        LocalDate(self.0 + n)
    }

    /// `YYYY-MM-DD` (the aggregate's key).
    pub fn key(self) -> String {
        let (y, m, d) = self.ymd();
        format!("{y:04}-{m:02}-{d:02}")
    }

    pub fn parse(s: &str) -> Option<LocalDate> {
        let mut it = s.splitn(3, '-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let date = LocalDate::from_ymd(y, m, d);
        // Reject impossible days (2026-02-30) rather than rolling over.
        (date.ymd() == (y, m, d)).then_some(date)
    }
}

/// One day's totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayStats {
    pub words: u64,
    /// Recorded audio, milliseconds.
    pub ms: u64,
    /// Committed dictations.
    pub count: u64,
    /// Words per app id (process name, e.g. `slack.exe`; "" = unknown).
    #[serde(default)]
    pub apps: BTreeMap<String, u64>,
}

/// The persisted aggregate: `YYYY-MM-DD` (local) → [`DayStats`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregates {
    /// Existing history was folded in once (or the user reset the stats,
    /// which must not bring the history back).
    #[serde(default)]
    pub backfilled: bool,
    #[serde(default)]
    pub days: BTreeMap<String, DayStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppWords {
    pub app: String,
    pub words: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayWords {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub words: u64,
}

/// What the Insights page shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub total_words: u64,
    /// Words in the last 7 days (today and the 6 before it).
    pub week_words: u64,
    pub dictations: u64,
    pub total_ms: u64,
    /// Words per minute of recorded audio; `None` before any audio.
    pub avg_wpm: Option<f64>,
    /// Consecutive days with a dictation, ending today (or yesterday, so
    /// the streak is not "broken" before today's first dictation).
    pub current_streak: u32,
    pub longest_streak: u32,
    /// The typing speed the estimate assumes.
    pub typing_wpm: f64,
    /// Typing the same words at `typing_wpm` minus the time spent speaking
    /// (never negative).
    pub saved_minutes: f64,
    pub top_apps: Vec<AppWords>,
    /// Oldest first, ending today; days without dictation are 0.
    pub last7: Vec<DayWords>,
    /// First day with data.
    pub since: Option<String>,
}

impl Aggregates {
    /// Count one committed dictation on `day`. Empty transcripts are not
    /// dictations (nothing was inserted).
    pub fn add(&mut self, day: LocalDate, words: u64, ms: u64, app: &str) {
        if words == 0 {
            return;
        }
        let d = self.days.entry(day.key()).or_default();
        d.words += words;
        d.ms += ms;
        d.count += 1;
        *d.apps.entry(app.to_string()).or_default() += words;
    }

    /// Forget every count. Stays `backfilled` so history is not re-added.
    pub fn reset(&mut self) {
        self.days.clear();
        self.backfilled = true;
    }

    fn active_days(&self) -> Vec<LocalDate> {
        self.days
            .iter()
            .filter(|(_, s)| s.count > 0)
            .filter_map(|(k, _)| LocalDate::parse(k))
            .collect()
    }

    pub fn summary(&self, today: LocalDate, typing_wpm: f64) -> Summary {
        let mut total_words = 0;
        let mut total_ms = 0;
        let mut dictations = 0;
        let mut apps: BTreeMap<&str, u64> = BTreeMap::new();
        for s in self.days.values() {
            total_words += s.words;
            total_ms += s.ms;
            dictations += s.count;
            for (a, w) in &s.apps {
                *apps.entry(a.as_str()).or_default() += w;
            }
        }
        let mut top_apps: Vec<AppWords> = apps
            .into_iter()
            .filter(|&(_, w)| w > 0)
            .map(|(app, words)| AppWords {
                app: app.to_string(),
                words,
            })
            .collect();
        // Most words first; ties by name so the order is stable.
        top_apps.sort_by(|a, b| b.words.cmp(&a.words).then_with(|| a.app.cmp(&b.app)));
        top_apps.truncate(TOP_APPS);
        let last7: Vec<DayWords> = (0..CHART_DAYS as i64)
            .rev()
            .map(|back| {
                let key = today.add_days(-back).key();
                let words = self.days.get(&key).map_or(0, |s| s.words);
                DayWords { date: key, words }
            })
            .collect();
        let week_words = last7.iter().map(|d| d.words).sum();
        let (current_streak, longest_streak) = streaks(&self.active_days(), today);
        let minutes = total_ms as f64 / 60_000.0;
        let typing_wpm = if typing_wpm > 0.0 {
            typing_wpm
        } else {
            DEFAULT_TYPING_WPM
        };
        Summary {
            total_words,
            week_words,
            dictations,
            total_ms,
            avg_wpm: (total_ms > 0).then(|| total_words as f64 / minutes),
            current_streak,
            longest_streak,
            typing_wpm,
            saved_minutes: (total_words as f64 / typing_wpm - minutes).max(0.0),
            top_apps,
            last7,
            since: self.active_days().first().map(|d| d.key()),
        }
    }
}

/// `(current, longest)` run of consecutive days in `days` (any order,
/// duplicates allowed). The current run ends today, or yesterday when
/// today has nothing yet; days after `today` (clock moved back) are
/// ignored for it.
pub fn streaks(days: &[LocalDate], today: LocalDate) -> (u32, u32) {
    let mut d: Vec<LocalDate> = days.to_vec();
    d.sort();
    d.dedup();
    let mut longest = 0u32;
    let mut run = 0u32;
    let mut prev: Option<LocalDate> = None;
    for &day in &d {
        run = match prev {
            Some(p) if p.add_days(1) == day => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(day);
    }
    let has = |x: LocalDate| d.binary_search(&x).is_ok();
    let mut end = today;
    if !has(end) {
        end = today.add_days(-1);
    }
    let mut current = 0u32;
    while has(end) {
        current += 1;
        end = end.add_days(-1);
    }
    (current, longest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> LocalDate {
        LocalDate::parse(s).unwrap()
    }

    #[test]
    fn civil_date_round_trips() {
        assert_eq!(LocalDate::from_ymd(1970, 1, 1), LocalDate(0));
        for s in [
            "1999-12-31",
            "2000-02-29",
            "2024-02-29",
            "2026-10-28",
            "2100-03-01",
        ] {
            assert_eq!(day(s).key(), s);
        }
        assert_eq!(day("2024-03-01").0 - day("2024-02-28").0, 2, "leap day");
        assert_eq!(day("2026-03-01").0 - day("2026-02-28").0, 1);
        assert!(LocalDate::parse("2026-02-30").is_none());
        assert!(LocalDate::parse("2026-13-01").is_none());
        assert!(LocalDate::parse("junk").is_none());
    }

    #[test]
    fn local_date_uses_the_offset_across_midnight() {
        // 2026-10-28 23:30 UTC.
        let ts = day("2026-10-28").0 * MS_PER_DAY + (23 * 60 + 30) * 60_000;
        assert_eq!(LocalDate::from_unix_ms(ts, 0).key(), "2026-10-28");
        assert_eq!(LocalDate::from_unix_ms(ts, 60).key(), "2026-10-29", "UTC+1");
        assert_eq!(LocalDate::from_unix_ms(ts, -300).key(), "2026-10-28");
        // 00:10 UTC is still the previous day in New York.
        let early = day("2026-10-29").0 * MS_PER_DAY + 10 * 60_000;
        assert_eq!(LocalDate::from_unix_ms(early, -300).key(), "2026-10-28");
        // Before the epoch rounds down, not toward zero.
        assert_eq!(LocalDate::from_unix_ms(-1, 0).key(), "1969-12-31");
    }

    #[test]
    fn streak_counts_consecutive_days_ending_today() {
        let t = day("2026-10-28");
        let d = |n: i64| t.add_days(-n);
        assert_eq!(streaks(&[], t), (0, 0));
        assert_eq!(streaks(&[d(0)], t), (1, 1));
        assert_eq!(streaks(&[d(0), d(1), d(2)], t), (3, 3));
        // Nothing today yet: yesterday's streak is still alive.
        assert_eq!(streaks(&[d(1), d(2)], t), (2, 2));
        // A missed day (yesterday) ends it.
        assert_eq!(streaks(&[d(2), d(3), d(4)], t), (0, 3));
        // Longest can be an older run; order and duplicates don't matter.
        assert_eq!(
            streaks(&[d(0), d(9), d(1), d(7), d(8), d(9), d(6)], t),
            (2, 4)
        );
        // A day "in the future" (clock moved back) doesn't extend today's run.
        assert_eq!(streaks(&[d(-1), d(0)], t), (1, 2));
    }

    #[test]
    fn streak_crosses_month_year_and_leap_boundaries() {
        let run = [day("2023-12-30"), day("2023-12-31"), day("2024-01-01")];
        assert_eq!(streaks(&run, day("2024-01-01")), (3, 3));
        let leap = [day("2024-02-28"), day("2024-02-29"), day("2024-03-01")];
        assert_eq!(streaks(&leap, day("2024-03-02")), (3, 3));
        let gap = [day("2026-02-28"), day("2026-03-02")];
        assert_eq!(streaks(&gap, day("2026-03-02")), (1, 1));
    }

    #[test]
    fn streak_follows_the_local_day_not_utc() {
        // Dictations at 23:50 and 00:10 local, UTC+2: two local days.
        let base = day("2026-10-27").0 * MS_PER_DAY;
        let a = base + (21 * 60 + 50) * 60_000; // 23:50 local on the 27th
        let b = base + (22 * 60 + 10) * 60_000; // 00:10 local on the 28th
        let mut g = Aggregates::default();
        for ts in [a, b] {
            g.add(LocalDate::from_unix_ms(ts, 120), 5, 1000, "x.exe");
        }
        let s = g.summary(day("2026-10-28"), DEFAULT_TYPING_WPM);
        assert_eq!((s.current_streak, s.longest_streak), (2, 2));
        // Same instants in UTC fall on one day.
        let mut u = Aggregates::default();
        for ts in [a, b] {
            u.add(LocalDate::from_unix_ms(ts, 0), 5, 1000, "x.exe");
        }
        assert_eq!(u.days.len(), 1);
    }

    #[test]
    fn aggregates_per_day_and_app() {
        let t = day("2026-10-28");
        let mut g = Aggregates::default();
        g.add(t, 10, 6_000, "slack.exe");
        g.add(t, 20, 6_000, "code.exe");
        g.add(t.add_days(-1), 30, 12_000, "slack.exe");
        g.add(t, 0, 5_000, "slack.exe"); // empty transcript: ignored
        let today = &g.days["2026-10-28"];
        assert_eq!((today.words, today.ms, today.count), (30, 12_000, 2));
        assert_eq!(today.apps["slack.exe"], 10);
        let s = g.summary(t, DEFAULT_TYPING_WPM);
        assert_eq!(s.total_words, 60);
        assert_eq!(s.dictations, 3);
        assert_eq!(s.total_ms, 24_000);
        assert_eq!(s.avg_wpm, Some(150.0), "60 words / 0.4 min");
        assert_eq!(
            s.top_apps,
            [
                AppWords {
                    app: "slack.exe".into(),
                    words: 40
                },
                AppWords {
                    app: "code.exe".into(),
                    words: 20
                },
            ]
        );
        assert_eq!(s.since.as_deref(), Some("2026-10-27"));
        // 60 words typed at 40 wpm = 1.5 min, spoken in 0.4 min.
        assert!((s.saved_minutes - 1.1).abs() < 1e-9, "{}", s.saved_minutes);
        assert_eq!(s.typing_wpm, 40.0);
    }

    #[test]
    fn last7_and_week_cover_today_and_six_days_before() {
        let t = day("2026-10-28");
        let mut g = Aggregates::default();
        g.add(t, 1, 0, "");
        g.add(t.add_days(-6), 2, 0, "");
        g.add(t.add_days(-7), 100, 0, ""); // outside the window
        let s = g.summary(t, DEFAULT_TYPING_WPM);
        assert_eq!(s.last7.len(), 7);
        assert_eq!(s.last7[0].date, "2026-10-22");
        assert_eq!(s.last7[6].date, "2026-10-28");
        assert_eq!(
            s.last7.iter().map(|d| d.words).collect::<Vec<_>>(),
            [2, 0, 0, 0, 0, 0, 1]
        );
        assert_eq!(s.week_words, 3);
        assert_eq!(s.total_words, 103);
        assert_eq!(s.avg_wpm, None, "no audio recorded");
    }

    #[test]
    fn top_apps_are_capped_and_ties_sorted_by_name() {
        let t = day("2026-10-28");
        let mut g = Aggregates::default();
        for a in ["f", "e", "d", "c", "b", "a"] {
            g.add(t, 3, 0, a);
        }
        g.add(t, 9, 0, "z");
        let apps: Vec<String> = g
            .summary(t, DEFAULT_TYPING_WPM)
            .top_apps
            .into_iter()
            .map(|a| a.app)
            .collect();
        assert_eq!(apps, ["z", "a", "b", "c", "d"]);
    }

    #[test]
    fn saved_time_never_negative_and_bad_wpm_falls_back() {
        let t = day("2026-10-28");
        let mut g = Aggregates::default();
        g.add(t, 1, 600_000, ""); // 1 word in 10 minutes
        let s = g.summary(t, 0.0);
        assert_eq!(s.saved_minutes, 0.0);
        assert_eq!(s.typing_wpm, DEFAULT_TYPING_WPM);
    }

    #[test]
    fn reset_clears_counts_and_blocks_backfill() {
        let t = day("2026-10-28");
        let mut g = Aggregates::default();
        g.add(t, 5, 1000, "a");
        g.reset();
        assert!(g.days.is_empty());
        assert!(g.backfilled);
        let s = g.summary(t, DEFAULT_TYPING_WPM);
        assert_eq!((s.total_words, s.current_streak, s.since), (0, 0, None));
    }

    #[test]
    fn json_shape_is_stable_and_tolerant() {
        let mut g = Aggregates::default();
        g.add(day("2026-10-28"), 4, 2000, "a.exe");
        let j = serde_json::to_value(&g).unwrap();
        assert_eq!(j["days"]["2026-10-28"]["words"], 4);
        assert_eq!(j["days"]["2026-10-28"]["apps"]["a.exe"], 4);
        let back: Aggregates = serde_json::from_value(j).unwrap();
        assert_eq!(back, g);
        // Missing fields default (older/foreign writers).
        let min: Aggregates =
            serde_json::from_str(r#"{"days":{"2026-10-28":{"words":1,"ms":0,"count":1}}}"#)
                .unwrap();
        assert!(!min.backfilled);
        assert!(min.days["2026-10-28"].apps.is_empty());
    }
}
