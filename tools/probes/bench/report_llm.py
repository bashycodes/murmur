#!/usr/bin/env python3
"""Print per-case LLM cleanup outputs side by side from one or more bench_llm.py JSONL files.

Usage: python3 -I tools/probes/bench/report_llm.py $CACHE/results/llm-*.jsonl [--case c03]
Flags per row: X = exact match, G = §7.1 guardrail would reject (raw ASR pasted instead).
"""
import argparse, json, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bench_llm import guard_reject  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--case")
    ap.add_argument("--width", type=int, default=200)
    ap.add_argument("--summary", action="store_true", help="per (prompt, model) quality table, guard recomputed")
    ap.add_argument("--latency", action="store_true",
                    help="per (prompt, threads, model): per-case MIN wall over all given runs (noise-robust on a shared box)")
    ap.add_argument("--judgements", help="llm_judgements.json: adds manual acceptable/harmful counts")
    a = ap.parse_args()
    rows = []
    for f in a.files:
        tag = pathlib.Path(f).stem.split("-")[1]
        tag = "rules" if tag.isdigit() else tag  # early runs predate the --prompt tag
        for l in open(f):
            r = json.loads(l)
            if r["kind"] == "case":
                r["_tag"] = tag
                r["_thr"] = pathlib.Path(f).stem.rsplit("-", 1)[-1]
                r["guard_reject"] = guard_reject(r["input"], r["output"], r["expected"])
                rows.append(r)
    if a.latency:
        print("| prompt | thr | model | runs | mean min-wall ms | p90 min-wall ms | mean cached-prompt tok | mean out tok | best gen tok/s |")
        print("|---|---|---|---|---|---|---|---|---|")
        for tag, thr, m in sorted({(r["_tag"], r["_thr"], r["model"]) for r in rows}):
            rs = [r for r in rows if (r["_tag"], r["_thr"], r["model"]) == (tag, thr, m)]
            per = {}
            for r in rs:
                per.setdefault(r["id"], []).append(r)
            mins = sorted(min(x["wall_ms"] for x in v) for v in per.values())
            n = len(mins)
            tps = max(x["gen_n"] / x["gen_ms"] * 1000 for x in rs if x["gen_ms"] > 0 and x["gen_n"] >= 10)
            print(f"| {tag} | {thr} | {m} | {len(rs) // n} | {sum(mins) / n:.0f} | {mins[int(0.9 * n) - 1]:.0f} | "
                  f"{sum(x['prompt_n'] for x in rs) / len(rs):.0f} | {sum(x['gen_n'] for x in rs) / len(rs):.0f} | {tps:.1f} |")
        return
    if a.summary:
        J = json.loads(pathlib.Path(a.judgements).read_text())["judgements"] if a.judgements else {}
        print("| prompt | model | exact | mean sim | better/worse vs raw | guard rejects | manual A/M/H |")
        print("|---|---|---|---|---|---|---|")
        keys = sorted({(r["_tag"], r["model"]) for r in rows})
        for tag, m in keys:
            rs = [r for r in rows if r["_tag"] == tag and r["model"] == m]
            n = len(rs)
            sim = sum(r["sim"] for r in rs) / n
            raw = sum(r["sim_raw"] for r in rs) / n
            b = sum(r["sim"] > r["sim_raw"] + 0.02 for r in rs); w = sum(r["sim"] < r["sim_raw"] - 0.02 for r in rs)
            j = J.get(f"{tag}/{m}")
            jm = "-" if not j else "/".join(str(sum(v == k for v in j.values())) for k in "AMH")
            print(f"| {tag} | {m} | {sum(r['exact'] for r in rs)}/{n} | {sim:.3f} (raw {raw:.3f}) | {b}/{w} | "
                  f"{sum(r['guard_reject'] for r in rs)} | {jm} |")
        return
    ids = sorted({r["id"] for r in rows})
    for i in ids:
        if a.case and i != a.case:
            continue
        rs = [r for r in rows if r["id"] == i]
        print(f"===== {i} {rs[0]['tags']}\n  IN : {rs[0]['input']!r}\n  EXP: {rs[0]['expected']!r}")
        for r in rs:
            fl = ("X" if r["exact"] else " ") + ("G" if r["guard_reject"] else " ")
            print(f"  {r['_tag'][:7]:7} {r['model'][:14]:14} {fl} {r['sim']:.2f} {r['output']!r}"[: a.width])


if __name__ == "__main__":
    main()
