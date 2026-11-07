#!/usr/bin/env python3
"""Privacy check: word 4-grams shared by a branch's added lines and private refs.

usage: git diff BASE -U0 | overlap.py REFS.json [REFS.json ...]

REFS: wispr_parity reference files (`clips.*.text`) or any JSON / text file
(every string in it is read). Prints each added line that shares a 4-word
run with a reference, the shared runs, and a total; exit 1 when any. Words
are lowercase letters, digits and apostrophes, so case and punctuation do
not hide a copy.
"""
import json
import re
import sys

W = re.compile(r"[a-z0-9']+")


def grams(text, n=4):
    w = W.findall(text.lower())
    return {tuple(w[i:i + n]) for i in range(len(w) - n + 1)}


def strings(v):
    if isinstance(v, str):
        yield v
    elif isinstance(v, dict):
        for x in v.values():
            yield from strings(x)
    elif isinstance(v, list):
        for x in v:
            yield from strings(x)


def main():
    ref = set()
    for p in sys.argv[1:]:
        raw = open(p, encoding="utf-8").read()
        try:
            texts = list(strings(json.loads(raw)))
        except json.JSONDecodeError:
            texts = [raw]
        for t in texts:
            ref |= grams(t)
    hits = 0
    for line in sys.stdin:
        if not line.startswith("+") or line.startswith("+++"):
            continue
        shared = grams(line[1:]) & ref
        if shared:
            hits += 1
            print(f"{line[1:].strip()[:100]!r}: {sorted(' '.join(g) for g in shared)}")
    print(f"{hits} added lines share a 4-word run with the references")
    sys.exit(1 if hits else 0)


if __name__ == "__main__":
    main()
