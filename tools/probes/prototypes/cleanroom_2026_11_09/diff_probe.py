"""Diff two probe JSONL outputs (before.jsonl vs after.jsonl) case by case.

Usage: python3 diff_probe.py before.jsonl after.jsonl [--full]
Prints a count and, per differing case, the case id, input and both outputs.
"""
import json
import sys


def load(p):
    out = {}
    with open(p) as f:
        for line in f:
            r = json.loads(line)
            out[r.get("id")] = r
    return out


def main():
    a, b = load(sys.argv[1]), load(sys.argv[2])
    full = "--full" in sys.argv
    diffs = []
    for k in a:
        ra, rb = a[k], b.get(k)
        if rb is None:
            diffs.append((k, ra, None))
            continue
        strip = lambda r: {x: y for x, y in r.items() if x not in ("id", "input")}
        if strip(ra) != strip(rb):
            diffs.append((k, ra, rb))
    print(f"{len(diffs)} / {len(a)} differ")
    for k, ra, rb in diffs:
        print("==", k)
        if full:
            print("  in :", json.dumps(ra.get("input"), ensure_ascii=False)[:400])
        for f in sorted(set(ra) | set(rb or {})):
            if f in ("id", "input"):
                continue
            va, vb = ra.get(f), (rb or {}).get(f)
            if va != vb:
                print(f"  {f}: {json.dumps(va, ensure_ascii=False)[:200]}")
                print(f"  {' ' * len(f)}→ {json.dumps(vb, ensure_ascii=False)[:200]}")


main()
