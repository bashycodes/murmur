#!/usr/bin/env python3
"""Check that no prompt few-shot shares a 5-word run with a test case.

usage: overlap.py PROMPT_DUMP.json [CASES.jsonl ...]

PROMPT_DUMP: the runner's `--dump-prompt` output. CASES: runner case files
(`input`); default `cases.jsonl` next to this script. Exit 1 on overlap.
"""
import json
import os
import re
import sys


def grams(t, n=5):
    w = re.findall(r"[a-z0-9']+", t.lower())
    return {tuple(w[i:i + n]) for i in range(len(w) - n + 1)}


def main():
    dump = json.load(open(sys.argv[1]))
    cases = sys.argv[2:] or [os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases.jsonl")]
    texts = [json.loads(l)["input"] for p in cases for l in open(p) if l.strip()]
    bad = 0
    for user, answer in dump["examples"]:
        g = grams(user) | grams(answer)
        for t in texts:
            if g & grams(t):
                print("overlap:", user[-80:], "|", t)
                bad += 1
    print(f"{len(dump['examples'])} few-shots x {len(texts)} cases, {bad} overlaps")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
