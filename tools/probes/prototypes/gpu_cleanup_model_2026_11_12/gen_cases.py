#!/usr/bin/env python3
"""bench_cases.jsonl from tools/probes/bench/cleanup_cases.json (the 19
hand-written cases of docs/model-benchmarks.md "Cleanup LLM") in the runner's
{id, input, app_category} shape, with `expected` kept for score.py."""
import json
import os

here = os.path.dirname(os.path.abspath(__file__))
src = os.path.join(here, "..", "..", "bench", "cleanup_cases.json")
CAT = {"email": "email", "work": "work_chat", "personal": "personal_chat", "other": "other"}
with open(os.path.join(here, "bench_cases.jsonl"), "w") as out:
    for c in json.load(open(src))["cases"]:
        row = {"id": c["id"], "input": c["input"], "app_category": CAT[c["category"]], "expected": c["expected"]}
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
