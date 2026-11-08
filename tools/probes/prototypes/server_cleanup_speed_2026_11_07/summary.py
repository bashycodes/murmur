#!/usr/bin/env python3
"""Latency summary of runner output (server-speed-clean rows).

  summary.py OUT.jsonl [...] [--rows]

Per file: commit / llm ms p50, p90, max; plugin counters (prompt, prefill
and generated tokens, prefill and generation ms) as medians; fallback
reasons. --rows prints each case's numbers (no text).
"""
import json
import sys


def pct(v, q):
    v = sorted(x for x in v if x is not None)
    if not v:
        return None
    k = (len(v) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(v) - 1)
    return round(v[lo] + (v[hi] - v[lo]) * (k - lo))


def main():
    files = [a for a in sys.argv[1:] if not a.startswith("--")]
    for path in files:
        rows = [json.loads(l) for l in open(path) if l.strip()]
        last = [r.get("last") or {} for r in rows]
        col = lambda k: [x.get(k) for x in last]
        fb = {}
        for r in rows:
            k = r.get("fallback_reason") or "-"
            fb[k] = fb.get(k, 0) + 1
        c = [r["commit_ms"] for r in rows]
        l = [r["llm_ms"] for r in rows]
        print(f"{path.rsplit('/', 1)[-1]}: n={len(rows)} commit p50 {pct(c, .5)} p90 {pct(c, .9)} max {max(c)}"
              f" | llm p50 {pct(l, .5)} max {max(l)} | prompt {pct(col('prompt_tokens'), .5)}"
              f" prefill {pct(col('prefill_tokens'), .5)} tok / {pct(col('prefill_ms'), .5)} ms"
              f" gen {pct(col('gen_tokens'), .5)} tok / {pct(col('gen_ms'), .5)} ms | {fb}")
        if "--rows" in sys.argv:
            for r, x in zip(rows, last):
                print(f"  {r['id']:<8} commit {r['commit_ms']:>6} llm {r['llm_ms']:>6} "
                      f"prefill {x.get('prefill_tokens')}/{x.get('prefill_ms')}ms "
                      f"gen {x.get('gen_tokens')}/{x.get('gen_ms')}ms {r.get('fallback_reason') or ''}")


if __name__ == "__main__":
    main()
