#!/usr/bin/env python3
"""Score crates/murmur-core/tests/audio_ctx_bench.rs output (dated probe,
2026-10-22): WER and total ASR time per (model, fit_ctx), plus the clips
whose WER changed between the full window and audio_ctx.

Usage: python3 -I tools/probes/audio_ctx_wer.py results.jsonl
"""
import json, sys, os
from collections import defaultdict

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench"))
from wer import wer_counts  # noqa: E402

rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
agg = defaultdict(lambda: [0, 0, 0])
per = {}
for r in rows:
    e, n = wer_counts(r["ref"], r["hyp"])
    k = (r["model"], r["fit_ctx"])
    agg[k][0] += e; agg[k][1] += n; agg[k][2] += r["asr_ms"]
    per[(r["model"], r["fit_ctx"], r["id"])] = (e, r["hyp"])
for (m, fit), (e, n, ms) in sorted(agg.items()):
    print(f"{m:28s} fit_ctx={str(fit):5s} WER {100*e/n:5.2f}%  ASR total {ms/1000:6.1f} s")
for (m, fit, i), (e, hyp) in sorted(per.items()):
    if fit and per[(m, False, i)][0] != e:
        print(f"  {m} {i}: errors {per[(m, False, i)][0]} -> {e}: {hyp[:120]}")
