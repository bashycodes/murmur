#!/usr/bin/env python3
"""Runner or live-server outputs -> one rows.jsonl for the parity score.py.

  rows.py OUT.jsonl [...] > rows.jsonl

path = file stem. Runner rows ({id, text, llm_ms}) and run_paths.py server
rows ({clip, text, commit_to_result_ms}) are both accepted.
"""
import json
import os
import sys

for p in sys.argv[1:]:
    tag = os.path.basename(p).rsplit(".jsonl", 1)[0]
    for line in open(p):
        if not line.strip():
            continue
        r = json.loads(line)
        print(json.dumps({"path": tag, "clip": r.get("id") or r.get("clip"),
                          "text": r.get("text") or "",
                          "ms": r.get("commit_to_result_ms", r.get("llm_ms"))}, ensure_ascii=False))
