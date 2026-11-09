"""Print each differing probe case with its input from cases.json.

Usage: python3 diff_cases.py cases.json before.jsonl after.jsonl
One line per case: id | text | ctx/cfg | before-out | after-out.
"""
import json
import sys


def load(p):
    with open(p) as f:
        return {json.loads(l)["id"]: json.loads(l)["r"] for l in f}


cases = {c["id"]: c for c in json.load(open(sys.argv[1]))}
a, b = load(sys.argv[2]), load(sys.argv[3])
n = 0
for k, ra in a.items():
    rb = b.get(k)
    if ra == rb:
        continue
    n += 1
    c = dict(cases.get(k, {}))
    c.pop("id", None)
    print(f"== {k}")
    print("   in :", json.dumps(c, ensure_ascii=False)[:300])
    print("   old:", json.dumps(ra, ensure_ascii=False)[:300])
    print("   new:", json.dumps(rb, ensure_ascii=False)[:300])
print(n, "differ")
