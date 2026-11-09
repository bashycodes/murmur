"""Probe (stray "yeah", 2026-11-07, D72 parity check): compare two
parity_ab.py runs (before / after) of one clip set. Prints numbers only:
pooled WER and term hits (with the wispr_parity scorer and its reference
file, or a plain Unicode word WER against a FLEURS-style TSV), how many
clips changed text and the shape of each change (words added / removed at
the end, elsewhere), and from the after run's `.debug` trace how many
pieces were not raised (D72) and how many of those sit under the engines'
0.003 RMS gate (they come back empty).

Usage: python3 -I compare.py SCORE_PY BEFORE.jsonl AFTER.jsonl [--refs REFS.json | --tsv DEV.tsv]
"""

import difflib
import importlib.util
import json
import re
import sys


def rows(path):
    return {r["clip"]: r for r in map(json.loads, open(path)) if r.get("clip")}


def tsv_refs(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        cols = line.rstrip("\n").split("\t")
        if len(cols) > 3:
            out[cols[1].removesuffix(".wav")] = cols[3]
    return out


def uwords(s):
    return re.findall(r"\w+", s.lower())


def main() -> None:
    score_py, before, after = sys.argv[1], sys.argv[2], sys.argv[3]
    opt = dict(zip(sys.argv[4::2], sys.argv[5::2]))
    spec = importlib.util.spec_from_file_location("score", score_py)
    sc = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sc)
    b, a = rows(before), rows(after)
    clips = sorted(set(b) & set(a))
    errs = sum(1 for c in clips if "error" in b[c] or "error" in a[c])
    if "--refs" in opt:
        refs = json.load(open(opt["--refs"]))
        for label, rs in (("before", b), ("after", a)):
            s = sc.score(refs, [{"path": label, "clip": c, "text": rs[c].get("raw", "")} for c in clips])[0]
            print(f"{label}: n {s['n']} WER {s['wer']:.4f} terms {s['terms'][0]}/{s['terms'][1]}")
    elif "--tsv" in opt:
        refs = tsv_refs(opt["--tsv"])
        for label, rs in (("before", b), ("after", a)):
            e = n = 0
            for c in clips:
                if c in refs:
                    r = uwords(refs[c])
                    e += sc.edit(r, uwords(rs[c].get("raw", "")))
                    n += len(r)
            print(f"{label}: n {sum(c in refs for c in clips)} WER {e / max(1, n):.4f}")
    changed = []
    for c in clips:
        x, y = b[c].get("raw", "").split(), a[c].get("raw", "").split()
        if x == y:
            continue
        ops = [o for o in difflib.SequenceMatcher(None, x, y, autojunk=False).get_opcodes() if o[0] != "equal"]
        shape = []
        for tag, i1, i2, j1, j2 in ops:
            where = "end" if i2 == len(x) and j2 == len(y) else ("start" if i1 == 0 and j1 == 0 else "mid")
            shape.append(f"{tag}{i2 - i1}->{j2 - j1}@{where}")
        changed.append(f"{c[:6]}…: {len(x)}->{len(y)} words, {' '.join(shape)}")
    print(f"clips {len(clips)}, errors {errs}, text changed {len(changed)}")
    for line in changed:
        print("  " + line)
    try:
        dbg = [l for l in open(after[: -len(".jsonl")] + ".debug")]
    except OSError:
        dbg = []
    pieces = lowered = gated = 0
    for l in dbg:
        kv = dict(re.findall(r"(\w+)=([\d.eE+-]+)", l))
        pieces += 1
        if float(kv["used"]) < float(kv["own"]):
            lowered += 1
            if float(kv["rms"]) * float(kv["used"]) < 0.003:
                gated += 1
    print(f"piece decodes {pieces}, not raised by D72 {lowered}, of those under the 0.003 gate {gated}")


if __name__ == "__main__":
    main()
