#!/usr/bin/env python3
"""How many cases two runner outputs answer with identical final text.

  same.py A.jsonl B.jsonl    prints "same N/M" and the differing ids
"""
import json
import sys


def load(p):
    return {r.get("id") or r.get("clip"): r.get("text") for r in map(json.loads, filter(str.strip, open(p)))}


a, b = load(sys.argv[1]), load(sys.argv[2])
ids = sorted(set(a) & set(b))
diff = [i for i in ids if a[i] != b[i]]
print(f"same {len(ids) - len(diff)}/{len(ids)}", "differ:", " ".join(diff) or "-")
