#!/usr/bin/env python3
"""Dated probe (2026-10-23): what LLM cleanup costs on the home murmur-server.

Consumer: docs/findings/2026-10-23-server-llm-cleanup-cost.md (tables).
Not a permanent tool.

Input: JSONL rows written by
  cargo run --release -p murmur-contract-tests --features parakeet \
      --example hybrid_latency -- --remote-only --reps 3 --jsonl OUT.jsonl ...
one file per configuration (e.g. before.jsonl = server without llm_path,
after.jsonl = same bench with llm_path set).

  server_llm_cost_2026_10_23.py before.jsonl after.jsonl [--texts]

Prints a per-clip markdown table (medians over the reps) and the added
release->text latency (after - before, per clip median) median / p90.
--texts also prints asr_text vs final_text of rep 0 for the quality notes.
"""
import json
import statistics as st
import sys
from collections import defaultdict


def load(p):
    by = defaultdict(list)
    for line in open(p):
        r = json.loads(line)
        by[r["file"]].append(r)
    return by


def med(xs):
    return round(st.median(xs)) if xs else 0


def p90(xs):
    xs = sorted(xs)
    if not xs:
        return 0
    k = (len(xs) - 1) * 0.9
    lo, hi = int(k), min(int(k) + 1, len(xs) - 1)
    return round(xs[lo] + (xs[hi] - xs[lo]) * (k - lo))


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    texts = "--texts" in sys.argv
    before, after = load(args[0]), load(args[1])
    clips = sorted(before, key=lambda f: before[f][0]["timings"]["audio_ms"])
    print("| clip | audio s | before asr | before e2e | after asr | after llm | after e2e | added | after status (reps) |")
    print("|---|---|---|---|---|---|---|---|---|")
    added, b_all, a_all, llm_all = [], [], [], []
    for f in clips:
        b, a = before[f], after.get(f, [])
        be = med([r["remote_commit_ms"] for r in b])
        ae = med([r["remote_commit_ms"] for r in a])
        b_all += [r["remote_commit_ms"] for r in b]
        a_all += [r["remote_commit_ms"] for r in a]
        llm_all += [r["timings"]["llm_ms"] for r in a]
        added.append(ae - be)
        statuses = ",".join(
            r["status"] + (f"({r['fallback_reason']})" if r.get("fallback_reason") else "")
            for r in a
        )
        print(
            f"| {f} | {b[0]['timings']['audio_ms'] / 1000:.1f} "
            f"| {med([r['timings']['asr_ms'] for r in b])} | {be} "
            f"| {med([r['timings']['asr_ms'] for r in a])} "
            f"| {med([r['timings']['llm_ms'] for r in a])} | {ae} | {ae - be:+d} | {statuses} |"
        )
    print()
    print(f"release->text all runs: before median {med(b_all)} p90 {p90(b_all)}; "
          f"after median {med(a_all)} p90 {p90(a_all)}")
    print(f"llm_ms all runs: median {med(llm_all)} p90 {p90(llm_all)}")
    print(f"added per clip (median of reps): median {med(added)} p90 {p90(added)}")
    if texts:
        for f in clips:
            for r in after.get(f, [])[:1]:
                print(f"\n## {f} [{r['status']} {r.get('fallback_reason')}]")
                print("ASR:  ", r["asr_text"])
                print("FINAL:", r["final_text"])


if __name__ == "__main__":
    main()
