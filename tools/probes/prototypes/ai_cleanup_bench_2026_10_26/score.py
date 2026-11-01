#!/usr/bin/env python3
"""Score the 2026-10-26 AI-cleanup bench.

usage: score.py LABEL=out.jsonl[:field] [...] [--fails LABEL] [--overlap]

* field: which output column to score (`output` for prod/rules runs;
  `out_none` / `out_strict` / `out_loose` for ai runs). Default `output`.
* Suite = the 88 formatting-eval cases (../formatting_eval_2026_10_24) plus the
  14 Wispr-target cases (wispr_cases.jsonl: 10 A/B pairs + 4 owner examples).
* pass: like formatting_eval score.py (curly quotes, dash variants and
  whitespace normalised; case and line breaks kept) and, per the owner, a
  final period is ignored on both sides (also before a closing quote or
  bracket). Per category.
* target-closeness (the 14 Wispr cases): 1 - token edit distance / max(len),
  where tokens are words and single punctuation marks, after the same
  normalisation. 1.0 = identical up to the final period. Also mean difflib
  character ratio and the number of exact (period-insensitive) matches.
* Guard reject rate (ai runs): share of cases whose verdict_strict /
  verdict_loose is a reject, and the share where the rejected answer was
  itself a pass (a "good answer rejected").
* llm_ms p50/p90 and how many exceed 1500 ms (OpenRouter rows: end-to-end
  from this box, request to last token). Cloud rows: mean and total usage.cost.
* --overlap: check that no prompt_v4* few-shot shares a 5-word run with a case.
"""
import difflib
import json
import os
import re
import sys
from collections import defaultdict

here = os.path.dirname(os.path.abspath(__file__))
# FMT_CASES: score against another formatting-eval file (e.g. the suite as
# it was before cases were added, to compare old and new runs on equal terms).
FMT = os.environ.get("FMT_CASES") or os.path.join(here, "..", "formatting_eval_2026_10_24", "cases.jsonl")
WISPR = os.path.join(here, "wispr_cases.jsonl")


def load(p):
    with open(p) as f:
        return [json.loads(l) for l in f if l.strip()]


def norm(s):
    s = s.replace("“", '"').replace("”", '"').replace("‘", "'").replace("’", "'")
    s = re.sub(r"\s*[—–]\s*", " - ", s)
    s = re.sub(r"[ \t]+-[ \t]+", " - ", s)
    lines = [re.sub(r"[ \t]+", " ", ln).strip() for ln in s.strip().split("\n")]
    s = "\n".join(lines)
    # Owner: ignore final-period differences (Wispr ran in formal mode).
    s = re.sub(r"\.(?=[\"')\]]*$)", "", s)
    return s


def tokens(s):
    return re.findall(r"[A-Za-z0-9$%']+|[^\sA-Za-z0-9$%']|\n", norm(s))


def edit(a, b):
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        cur = [i]
        for j, y in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x != y)))
        prev = cur
    return prev[-1]


def closeness(got, exp):
    a, b = tokens(got), tokens(exp)
    return 1.0 if not a and not b else 1 - edit(a, b) / max(len(a), len(b))


def pct(xs, q):
    if not xs:
        return 0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def cases():
    # The suite stays the 88 cases of phase 1: the 4 "regression" cases
    # integration added on 2026-10-26 (5d0747b) reuse the A/B ids ab5/ab7/ab8
    # and would collide with the Wispr cases.
    return [c for c in load(FMT) if c["category"] != "regression"] + load(WISPR)


def passes(c, got):
    return norm(got) in {norm(t) for t in [c["expected"]] + c.get("accept", [])}


def score(cs, outs, field):
    by = {o["id"]: o for o in outs}
    tot, ok = defaultdict(int), defaultdict(int)
    fails, close, ratio, llm = [], [], [], []
    rej = {"strict": 0, "loose": 0}
    good_rej = {"strict": 0, "loose": 0}
    n_fmt = n_pass_fmt = 0
    fallbacks, verdicts, costs = defaultdict(int), [], []
    for c in cs:
        o = by.get(c["id"], {})
        got = o.get(field, "")
        p = passes(c, got)
        tot[c["category"]] += 1
        ok[c["category"]] += p
        if c["category"] not in ("wispr_ab", "owner"):
            n_fmt += 1
            n_pass_fmt += p
        else:
            close.append(closeness(got, c["expected"]))
            ratio.append(difflib.SequenceMatcher(None, norm(got), norm(c["expected"])).ratio())
        if "llm_ms" in o and (o.get("llm_ms") or o.get("llm_raw") is not None):
            llm.append(o["llm_ms"])
        for g in ("strict", "loose"):
            if o.get(f"verdict_{g}"):
                rej[g] += 1
                good_rej[g] += passes(c, o.get("out_none", ""))
        if o.get("fallback"):
            fallbacks[o["fallback"]] += 1
        if o.get("verdict_strict") or o.get("verdict_loose"):
            verdicts.append((c["id"], o.get("verdict_strict"), o.get("verdict_loose")))
        if not p:
            fails.append((c, got, o))
    # Spend counts every call the run made (also cases outside the suite).
    costs = [o["cloud"].get("cost_usd") or 0.0 for o in outs if o.get("cloud")]
    return {"fallbacks": dict(fallbacks), "verdicts": verdicts, "costs": costs,"tot": tot, "ok": ok, "fmt": (n_pass_fmt, n_fmt), "close": sum(close) / len(close),
            "ratio": sum(ratio) / len(ratio), "wexact": ok["wispr_ab"] + ok["owner"], "rej": rej,
            "good_rej": good_rej, "n": len(cs), "llm": llm, "fails": fails}


def overlap():
    ex = [e for p in sorted(os.listdir(here)) if p.startswith("prompt_v4")
          for e in json.load(open(os.path.join(here, p)))["examples"]]

    def grams(s, n=5):
        w = re.findall(r"[a-z0-9']+", s.lower())
        return {" ".join(w[i:i + n]) for i in range(len(w) - n + 1)}

    hits = 0
    for e in ex:
        for c in cases():
            o = grams(e[2]) & grams(c["input"])
            if o:
                hits += 1
                print(f"overlap: example {e[2][:40]!r} ~ case {c['id']}: {sorted(o)}")
    print(f"overlap check: {len(ex)} examples x {len(cases())} cases, {hits} hits")


def main():
    args = sys.argv[1:]
    if "--overlap" in args:
        overlap()
        args.remove("--overlap")
    if "--verdicts" in args:
        args.remove("--verdicts")
    fails_for = None
    if "--fails" in args:
        i = args.index("--fails")
        fails_for = args[i + 1]
        del args[i:i + 2]
    if not args:
        return
    cs = cases()
    res = {}
    for a in args:
        label, spec = a.split("=", 1)
        path, _, field = spec.partition(":")
        res[label] = score(cs, load(path), field or "output")
    labels = list(res)
    cats = list(dict.fromkeys(c["category"] for c in cs))
    r0 = res[labels[0]]
    print("| category | n | " + " | ".join(labels) + " |")
    print("|---|---|" + "---|" * len(labels))
    for k in cats:
        print(f"| {k} | {r0['tot'][k]} | " + " | ".join(f"{res[l]['ok'][k]}/{r0['tot'][k]}" for l in labels) + " |")
    n_fmt = r0["fmt"][1]
    print(f"| **88-suite pass** | {n_fmt} | " + " | ".join(f"{res[l]['fmt'][0] / n_fmt:.0%}" for l in labels) + " |")
    print("| **Wispr exact (14)** | 14 | " + " | ".join(f"{res[l]['wexact']}/14" for l in labels) + " |")
    print("| **target-closeness (14)** | 14 | " + " | ".join(f"{res[l]['close']:.3f}" for l in labels) + " |")
    print("| Wispr char ratio (14) | 14 | " + " | ".join(f"{res[l]['ratio']:.3f}" for l in labels) + " |")
    for g in ("strict", "loose"):
        print(f"| guard {g} rejects (good rejected) | {r0['n']} | " + " | ".join(
            f"{res[l]['rej'][g]} ({res[l]['good_rej'][g]})" if res[l]["llm"] else "-" for l in labels) + " |")
    print("| llm ms p50/p90 | | " + " | ".join(
        f"{pct(res[l]['llm'], .5)}/{pct(res[l]['llm'], .9)}" if res[l]["llm"] else "-" for l in labels) + " |")
    print("| LLM calls > 1500 ms | | " + " | ".join(
        f"{sum(x > 1500 for x in res[l]['llm'])}/{len(res[l]['llm'])}" if res[l]["llm"] else "-" for l in labels) + " |")
    print("| cloud $/call mean (total) | | " + " | ".join(
        f"{sum(res[l]['costs']) / len(res[l]['costs']):.5f} ({sum(res[l]['costs']):.4f})" if res[l]["costs"] else "-"
        for l in labels) + " |")
    print("| prod fallbacks | | " + " | ".join(
        ", ".join(f"{k} {v}" for k, v in sorted(res[l]["fallbacks"].items())) or "-" for l in labels) + " |")
    if "--verdicts" in sys.argv:
        for l in labels:
            for cid, s, lo in res[l]["verdicts"]:
                print(f"- {l} {cid}: strict={s!r} loose={lo!r}")
    if fails_for:
        print(f"\n### fails: {fails_for}")
        for c, got, o in res[fails_for]["fails"]:
            v = o.get("verdict_strict") or o.get("fallback") or ""
            print(f"- {c['id']} [{c['category']}] {v}\n    in : {c['input']!r}\n    exp: {c['expected']!r}\n    got: {got!r}")


if __name__ == "__main__":
    main()
