#!/usr/bin/env python3
"""Score stt_survey JSONL (dated probe 2026-10-23) with the benchmark normaliser
(tools/probes/bench/wer.py), so numbers compare with docs/model-benchmarks.md.

Usage: python3 -I score.py run1.jsonl [run2.jsonl ...]
Per file: clip WER / RTF / mean cores used, then one row per long clip."""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "bench"))
from wer import wer_counts  # noqa: E402

for path in sys.argv[1:]:
    rows = [json.loads(l) for l in open(path) if l.startswith("{")]
    if not rows:
        print(f"{path}: no rows"); continue
    r0 = rows[0]
    clips = [r for r in rows if r["kind"] == "clip"]
    if clips:
        e = n = 0
        for r in clips:
            a, b = wer_counts(r["ref"], r["hyp"]); e += a; n += b
        inf = sum(r["infer_ms"] for r in clips); aud = sum(r["dur_s"] for r in clips) * 1e3
        cores = sum(r["cpu_ms"] for r in clips) / inf
        print(f"{os.path.basename(path)}: {r0['dir']} thr={r0['threads']} beam={r0['beam']} "
              f"load={r0['load_ms']:.0f}ms clips={len(clips)} WER={100*e/n:.2f}% ({e}/{n}) "
              f"RTF={inf/aud:.3f} mean={inf/len(clips):.0f}ms cores={cores:.2f}")
    for r in rows:
        if r["kind"] == "long":
            a, b = wer_counts(r["ref"], r["hyp"])
            print(f"   {r['id']:<16} {r['dur_s']:5.1f}s  {r['infer_ms']:7.0f} ms  RTF={r['infer_ms']/r['dur_s']/1e3:.3f} "
                  f"cores={r['cores']:.2f} WER={100*a/max(b,1):.1f}%")
