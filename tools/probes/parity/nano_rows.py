#!/usr/bin/env python3
"""Turn the phone probe's folder-batch results into score.py rows.

usage: nano_rows.py BATCH.jsonl [BATCH.jsonl ...] [--set SET] [-o ROWS.jsonl]

BATCH.jsonl: what the hybrid-debug English speech probe's "Run folder batch"
writes (Android/data/<pkg>/files/nano-batch/results/batch-<UTC>.jsonl), one
line per (WAV, engine): {"set", "id", "engine": "genai"|"platform",
"mode": "advanced"|"basic"|null, "text", "first_partial_ms", "latency_ms",
"outcome", "error", ...}: the run's probe-english.jsonl line plus "set" and
"id". Early batch files named latency_ms "final_ms_after_audio_end"; both
are read.

Rows (the input of score.py from tools/probes/parity, parity-eval
branch): {"path", "clip", "text", "ms"}, with path
"gemini-nano · raw" (GenAI advanced), "genai-basic · raw" (GenAI basic) or
"platform · raw". ms is end of audio to the last final text, the closest
match to the other paths' ASR latency; a failed run scores as empty text
and carries its "outcome". A clip run twice keeps its last line.
--set keeps only that set folder's lines; it is required when the input holds
more than one set (clip ids repeat across sets, and rows carry no set).
"""
import argparse
import json
import sys

PATHS = {("genai", "advanced"): "gemini-nano · raw", ("genai", "basic"): "genai-basic · raw"}


def path_of(r):
    if r.get("engine") == "platform":
        return "platform · raw"
    return PATHS.get((r.get("engine"), r.get("mode")), f"{r.get('engine')} {r.get('mode')} · raw")


def latency(r):
    return r["latency_ms"] if "latency_ms" in r else r.get("final_ms_after_audio_end")


def rows(lines, only_set=None):
    if only_set is None:
        sets = sorted({str(r.get("set")) for r in lines})
        if len(sets) > 1:
            raise ValueError(f"input holds {len(sets)} sets ({', '.join(sets)}); pick one with --set")
    out = {}
    for r in lines:
        if only_set is not None and r.get("set") != only_set:
            continue
        row = {"path": path_of(r), "clip": r["id"], "text": r.get("text") or "",
               "ms": latency(r)}
        if r.get("outcome") != "ok":
            row["outcome"] = r.get("outcome")
            if r.get("error"):
                row["error"] = r["error"]
        out[(r.get("set"), row["path"], row["clip"])] = row
    return list(out.values())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("batch", nargs="+")
    ap.add_argument("--set", dest="only_set")
    ap.add_argument("-o", "--out")
    a = ap.parse_args()
    lines = [json.loads(s) for p in a.batch for s in open(p, encoding="utf-8") if s.strip()]
    try:
        rs = rows(lines, a.only_set)
    except ValueError as e:
        sys.exit(f"nano_rows: {e}")
    f = open(a.out, "w", encoding="utf-8") if a.out else sys.stdout
    for r in rs:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")
    bad = sum(1 for r in rs if "outcome" in r)
    print(f"{len(rs)} rows ({bad} not ok)", file=sys.stderr)


if __name__ == "__main__":
    main()
