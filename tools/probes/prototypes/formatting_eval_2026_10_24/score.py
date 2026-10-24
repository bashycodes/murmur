#!/usr/bin/env python3
"""Score formatting-eval outputs against cases.jsonl.

usage: score.py cases.jsonl LABEL=out.jsonl [LABEL=out.jsonl ...] [--fails LABEL] [--md]

Per option: exact-match rate (equal to `expected` up to trailing whitespace: the Android insert adds a space), pass rate
(normalised match against `expected` or any `accept`), mean normalised
edit similarity (difflib ratio, best over expected+accept), per-category
pass rate, and llm_ms / commit_ms p50/p90. Normalisation: curly quotes ->
straight, dash variants with spacing -> " - ", spaces collapsed per line,
trailing spaces stripped; case and newlines are kept (they are formatting).
"""
import difflib
import json
import re
import sys
from collections import defaultdict


def norm(s: str) -> str:
    s = s.replace("“", '"').replace("”", '"').replace("‘", "'").replace("’", "'")
    s = re.sub(r"\s*[—–]\s*", " - ", s)
    s = re.sub(r"[ \t]+-[ \t]+", " - ", s)
    lines = [re.sub(r"[ \t]+", " ", ln).strip() for ln in s.strip().split("\n")]
    return "\n".join(lines)


def pct(xs, q):
    if not xs:
        return 0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def load(p):
    with open(p) as f:
        return [json.loads(l) for l in f if l.strip()]


def score(cases, outs):
    by_id = {o["id"]: o for o in outs}
    cat_tot, cat_pass = defaultdict(int), defaultdict(int)
    exact = passed = 0
    sims, llm, commit, fails, fallbacks = [], [], [], [], defaultdict(int)
    for c in cases:
        o = by_id.get(c["id"])
        got = (o or {}).get("output", "")
        targets = [c["expected"]] + c.get("accept", [])
        ok = norm(got) in {norm(t) for t in targets}
        sim = max(difflib.SequenceMatcher(None, norm(got), norm(t)).ratio() if (got or t) else 1.0 for t in targets)
        exact += got.rstrip() == c["expected"].rstrip()
        passed += ok
        sims.append(sim)
        cat_tot[c["category"]] += 1
        cat_pass[c["category"]] += ok
        if o:
            llm.append(o.get("llm_ms", 0))
            commit.append(o.get("commit_ms", 0))
            if o.get("fallback"):
                fallbacks[o["fallback"]] += 1
        if not ok:
            fails.append((c, got, o))
    n = len(cases)
    return {
        "n": n, "exact": exact / n, "pass": passed / n, "sim": sum(sims) / n,
        "cats": {k: (cat_pass[k], cat_tot[k]) for k in cat_tot},
        "llm_p50": pct(llm, 0.5), "llm_p90": pct(llm, 0.9),
        "commit_p50": pct(commit, 0.5), "commit_p90": pct(commit, 0.9),
        "fallbacks": dict(fallbacks), "fails": fails,
    }


def main():
    args = sys.argv[1:]
    fails_for = None
    if "--fails" in args:
        i = args.index("--fails")
        fails_for = args[i + 1]
        del args[i:i + 2]
    cases = load(args[0])
    res = {}
    for a in args[1:]:
        label, path = a.split("=", 1)
        res[label] = score(cases, load(path))
    cats = list(dict.fromkeys(c["category"] for c in cases))
    labels = list(res)
    print("| category | n | " + " | ".join(labels) + " |")
    print("|---|---|" + "---|" * len(labels))
    for k in cats:
        n = res[labels[0]]["cats"][k][1]
        print(f"| {k} | {n} | " + " | ".join(f"{res[l]['cats'][k][0]}/{n}" for l in labels) + " |")
    for key, fmt in [("pass", "{:.0%}"), ("exact", "{:.0%}"), ("sim", "{:.3f}")]:
        print(f"| **{key}** | {len(cases)} | " + " | ".join(fmt.format(res[l][key]) for l in labels) + " |")
    print("| llm ms p50/p90 | | " + " | ".join(f"{res[l]['llm_p50']}/{res[l]['llm_p90']}" for l in labels) + " |")
    print("| commit ms p50/p90 | | " + " | ".join(f"{res[l]['commit_p50']}/{res[l]['commit_p90']}" for l in labels) + " |")
    print("| fallbacks | | " + " | ".join(", ".join(f"{k} {v}" for k, v in res[l]["fallbacks"].items()) or "-" for l in labels) + " |")
    if fails_for:
        print(f"\n### fails: {fails_for}")
        for c, got, o in res[fails_for]["fails"]:
            fb = (o or {}).get("fallback") or ""
            print(f"- {c['id']} [{c['category']}/{c['style']}] {fb}\n    in : {c['input']!r}\n    exp: {c['expected']!r}\n    got: {got!r}")


if __name__ == "__main__":
    main()
