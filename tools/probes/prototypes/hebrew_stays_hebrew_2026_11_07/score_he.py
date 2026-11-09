#!/usr/bin/env python3
"""Did the cleanup keep Hebrew Hebrew? Scores runner rows on he_cases.jsonl.

usage: score_he.py CASES.jsonl ROWS.jsonl [ROWS.jsonl ...] [--show]

Per file: rows the LLM ran on; of those, how many model answers (`raw`) and
how many final texts (`text`) are Hebrew (Hebrew share of letters >= 0.5,
the same test as murmur_core::script_lock::drifted); how many final texts
the model changed (status "formatted"); the mean share of the input's
Hebrew words still in the final text (word kept) over Hebrew finals, and how
many lost more than 20 % of them (cut); and
LLM p50 / p95 ms. `--show` prints every row whose model answer was not
Hebrew.
"""
import json
import re
import statistics
import sys

HEB = re.compile(r"[֐-׿יִ-ﭏ]")
LAT = re.compile(r"[A-Za-zÀ-ɏ]")
WORD = re.compile(r"[֐-׿]+")


def hebrew(text):
    if not text:
        return None
    h, l = len(HEB.findall(text)), len(LAT.findall(text))
    if h + l < 3:
        return None
    return h / (h + l) >= 0.5


def kept(src, out):
    s = WORD.findall(src)
    o = set(WORD.findall(out))
    return sum(w in o for w in s) / len(s) if s else 1.0


def pct(xs, q):
    if not xs:
        return 0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def main():
    argv = sys.argv[1:]
    show = "--show" in argv
    paths = [a for a in argv if not a.startswith("--")]
    cases = {c["id"]: c for c in map(json.loads, open(paths[0])) if c}
    print("| rows | LLM ran | answer Hebrew | final Hebrew | final formatted | words kept | cut | llm p50 / p95 ms |")
    print("|---|---|---|---|---|---|---|---|")
    for path in paths[1:]:
        rows = [json.loads(l) for l in open(path) if l.strip()]
        ran = [r for r in rows if r.get("fallback_reason") != "llm_skipped_clean" and (r.get("llm_ms") or 0) > 0]
        ans = sum(hebrew(r.get("raw")) is True for r in ran)
        fin = sum(hebrew(r.get("text")) is True for r in ran)
        fmt = sum(str(r.get("status")).lower() == "formatted" for r in ran)
        ks = [kept(cases[r["id"]]["input"], r["text"]) for r in ran if hebrew(r.get("text"))]
        ms = [r["llm_ms"] for r in ran]
        n = len(ran)
        print(
            f"| {path.split('/')[-1]} | {n}/{len(rows)} | {ans}/{n} | {fin}/{n} | {fmt}/{n} | "
            f"{statistics.mean(ks) if ks else 0:.2f} | {sum(k < 0.8 for k in ks)} | {pct(ms, .5)} / {pct(ms, .95)} |"
        )
        if show:
            for r in ran:
                if hebrew(r.get("raw")) is not True:
                    print(f"  {r['id']} [{r.get('fallback_reason')}] raw={r.get('raw')!r} final={r['text']!r}")


if __name__ == "__main__":
    main()
