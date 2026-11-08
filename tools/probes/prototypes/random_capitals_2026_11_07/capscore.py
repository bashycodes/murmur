#!/usr/bin/env python3
"""Wrong mid-sentence capitals of ASR outputs against cased references.

Probe for docs/findings/2026-11-07-random-capitals.md. A word counts as a
wrong capital when the output capitalises it outside a sentence start and
the reference never capitalises it (and has it lower case). Prints counts
only, unless `--show` (owner content: never into the repo).

Usage: capscore.py REFS.json OUT.jsonl [FIELD=raw] [--show]
REFS.json = {"clips": {id: {"text": ...}}}; OUT.jsonl rows {"clip", FIELD}.
"""
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from history_midcaps import midcaps  # noqa: E402

WORD = re.compile(r"[A-Za-z][A-Za-z'’-]*")


def wrong_caps(out, ref):
    ref_words = WORD.findall(ref)
    lower = {w for w in ref_words if w[0].islower()}
    capped = {w.lower() for w in ref_words if w[0].isupper()}
    bad = []
    for _, w in midcaps(out):
        m = WORD.search(w)
        if not m:
            continue
        k = m.group(0).lower()
        if k in {x.lower() for x in lower} and k not in capped:
            bad.append(m.group(0))
    return bad


def main():
    refs = json.load(open(sys.argv[1]))["clips"]
    field = next((a for a in sys.argv[3:] if not a.startswith("--")), "raw")
    show = "--show" in sys.argv
    n_clips = n_bad = n_rows = 0
    for line in open(sys.argv[2]):
        r = json.loads(line)
        ref = refs.get(r.get("clip"))
        if not ref or not isinstance(ref, dict):
            continue
        n_rows += 1
        bad = wrong_caps(r.get(field) or "", ref["text"])
        n_bad += len(bad)
        n_clips += bool(bad)
        if show and bad:
            print(r["clip"], bad)
            print("  out>", r.get(field))
    print(f"{Path(sys.argv[2]).name}: clips={n_rows} clips_with_wrong_caps={n_clips} wrong_caps={n_bad}")


if __name__ == "__main__":
    main()
