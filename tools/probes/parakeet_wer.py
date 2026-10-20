#!/usr/bin/env python3
"""Score spot_check JSONL (crates/flow-asr-sherpa/examples/spot_check.rs) with
the benchmark normaliser (tools/probes/bench/wer.py) so the numbers compare
with docs/model-benchmarks.md.  Usage: parakeet_wer.py out.jsonl [...]"""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench"))
from wer import wer_counts  # noqa: E402

for path in sys.argv[1:]:
    err = words = 0
    rows = [json.loads(l) for l in open(path) if l.strip()]
    for r in rows:
        e, n = wer_counts(r["ref"], r["hyp"])
        err += e; words += n
        if e:
            print(f"  {r['id']}: {e} err: {r['hyp'][:100]}")
    infer = sum(r["infer_ms"] for r in rows) / 1e3
    audio = sum(r["dur_s"] for r in rows)
    print(f"{path}: {len(rows)} clips, WER {100*err/words:.1f}% ({err}/{words}), RTF {infer/audio:.3f}")

# With BASELINE=file.jsonl: count words whose surface form (case, attached
# punctuation) changed against the baseline hyp - WER's normaliser hides
# these, but a dictation user sees them.
base = os.environ.get("BASELINE")
if base:
    b = {r["id"]: r["hyp"].split() for r in map(json.loads, open(base))}
    for path in sys.argv[1:]:
        changed = 0
        for r in map(json.loads, open(path)):
            h, g = r["hyp"].split(), b[r["id"]]
            changed += sum(1 for x, y in zip(h, g) if x != y and x.lower() == y.lower()) + abs(len(h) - len(g))
        print(f"{path}: {changed} surface-only word changes vs {os.path.basename(base)}")
