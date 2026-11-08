#!/usr/bin/env python3
"""Key inventory of a dictations.jsonl debug log (2026-11-07 probe).

Prints row kinds and the keys each kind carries, no values.
Usage: jsonl_keys.py <dictations.jsonl>
"""
import collections
import json
import sys

kinds = collections.Counter()
keys = collections.defaultdict(collections.Counter)
for line in open(sys.argv[1], encoding="utf-8"):
    d = json.loads(line)
    k = d.get("kind")
    kinds[k] += 1
    for f, v in d.items():
        keys[k][f] += 1
        if isinstance(v, dict):
            for g in v:
                keys[k][f"{f}.{g}"] += 1
print(kinds)
for k, v in keys.items():
    print(k, dict(v))
