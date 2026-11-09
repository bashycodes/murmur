#!/usr/bin/env python3
"""Write crates/murmur-core/fixtures/insights/insights_vectors.json.

Golden vectors for the Insights math (murmur_core::insights). The Rust
crate checks them (crates/murmur-core/tests/insights_vectors.rs) and so does
the Android Kotlin mirror (dev.murmur.data.Insights, InsightsVectorsTest),
so the two implementations cannot drift apart unnoticed.

The expected values below are written out by hand (not computed by either
implementation); this script only expands the repetitive parts (the 7-day
chart rows) and keeps the JSON stable. Run from the repo root:

    python3 tools/gen-insights-vectors.py
"""

import datetime as dt
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "crates/murmur-core/fixtures/insights/insights_vectors.json"


def last7(today, words):
    t = dt.date.fromisoformat(today)
    days = [(t - dt.timedelta(days=back)).isoformat() for back in range(6, -1, -1)]
    return [{"date": k, "words": words.get(k, 0)} for k in days]


def add(date, words, ms=0, app=""):
    return {"date": date, "words": words, "ms": ms, "app": app}


def summary(today, words_by_day, **k):
    s = {
        "total_words": 0, "week_words": 0, "dictations": 0, "total_ms": 0, "avg_wpm": None,
        "current_streak": 0, "longest_streak": 0, "typing_wpm": 40.0, "saved_minutes": 0.0,
        "top_apps": [], "last7": last7(today, words_by_day), "since": None,
    }
    s.update(k)
    return s


CASES = []


def case(name, today, adds, words_by_day, typing_wpm=40.0, reset=False, **expect):
    c = {"name": name, "today": today, "typing_wpm": typing_wpm, "adds": adds}
    if reset:
        c["reset"] = True
    c["expect"] = summary(today, words_by_day, **expect)
    CASES.append(c)


T = "2026-10-28"
case("empty", T, [], {})
case(
    "per_day_and_app", T,
    [add(T, 10, 6000, "com.slack"), add(T, 20, 6000, "com.code"),
     add("2026-10-27", 30, 12000, "com.slack"), add(T, 0, 5000, "com.slack")],  # 0 words: not a dictation
    {T: 30, "2026-10-27": 30},
    total_words=60, week_words=60, dictations=3, total_ms=24000,
    avg_wpm=150.0,  # 60 words / 0.4 min
    current_streak=2, longest_streak=2,
    saved_minutes=1.1,  # 60 / 40 = 1.5 min typing - 0.4 min speaking
    top_apps=[{"app": "com.slack", "words": 40}, {"app": "com.code", "words": 20}],
    since="2026-10-27",
)
case(
    "streak_survives_empty_today", T, [add("2026-10-27", 1), add("2026-10-26", 1)],
    {"2026-10-27": 1, "2026-10-26": 1},
    total_words=2, week_words=2, dictations=2, current_streak=2, longest_streak=2,
    saved_minutes=0.05, top_apps=[{"app": "", "words": 2}], since="2026-10-26",
)
case(
    "missed_yesterday_ends_streak", T, [add("2026-10-24", 1), add("2026-10-25", 1), add("2026-10-26", 1)],
    {"2026-10-24": 1, "2026-10-25": 1, "2026-10-26": 1},
    total_words=3, week_words=3, dictations=3, current_streak=0, longest_streak=3,
    saved_minutes=0.075, top_apps=[{"app": "", "words": 3}], since="2026-10-24",
)
case(
    "week_window_excludes_day_seven", T, [add(T, 1), add("2026-10-22", 2), add("2026-10-21", 100)],
    {T: 1, "2026-10-22": 2},
    total_words=103, week_words=3, dictations=3, current_streak=1, longest_streak=2,
    saved_minutes=2.575, top_apps=[{"app": "", "words": 103}], since="2026-10-21",
)
Y = "2024-01-01"
case(
    "streak_crosses_year", Y, [add("2023-12-30", 1), add("2023-12-31", 1), add(Y, 1)],
    {"2023-12-30": 1, "2023-12-31": 1, Y: 1},
    total_words=3, week_words=3, dictations=3, current_streak=3, longest_streak=3,
    saved_minutes=0.075, top_apps=[{"app": "", "words": 3}], since="2023-12-30",
)
L = "2024-03-02"
case(
    "streak_crosses_leap_day_and_survives_empty_today", L,
    [add("2024-02-28", 1), add("2024-02-29", 1), add("2024-03-01", 1)],
    {"2024-02-28": 1, "2024-02-29": 1, "2024-03-01": 1},
    total_words=3, week_words=3, dictations=3, current_streak=3, longest_streak=3,
    saved_minutes=0.075, top_apps=[{"app": "", "words": 3}], since="2024-02-28",
)
case(
    "future_day_does_not_extend_today", T, [add("2026-10-29", 1), add(T, 1)], {T: 1},
    total_words=2, week_words=1, dictations=2, current_streak=1, longest_streak=2,
    saved_minutes=0.05, top_apps=[{"app": "", "words": 2}], since=T,
)
case(
    "top_apps_capped_ties_by_name", T, [add(T, 3, 0, a) for a in "fedcba"] + [add(T, 9, 0, "z")], {T: 27},
    total_words=27, week_words=27, dictations=7, current_streak=1, longest_streak=1, saved_minutes=0.675,
    top_apps=[{"app": "z", "words": 9}] + [{"app": a, "words": 3} for a in "abcd"], since=T,
)
case(
    "saved_never_negative_bad_wpm_falls_back", T, [add(T, 1, 600000)], {T: 1}, typing_wpm=0.0,
    total_words=1, week_words=1, dictations=1, total_ms=600000, avg_wpm=0.1,
    current_streak=1, longest_streak=1, saved_minutes=0.0, top_apps=[{"app": "", "words": 1}], since=T,
)
case("reset_clears_everything", T, [add(T, 5, 1000, "a")], {}, reset=True)

STREAKS = [
    {"today": T, "days": [], "current": 0, "longest": 0},
    {"today": T, "days": [T], "current": 1, "longest": 1},
    {"today": T, "days": [T, "2026-10-27", "2026-10-26"], "current": 3, "longest": 3},
    # Nothing today yet: yesterday's streak is still alive.
    {"today": T, "days": ["2026-10-27", "2026-10-26"], "current": 2, "longest": 2},
    # A missed day (yesterday) ends it.
    {"today": T, "days": ["2026-10-26", "2026-10-25", "2026-10-24"], "current": 0, "longest": 3},
    # Longest can be an older run; order and duplicates don't matter.
    {"today": T, "days": [T, "2026-10-19", "2026-10-27", "2026-10-21", "2026-10-20", "2026-10-19", "2026-10-22"],
     "current": 2, "longest": 4},
    # A day "in the future" (clock moved back) doesn't extend today's run.
    {"today": T, "days": ["2026-10-29", T], "current": 1, "longest": 2},
    {"today": "2026-03-02", "days": ["2026-02-28", "2026-03-02"], "current": 1, "longest": 1},
    {"today": "2024-03-01", "days": ["2024-02-28", "2024-02-29", "2024-03-01"], "current": 3, "longest": 3},
]

# Unix ms + UTC offset (minutes) -> local date.
DATES = [
    {"ts_ms": 1793230200000, "offset_min": 0, "date": "2026-10-28"},  # 23:30 UTC
    {"ts_ms": 1793230200000, "offset_min": 60, "date": "2026-10-29"},
    {"ts_ms": 1793230200000, "offset_min": -300, "date": "2026-10-28"},
    {"ts_ms": 1793232600000, "offset_min": -300, "date": "2026-10-28"},  # 00:10 UTC on the 29th
    {"ts_ms": -1, "offset_min": 0, "date": "1969-12-31"},
]

if __name__ == "__main__":
    doc = {
        "_comment": "Generated by tools/gen-insights-vectors.py; checked by crates/murmur-core/tests/insights_vectors.rs "
                    "and the Android InsightsVectorsTest. Numbers compare within 1e-9.",
        "cases": CASES, "streaks": STREAKS, "dates": DATES,
    }
    OUT.write_text(json.dumps(doc, indent=1) + "\n")
    print(f"wrote {OUT} ({len(CASES)} cases)")
