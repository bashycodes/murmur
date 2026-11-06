#!/usr/bin/env python3
"""Pass rates of runner rows on the synthetic list / correction cases.

usage: score_cases.py ROWS.jsonl [ROWS.jsonl ...] [--fails]

ROWS: the runner's output (`id`, `text`, `llm_ms`, `prompt_chars`). A case
passes when every check in `refs.json` holds (regex must match, or must not
with "absent"). Prints per group (L lists, C corrections, X both, N
negatives) and the LLM time p50 / p90.
"""
import json
import os
import re
import statistics
import sys


def check(c, text):
    found = re.search(c["re"], text, re.I if "i" in c.get("flags", "") else 0) is not None
    return not found if c.get("absent") else found


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    fails = "--fails" in sys.argv
    refs = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "refs.json")))["clips"]
    print("| rows | lists L | corrections C | both X | negatives N | all | llm p50 ms | p90 ms | prompt chars |")
    print("|---|---|---|---|---|---|---|---|---|")
    detail = []
    for path in args:
        rows = [json.loads(l) for l in open(path) if l.strip()]
        groups, ms = {}, []
        for r in rows:
            ref = refs[r["id"]]
            text = r.get("text") or ""
            bad = [c["name"] for c in ref["checks"] if not check(c, text)]
            g = groups.setdefault(r["id"][0], [0, 0])
            g[0] += not bad
            g[1] += 1
            ms.append(r.get("llm_ms") or 0)
            if bad:
                detail.append(f"{os.path.basename(path)} {r['id']}: {bad} -> {text!r}")
        tot = [sum(v[0] for v in groups.values()), sum(v[1] for v in groups.values())]
        cell = lambda k: f"{groups[k][0]}/{groups[k][1]}" if k in groups else "-"
        q = statistics.quantiles(ms, n=10)[8] if len(ms) > 1 else ms[0]
        chars = max((r.get("prompt_chars") or 0) for r in rows)
        print(f"| {os.path.basename(path)} | {cell('L')} | {cell('C')} | {cell('X')} | {cell('N')} "
              f"| {tot[0]}/{tot[1]} | {statistics.median(ms):.0f} | {q:.0f} | {chars} |")
    if fails:
        print()
        print("\n".join(detail))


if __name__ == "__main__":
    main()
