#!/usr/bin/env python3
"""One row per model over the four suites run.sh writes.

usage: score.py TAG [TAG ...] [--fails]

* bench (19 cases, expected texts): exact match and mean similarity
  (1 - char Levenshtein / max length) after normalising both sides: HTML
  list tags and list markers dropped (the expected texts use <ol>/<ul>, the
  product writes plain-text lists), whitespace collapsed. `raw sim` is the
  input pasted unchanged; better/worse = cases more than 0.02 above / below it.
* numbers (61 cases): cases whose regex checks all hold (numbers_2026_11_06
  refs.json).
* Hebrew (28 dev + 14 holdout): model answers that are Hebrew, final texts
  that are Hebrew (the score_he.py test, D65), mean share of the input's
  Hebrew words kept, finals that lost more than 20 % of them ("cut"),
  finals with a Latin-letter word the input does not have ("leak": an
  English word inside Hebrew), "junk": finals with a leak or any Arabic,
  Cyrillic, CJK or U+FFFD character (always harmful: the D65 script check
  passes them because the line stays mostly Hebrew), and finals whose Hebrew words differ from the
  input's at all ("reworded"; listed by --fails for a manual harm check),
  finals with any punctuation mark ("punct.": the inputs have none, so this
  is how often the model punctuated a Hebrew line at all).
* tokens: mean prompt tokens prefilled per dictation (the cached prefix not
  counted) and mean generated tokens, from the plugin's `last` counters; the
  GPU latency estimate in docs/model-benchmarks.md is built from these.
* fallback: rows whose answer the core rejected (guard, script check, timeout).
"""
import json
import os
import re
import statistics
import sys

here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(here, "..", "hebrew_stays_hebrew_2026_11_07"))
from score_he import hebrew, kept  # noqa: E402

P = os.path.join(here, "..")
FOREIGN = re.compile(r"[\u0400-\u04ff\u0600-\u06ff\u3000-\u9fff\uff00-\uffef\ufffd]")
PUN = re.compile(r"[,.?!:;]")
LATIN = re.compile(r"[A-Za-z]{2,}")
HEW = re.compile(r"[\u0590-\u05ff]+")


def lev(a, b):
    if len(a) < len(b):
        a, b = b, a
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def norm(t):
    t = re.sub(r"</?(ol|ul|li|p|br)\s*/?>", "\n", t or "")
    t = re.sub(r"(?m)^\s*(\d+[.)]|[-*•])\s+", "", t)
    return re.sub(r"\s+", " ", t).strip()


def sim(a, b):
    a, b = norm(a), norm(b)
    m = max(len(a), len(b))
    return 1.0 if m == 0 else 1 - lev(a, b) / m


def rows(tag, suite):
    p = os.path.join(here, "results", f"{tag}-{suite}.jsonl")
    if not os.path.exists(p):
        return None
    return [json.loads(l) for l in open(p) if l.strip()]


def cases(path):
    return {c["id"]: c for c in (json.loads(l) for l in open(path) if l.strip())}


def check(c, text):
    found = re.search(c["re"], text) is not None
    return not found if c.get("absent") else found


def main():
    argv = sys.argv[1:]
    fails = "--fails" in argv
    tags = [a for a in argv if not a.startswith("--")]
    bench = cases(os.path.join(here, "bench_cases.jsonl"))
    refs = json.load(open(os.path.join(P, "numbers_2026_11_06", "refs.json")))["clips"]
    he = cases(os.path.join(P, "hebrew_stays_hebrew_2026_11_07", "he_cases.jsonl"))
    he.update(cases(os.path.join(P, "hebrew_stays_hebrew_2026_11_07", "he_holdout.jsonl")))
    raw_sim = statistics.mean(sim(c["input"], c["expected"]) for c in bench.values())
    print(f"bench raw-input sim {raw_sim:.3f}")
    print("| model | bench exact | bench sim | better/worse | numbers pass | HE answer Hebrew | HE final Hebrew"
          " | HE words kept | HE cut | HE leak | HE junk | HE reworded | HE punct. | fallbacks | prefill tok / gen tok (mean) | CPU LLM p50 s |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    detail = []
    for tag in tags:
        cells, allrows = [], []
        b = rows(tag, "bench")
        if b:
            allrows += b
            ss = [sim(r["text"], bench[r["id"]]["expected"]) for r in b]
            base = [sim(bench[r["id"]]["input"], bench[r["id"]]["expected"]) for r in b]
            ex = sum(norm(r["text"]) == norm(bench[r["id"]]["expected"]) for r in b)
            bw = (sum(s > x + 0.02 for s, x in zip(ss, base)), sum(s < x - 0.02 for s, x in zip(ss, base)))
            cells += [f"{ex}/{len(b)}", f"{statistics.mean(ss):.3f}", f"{bw[0]}/{bw[1]}"]
            for r, s in zip(b, ss):
                if s < 0.9:
                    detail.append(f"{tag} bench {r['id']} {s:.2f}: {r['text']!r}")
        else:
            cells += ["-"] * 3
        n = rows(tag, "numbers")
        if n:
            allrows += n
            ok = 0
            for r in n:
                bad = [c["name"] for c in refs[r["id"]]["checks"] if not check(c, r.get("text") or "")]
                ok += not bad
                if bad:
                    detail.append(f"{tag} numbers {r['id']} {bad}: {r.get('text')!r}")
            cells.append(f"{ok}/{len(n)}")
        else:
            cells.append("-")
        h = (rows(tag, "he") or []) + (rows(tag, "ho") or [])
        if h:
            allrows += h
            ans = sum(hebrew(r.get("raw")) is True for r in h)
            fin = sum(hebrew(r.get("text")) is True for r in h)
            ks = [kept(he[r["id"]]["input"], r["text"]) for r in h if hebrew(r.get("text"))]
            leak = re_w = junk = 0
            for r in h:
                src, out = he[r["id"]]["input"], r.get("text") or ""
                lk = bool(set(LATIN.findall(out)) - set(LATIN.findall(src)))
                leak += lk
                if lk or FOREIGN.search(out):
                    junk += 1
                    detail.append(f"{tag} he {r['id']} junk: {out!r}")
                if HEW.findall(src) != HEW.findall(out):
                    re_w += 1
                    detail.append(f"{tag} he {r['id']} reworded: {out!r}")
            cells += [f"{ans}/{len(h)}", f"{fin}/{len(h)}", f"{statistics.mean(ks):.2f}", str(sum(k < 0.8 for k in ks)),
                      str(leak), str(junk), str(re_w), f"{sum(bool(PUN.search(r.get('text') or '')) for r in h)}/{len(h)}"]
            for r in h:
                if hebrew(r.get("raw")) is not True:
                    detail.append(f"{tag} he {r['id']} not Hebrew [{r.get('fallback_reason')}]: {r.get('raw')!r}")
        else:
            cells += ["-"] * 8
        fb = sum(1 for r in allrows if r.get("fallback_reason") and r["fallback_reason"] != "llm_skipped_clean")
        last = [r["last"] for r in allrows if isinstance(r.get("last"), dict)]
        pre = [l.get("prefill_tokens") or l.get("prefilled") for l in last]
        gen = [l.get("gen_tokens") or l.get("generated") for l in last]
        pre = [x for x in pre if isinstance(x, (int, float))]
        gen = [x for x in gen if isinstance(x, (int, float))]
        ms = [r["llm_ms"] for r in allrows if r.get("llm_ms")]
        cells += [
            f"{fb}/{len(allrows)}",
            f"{statistics.mean(pre):.0f} / {statistics.mean(gen):.0f}" if pre and gen else "-",
            f"{statistics.median(ms) / 1000:.1f}" if ms else "-",
        ]
        print(f"| {tag} | " + " | ".join(cells) + " |")
    if fails:
        print("\n".join(detail))


if __name__ == "__main__":
    main()
