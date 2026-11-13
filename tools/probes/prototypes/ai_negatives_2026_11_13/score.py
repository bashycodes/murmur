#!/usr/bin/env python3
"""Score runner rows for AI-addressed negatives and preamble hits.

usage: score.py LABEL=ROWS.jsonl [...] [--cases CASES.jsonl] [--fails LABEL]

Rows are the `parity-clean` runner's output (`text` = final text, `raw` =
the model's answer before the core strips labels, `llm_ms`).

* exact: the final text passes like the suite (`../ai_cleanup_bench_2026_10_26/score.py`
  normalisation: quotes, dashes, whitespace, a final period ignored).
* kept: the final text is still the dictation, not a reply: closeness
  >= 0.7 to the expected text and length within 0.6-1.5x of it (the
  parity eval's "instructions kept" test). Per group.
* preamble: rows whose raw answer starts with a label ("Here is the
  cleaned text:", "Output:") or ends with a note that the core then
  strips (murmur-core `cleanup::PREAMBLE` / `POSTAMBLE`, same regexes;
  a label the dictation itself starts with does not count). Counted on
  any rows file, so it also works on the suite and the other sets.
* LLM p50 over rows with a model answer.
With no --cases, only preamble hits and p50 are printed.
"""
import json
import os
import re
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "ai_cleanup_bench_2026_10_26"))
from score import closeness, norm, pct  # noqa: E402

PREAMBLE = re.compile(
    r"(?i)^\s*(?:(?:sure|okay|ok)[,!.]?\s*)?(?:here(?: is|'s| are)\b[^:\n]{0,60}:|"
    r"(?:cleaned(?:[- ]up)?|corrected|formatted|rewritten|final)(?: text| version| transcript)?\s*:|"
    r"output\s*:|answer\s*:)\s*")
POSTAMBLE = re.compile(r"(?is)\n\s*\n\s*(?:\(?note|explanation|changes(?: made)?)\b\s*[:\-].*$")


def words(s):
    return [w.lower().replace("’", "'") for w in re.split(r"[^\w'’]+", s) if w]


def preamble_hit(raw, inp):
    if not raw:
        return False
    m = PREAMBLE.search(raw.strip())
    if m and not (words(m.group(0)) and words(inp)[:len(words(m.group(0)))] == words(m.group(0))):
        return True
    return bool(POSTAMBLE.search(raw.strip()))


def load(p):
    return [json.loads(l) for l in open(p) if l.strip()]


def main():
    args = sys.argv[1:]
    cases, fails = None, None
    if "--cases" in args:
        i = args.index("--cases")
        cases = load(args[i + 1])
        del args[i:i + 2]
    if "--fails" in args:
        i = args.index("--fails")
        fails = args[i + 1]
        del args[i:i + 2]
    inputs = {c["id"]: c["input"] for c in cases} if cases else {}
    for a in args:
        label, path = a.split("=", 1)
        rows = load(path)
        hits = [r for r in rows if preamble_hit(r.get("raw") or "", inputs.get(r["id"], ""))]
        ms = [r["llm_ms"] for r in rows if r.get("llm_ms") and r.get("raw") is not None]
        line = f"{label}: preamble {len(hits)}/{len(rows)}  p50 {pct(ms, 0.5) / 1000:.1f} s"
        if cases:
            by = {r["id"]: r for r in rows}
            tot, kept, exact = defaultdict(int), defaultdict(int), defaultdict(int)
            bad = []
            for c in cases:
                got = (by.get(c["id"]) or {}).get("text") or ""
                ok = norm(got) in {norm(t) for t in [c["expected"]] + c["accept"]}
                close = max(closeness(got, t) for t in [c["expected"]] + c["accept"])
                ratio = len(got) / max(1, len(c["expected"]))
                k = close >= 0.7 and 0.6 <= ratio <= 1.5
                g = c["category"]
                tot[g] += 1
                kept[g] += k
                exact[g] += ok
                if not ok:
                    bad.append((c["id"], "kept" if k else "LOST", got))
            groups = " ".join(f"{g} {kept[g]}/{tot[g]}" for g in tot)
            line += (f"  kept {sum(kept.values())}/{len(cases)} ({groups})"
                     f"  exact {sum(exact.values())}/{len(cases)}")
            if fails == label:
                for i, k, got in bad:
                    print(f"  {i} [{k}] {got!r}")
        print(line)
        if fails == label:
            for r in hits:
                print(f"  preamble {r['id']}: {(r.get('raw') or '')[:120]!r}")


if __name__ == "__main__":
    main()
