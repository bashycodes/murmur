#!/usr/bin/env python3
"""Score a context-echo runner output (see README.md).

usage: score.py CASES.jsonl OUT.jsonl [--fails]

E rows pass when the final text does not start with the end of the text
before the cursor (or end with the start of the text after it) unless the
transcript said those words. N rows pass when the `keep` phrase the speaker
dictated is still in the final text.
"""
import json
import re
import sys


def keys(s):
    return [k for k in (re.sub(r"^\W+|\W+$", "", w).lower() for w in s.split()) if k]


def has_run(hay, needle):
    n = len(needle)
    return n > 0 and any(hay[i:i + n] == needle for i in range(len(hay) - n + 1))


def echoed(text, transcript, before, after):
    t, said = keys(text), keys(transcript)
    b, a = keys(before), keys(after)
    for k in range(1, min(len(b), len(t)) + 1):
        if (k >= 2 or k == len(b)) and t[:k] == b[-k:] and not has_run(said, b[-k:]):
            return True
    for k in range(1, min(len(a), len(t)) + 1):
        if (k >= 2 or k == len(a)) and t[-k:] == a[:k] and not has_run(said, a[:k]):
            return True
    return False


def main():
    cases = {c["id"]: c for c in map(json.loads, open(sys.argv[1])) if c}
    rows = [json.loads(l) for l in open(sys.argv[2]) if l.strip()]
    fails = "--fails" in sys.argv
    score = {"E": [0, 0], "N": [0, 0]}
    for r in rows:
        c = cases[r["id"]]
        text = r["text"] or ""
        if c["group"] == "E":
            ok = not echoed(text, c["input"], c.get("before", ""), c.get("after", ""))
        else:
            ok = c["keep"].lower() in text.lower()
        score[c["group"]][0] += ok
        score[c["group"]][1] += 1
        if fails and not ok:
            print(f"  FAIL {r['id']}: {text!r}")
    ms = sorted(r["llm_ms"] for r in rows if r.get("llm_ms"))
    p50 = ms[len(ms) // 2] if ms else None
    print(f"E no echo {score['E'][0]}/{score['E'][1]}  N kept {score['N'][0]}/{score['N'][1]}  llm p50 {p50} ms")


if __name__ == "__main__":
    main()
