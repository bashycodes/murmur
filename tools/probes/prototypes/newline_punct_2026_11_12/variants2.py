#!/usr/bin/env python3
"""Second pass (1.4.0): reorder the few-shots of the first-pass prompt (c+d+e) to win back
self-corrections without losing the layout gain.

usage: variants2.py PROMPT_V5_DUMP.json OUT_DIR [NAME ...]

PROMPT_V5_DUMP.json: the runner's `--dump-prompt` of the first-pass v5 (c+d+e, layout
turns last). The system text is kept; only the order (and, for some names,
the set) of few-shot turns changes. Turns are found by words in their
transcript, so the script fails loudly if the prompt changes under it.

Names:
  X  correction shots (I mean / sorry / no) move after the layout shots,
     just before the closing "only filler" turn: lists stay far from the
     transcript, corrections come nearest.
  Y  correction shots sit in the middle of the layout block (after v4c's
     three layout shots, before the three made-up ones).
  V  list and negative shots move to the start of the examples; then v4c's
     other shots, the correction shots, and the layout shots last.
  A  layout and correction shots alternate (layout, correction, ...), the
     last turn before "only filler" a layout one.
  N  X with the three made-up layout shots replaced by new shapes (review:
     the first ones mirrored suite cases: three nouns on "new line" lines,
     "comma ... exclamation point ... question mark" chat, "Period. New
     paragraph."). The new ones: two sentences on two lines with a code
     in digits (personal chat); colon, semicolon and dash in one raw
     sentence (document); the punctuated shape with "exclamation point."
     as its own sentence, one "new line" and a ", comma," (work chat).
     Checked for 5-word overlap with every case set, sets 1 and 2
     included. N shipped as prompt v6 (D89).
  Nr N with 1.3.0 v5's rule text (needs V5_DUMP=<--dump-prompt of 1.3.0
     v5>): the rewording costs ~115 tokens. Not shipped (corrections 4/7).
  Nt N without v4c's two counted-list shots ("three steps to reset it",
     "two things before the demo"), ~165 tokens less. Not shipped (lists
     1/5).
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

LIST_KEYS = ("a few things to pack", "my two goals this year", "stuff for the bake sale")
NEG_KEYS = ("no worries I grabbed", "the review is thursday not")
CORR_KEYS = ("the standup moves to ten I mean", "Grab the green bag, sorry", "Order twelve, no, fifteen")
V4C_LAYOUT_KEYS = ("new paragraph starting next week", "Packing for Sunday. New line.", "pack the tent comma")
NEW_LAYOUT_KEYS = ("errands new line post office", "good catch comma thanks", "Call the bank, comma,")


CASUAL = ("casual: relaxed chat punctuation, no final period (keep \"?\" and \"!\"); keep sentence "
          "capitals and every word except fillers")
SHOTS_N = [
    ("APP: Messages | CATEGORY: personal chat\nSTYLE: " + CASUAL,
     "the gate code is four four one two new line the spare key is under the blue pot",
     "The gate code is 4412\nThe spare key is under the blue pot"),
    ("APP: Docs | CATEGORY: document",
     "reminder colon the fridge gets cleared out on friday semicolon anything without a name on it goes dash "
     "no exceptions",
     "Reminder: the fridge gets cleared out on Friday; anything without a name on it goes — no exceptions."),
    ("APP: Slack | CATEGORY: work chat\nSTYLE: " + CASUAL,
     "Congrats on the new job, exclamation point. New line. Drinks are on me this Friday, comma, no excuses.",
     "Congrats on the new job!\nDrinks are on me this Friday, no excuses"),
]


def grams(s, n=5):
    w = re.findall(r"[a-z0-9']+", s.lower())
    return {" ".join(w[i:i + n]) for i in range(len(w) - n + 1)}


def overlap_check(texts):
    files = [os.path.join(HERE, "..", "formatting_eval_2026_10_24", "cases.jsonl"),
             os.path.join(HERE, "cases.jsonl"), os.path.join(HERE, "cases2.jsonl"),
             os.path.join(HERE, "..", "lists_corrections_2026_11_06", "cases.jsonl")]
    cases = [json.loads(l)["input"] for f in files for l in open(f) if l.strip()]
    hits = [(t, c) for t in texts for c in cases if grams(t) & grams(c)]
    for t, c in hits:
        print(f"OVERLAP: {t[:40]!r} ~ {c[:40]!r}: {sorted(grams(t) & grams(c))}")
    print(f"overlap check: {len(texts)} shots x {len(cases)} cases, {len(hits)} hits")
    return not hits


def pick(examples, keys):
    out = []
    for k in keys:
        hit = [e for e in examples if k in e[0]]
        assert len(hit) == 1, (k, len(hit))
        out.append(hit[0])
    return out


V4C_COUNTED_LIST_KEYS = ("three steps to reset it one unplug", "Two things before the demo, first")


def build(base, name):
    if name in ("Nr", "Nt"):
        p = build(base, "N")
        if name == "Nr":
            p["system"] = json.load(open(os.environ["V5_DUMP"]))["system"]
        else:
            drop = pick(p["examples"], V4C_COUNTED_LIST_KEYS)
            p["examples"] = [e for e in p["examples"] if e not in drop]
        return p
    ex = [list(e) for e in base["examples"]]
    filler = ex.pop()
    assert filler[1] == "", filler
    lists, negs, corr = pick(ex, LIST_KEYS), pick(ex, NEG_KEYS), pick(ex, CORR_KEYS)
    v4l, newl = pick(ex, V4C_LAYOUT_KEYS), pick(ex, NEW_LAYOUT_KEYS)
    moved = lists + negs + corr + v4l + newl
    rest = [e for e in ex if e not in moved]
    # first pass (c+d+e): rest, then v5's own shots in their order, then layout.
    own = [e for e in ex if e in lists + negs + corr]
    if name == "N":
        newl = [[f"{h}\nTRANSCRIPT: «{t}»", a] for h, t, a in SHOTS_N]
        order = rest + [e for e in own if e not in corr] + v4l + newl + corr
        return {"system": base["system"], "examples": order + [filler]}
    if name == "X":
        order = rest + [e for e in own if e not in corr] + v4l + newl + corr
    elif name == "Y":
        order = rest + [e for e in own if e not in corr] + v4l + corr + newl
    elif name == "V":
        order = [e for e in own if e not in corr] + rest + corr + v4l + newl
    elif name == "A":
        lay = v4l + newl
        mixed = []
        for i, e in enumerate(lay):
            mixed.append(e)
            if i < len(corr):
                mixed.append(corr[i])
        order = rest + [e for e in own if e not in corr] + mixed
    else:
        raise SystemExit(f"unknown variant {name}")
    assert sorted(map(tuple, order)) == sorted(map(tuple, ex)), name
    return {"system": base["system"], "examples": order + [filler]}


def main():
    base = json.load(open(sys.argv[1]))
    out = sys.argv[2]
    os.makedirs(out, exist_ok=True)
    if not overlap_check([t for _, t, _ in SHOTS_N]):
        raise SystemExit(1)
    for v in sys.argv[3:] or ["X", "Y", "V", "A", "N"]:
        p = build(base, v)
        with open(os.path.join(out, f"{v}.json"), "w") as f:
            json.dump(p, f, ensure_ascii=False, indent=1)
        print(f"{v}: {len(p['examples'])} shots")


if __name__ == "__main__":
    main()
