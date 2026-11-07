#!/usr/bin/env python3
"""Score `joins core` / `joins pieces` output: WER per clip and mode, numbers only.

usage: wer.py SCORE_PY REFS.json OUT.jsonl [--field asr_text]

SCORE_PY is the parity eval's score.py (its `wer` normalises case,
punctuation and spoken numbers). Prints no clip text, so the output can be
pasted into findings. For `pieces` output it scores the joined pieces with
and without prior_text, and counts repeated n-grams at joins.
"""
import importlib.util
import json
import sys


def load(path):
    spec = importlib.util.spec_from_file_location("score", path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def join_repeats(pieces, n=4):
    """Joins where the right piece repeats a word n-gram from the left piece's tail."""
    hits = 0
    for left, right in zip(pieces, pieces[1:]):
        lw = [w.strip(".,?!\"'").lower() for w in left.split()][-30:]
        rw = [w.strip(".,?!\"'").lower() for w in right.split()][:30]
        grams = {tuple(lw[i : i + n]) for i in range(len(lw) - n + 1)}
        if any(tuple(rw[i : i + n]) in grams for i in range(len(rw) - n + 1)):
            hits += 1
    return hits


def main():
    sc = load(sys.argv[1])
    refs = json.load(open(sys.argv[2]))["clips"]
    field = sys.argv[sys.argv.index("--field") + 1] if "--field" in sys.argv else "asr_text"
    rows = [json.loads(l) for l in open(sys.argv[3])]
    tot = {}
    for r in rows:
        ref = refs[r["clip"]]["text"]
        if "mode" in r:
            e, n = sc.wer(ref, r[field] or "")
            print(f"{r['clip']} {r['mode']:5} segs={r.get('segments')} WER={e / n:.1%}")
            tot.setdefault(r["mode"], []).append((e, n))
        else:
            for k in ("no_prior", "with_prior"):
                e, n = sc.wer(ref, " ".join(t for t in r[k] if t))
                rep = join_repeats([t for t in r[k] if t])
                print(f"{r['clip']} {k:10} pieces={len(r[k])} joins_repeating={rep} WER={e / n:.1%}")
                tot.setdefault(k, []).append((e, n))
    for k, v in tot.items():
        print(f"ALL {k}: mean {sum(e / n for e, n in v) / len(v):.1%}, pooled {sum(e for e, _ in v) / sum(n for _, n in v):.1%}")


if __name__ == "__main__":
    main()
