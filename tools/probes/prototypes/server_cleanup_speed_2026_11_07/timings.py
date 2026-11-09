#!/usr/bin/env python3
"""Latency summary of live murmur-server runs (run_paths.py server output).

  timings.py server_TAG.jsonl [...] [--rows]

Per file: commit->result p50 / p90 / max, server-side asr_ms / llm_ms
p50 / max, fallback reasons. --rows prints every clip's numbers (no text).
"""
import json
import sys


def pct(v, q):
    v = sorted(v)
    if not v:
        return None
    k = (len(v) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(v) - 1)
    return round(v[lo] + (v[hi] - v[lo]) * (k - lo))


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    rows_flag = "--rows" in sys.argv
    for path in args:
        rows = [json.loads(l) for l in open(path) if l.strip()]
        c2r = [r["commit_to_result_ms"] for r in rows]
        t = [r.get("timings") or {} for r in rows]
        asr = [x.get("asr_ms", 0) for x in t]
        llm = [x.get("llm_ms", 0) for x in t]
        fb = {}
        for r in rows:
            k = r.get("fallback_reason") or "-"
            fb[k] = fb.get(k, 0) + 1
        print(f"{path}: n={len(rows)} commit->result p50 {pct(c2r, .5)} p90 {pct(c2r, .9)} "
              f"max {max(c2r)} | asr p50 {pct(asr, .5)} max {max(asr)} | llm p50 {pct(llm, .5)} "
              f"max {max(llm)} | fallback {fb}")
        if rows_flag:
            for r, x in zip(rows, t):
                print(f"  {r['clip']:<24} c2r {r['commit_to_result_ms']:>6} asr {x.get('asr_ms', 0):>5} "
                      f"llm {x.get('llm_ms', 0):>5} audio {x.get('audio_ms', 0):>6} "
                      f"raw {len(r.get('raw') or ''):>4}ch {r.get('fallback_reason') or ''}")


if __name__ == "__main__":
    main()
