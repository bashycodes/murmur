#!/usr/bin/env python3
"""Pass rates and LLM cost of runner rows on the spoken-identifier cases.

usage: score_cases.py ROWS.jsonl [ROWS.jsonl ...] [--refs REFS.json] [--fails]

ROWS: this probe's runner output (`id`, `text`, `llm_ms`, `fallback_reason`,
`prompt_chars`). A case passes when every check in the refs holds (regex
must match, or must not with "absent"). Default refs: `refs.json` next to
this script; any parity reference file works too (only clips with
checks count, so the parity eval's identifier lines can be scored with a
private refs file holding just their checks). Per file: passes per group
(first letter of the id), passes on the held-out half (refs "split":
"held"), rows the LLM ran on, mean prompt characters and LLM time p50 / p95
over the rows where it ran.
"""
import json
import os
import re
import statistics
import sys


def check(c, text):
    found = re.search(c["re"], text) is not None
    return not found if c.get("absent") else found


def pct(xs, q):
    if not xs:
        return 0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def main():
    argv = sys.argv[1:]
    fails = "--fails" in argv
    refs_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "refs.json")
    if "--refs" in argv:
        refs_path = argv[argv.index("--refs") + 1]
        argv.remove(refs_path)
    paths = [a for a in argv if not a.startswith("--")]
    refs = json.load(open(refs_path))["clips"]
    detail = []
    print("| rows | groups | cases | held | LLM ran | prompt chars | llm p50 ms | p95 ms |")
    print("|---|---|---|---|---|---|---|---|")
    for path in paths:
        rows = [json.loads(line) for line in open(path) if line.strip()]
        groups, held, ran, ms, chars = {}, [0, 0], 0, [], []
        for r in rows:
            if r.get("fallback_reason") != "llm_skipped_clean" and (r.get("llm_ms") or 0) > 0:
                ran += 1
                ms.append(r["llm_ms"])
                if r.get("prompt_chars"):
                    chars.append(r["prompt_chars"])
            ref = refs.get(r["id"]) or {}
            if not ref.get("checks"):
                continue
            text = r.get("text") or ""
            bad = [c["name"] for c in ref["checks"] if not check(c, text)]
            g = groups.setdefault(r["id"][0], [0, 0])
            g[0] += not bad
            g[1] += 1
            if ref.get("split") == "held":
                held[0] += not bad
                held[1] += 1
            if bad:
                detail.append(f"{os.path.basename(path)} {r['id']}: {bad} -> {text!r}"
                              f" ({r.get('fallback_reason')})")
        tot = [sum(v[0] for v in groups.values()), sum(v[1] for v in groups.values())]
        gs = " ".join(f"{k} {v[0]}/{v[1]}" for k, v in sorted(groups.items()))
        print(f"| {os.path.basename(path)} | {gs} | {tot[0]}/{tot[1]} | {held[0]}/{held[1]} "
              f"| {ran}/{len(rows)} | {statistics.mean(chars) if chars else 0:.0f} "
              f"| {statistics.median(ms) if ms else 0:.0f} | {pct(ms, 0.95):.0f} |")
    if fails:
        print()
        print("\n".join(detail))


if __name__ == "__main__":
    main()
