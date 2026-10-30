#!/usr/bin/env python3
"""Median llm_ms by dictation length (words) per llm_win.py result file (dated probe, 2026-10-30)."""
import json, statistics, sys, pathlib

B = [(0, 10, "<=10"), (11, 20, "11-20"), (21, 35, "21-35"), (36, 10**9, ">35")]
print("| run | " + " | ".join(f"{b[2]} words" for b in B) + " | gen tok/s | p50 | p90 |")
print("|---|" + "---|" * (len(B) + 3))
for f in sys.argv[1:]:
    rows = [json.loads(l) for l in open(f, encoding="utf-8") if l.strip()]
    cells = []
    for lo, hi, _ in B:
        xs = [r["llm_ms"] for r in rows if lo <= r["in_words"] <= hi]
        cells.append(f"{statistics.median(xs):.0f} (n={len(xs)})" if xs else "-")
    g = [r for r in rows if r.get("gen_n") and r["gen_n"] >= 10]
    tps = sum(r["gen_n"] for r in g) / sum(r["gen_ms"] for r in g) * 1000 if g else 0
    ms = sorted(r["llm_ms"] for r in rows)
    lab = pathlib.Path(f).stem.removeprefix("v4c-").rsplit("-2026", 1)[0]
    print(f"| {lab} | " + " | ".join(cells) + f" | {tps:.0f} | {ms[len(ms) // 2]} | {ms[int(0.9 * len(ms)) - 1]} |")
