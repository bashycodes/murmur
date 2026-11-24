#!/usr/bin/env python3
"""Score runner rows: planted names spelled right, llm_timeout rate, llm_ms.

    score.py OUT_DIR/*.jsonl

A case counts when every planted name is in the output exactly (case
included); "any case" ignores case. One row per file and per (file, variant), as a Markdown table.
"""

import json
import pathlib
import sys


def p95(xs):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(0.95 * (len(xs) - 1))))] if xs else 0


def stats(rows):
    n = len(rows)
    ok = sum(all(p in (r["text"] or "") for p in r["planted"]) for r in rows)
    names = sum(len(r["planted"]) for r in rows)
    names_ok = sum(p in (r["text"] or "") for r in rows for p in r["planted"])
    any_case = sum(all(p.lower() in (r["text"] or "").lower() for p in r["planted"]) for r in rows)
    timeouts = sum(r["fallback_reason"] == "llm_timeout" for r in rows)
    other_fb = sum(bool(r["fallback_reason"]) and r["fallback_reason"] != "llm_timeout" for r in rows)
    ms = [r["llm_ms"] for r in rows]
    prompt = [((r.get("last") or {}).get("prompt_tokens") or 0) for r in rows]
    return (f"{ok}/{n} | {any_case}/{n} | {100 * names_ok / names:.0f} % | {timeouts}/{n} ({100 * timeouts / n:.0f} %) "
            f"| {other_fb} | {sorted(ms)[len(ms) // 2]} | {p95(ms)} | {max(ms)} "
            f"| {sum(r['user_chars'] for r in rows) // n} | {max(prompt)}")


def main():
    print("| run | variant | cases right | any case | names right | llm_timeout | other fallback "
          "| p50 llm_ms | p95 llm_ms | max llm_ms | avg user chars | max prompt tokens |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for f in sys.argv[1:]:
        rows = [json.loads(l) for l in open(f) if l.strip()]
        tag = pathlib.Path(f).stem
        print(f"| {tag} | all | {stats(rows)} |")
        for v in ("short", "full"):
            sub = [r for r in rows if r["variant"] == v]
            if sub:
                print(f"| {tag} | {v} | {stats(sub)} |")


if __name__ == "__main__":
    main()
