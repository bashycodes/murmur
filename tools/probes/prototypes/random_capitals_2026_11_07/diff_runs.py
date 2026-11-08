#!/usr/bin/env python3
"""Clips whose `raw` differs between two run_set.py outputs.

Probe for docs/findings/2026-11-07-random-capitals.md. Prints the count;
`--show` prints both texts (owner content: never into the repo).
Usage: diff_runs.py BEFORE.jsonl AFTER.jsonl [--show]
"""
import json
import sys


def load(p):
    return {r["clip"]: r for r in map(json.loads, open(p, encoding="utf-8"))}


a, b = load(sys.argv[1]), load(sys.argv[2])
changed = [k for k in a if k in b and a[k]["raw"] != b[k]["raw"]]
print(f"{sys.argv[2]}: clips={len(a)} changed={len(changed)}")
if "--show" in sys.argv:
    for k in changed:
        print(" ", k)
        print("   before>", a[k]["raw"])
        print("   after> ", b[k]["raw"])
