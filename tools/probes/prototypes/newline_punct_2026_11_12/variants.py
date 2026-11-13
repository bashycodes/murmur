#!/usr/bin/env python3
"""Prompt v5 variants for spoken line commands and spoken marks.

usage: variants.py PROMPT_V5_DUMP.json OUT_DIR

PROMPT_V5_DUMP.json: the runner's `--dump-prompt` of today's v5 (system,
examples, user). Writes one `--override` JSON per variant to OUT_DIR and
checks that no added few-shot shares a 5-word run with a suite case
(`../formatting_eval_2026_10_24/cases.jsonl`), a set-1 case
(`cases.jsonl`) or a list/correction case
(`../lists_corrections_2026_11_06/cases.jsonl`).

Variants (letters combine, e.g. "abc"):
  a  the spoken-marks / line-command rule moves to the top of the rule list
     (v5 put lists and corrections first, which pushed it down to 4th).
  b  the list rule and list few-shots stop asking for the blank line before
     a list; the core adds it after the model anyway
     (`cleanup::blank_line_before_html_lists`), so the only blank line the
     model is taught is "new paragraph"'s.
  c  reworded rules: line commands decide the layout and are never a list;
     "new line" is one line break, not a blank line; a spoken mark is never
     left as a word, also when written as its own sentence or between
     commas, and "comma" is never another mark.
  d  three made-up few-shots after v5's own: lines dictated with "new line"
     (no list), spoken marks in raw speech, spoken marks in the punctuated
     shape.
  e  v4c's own line-command and spoken-mark few-shots move from the middle
     to the end (after v5's list and correction shots, before the closing
     "only filler" turn), so the turns nearest the transcript are not all
     lists.
  i  the list rule opens with its condition: a list only when the speaker
     does not lay the text out with "new line" / "new paragraph" (c puts
     that exception at the rule's end, where the 3B ignored it).
  L  one "Layout" rule, decided in order, replaces the list rule and the
     line-command sentence of the marks rule: (1) spoken "new line" /
     "new paragraph" give exactly those breaks and nothing else; (2) else a
     counted or announced list of 3+ items is a list (v5's wording);
     (3) else plain sentences. Combine with b for the blank line.
  s  v5's bake-sale few-shot announces its list ("a list for the bake sale
     ..."): as shipped its lead-in is a bare noun phrase ("stuff for the
     bake sale"), which teaches "<noun phrase> + items = bullets", wider than
     the rule's "announces a list".
  o  v5's correction and list rules go back to v4c's place in the rule
     list (after the number and comma rules) instead of first.
"""
import json
import os
import re
import sys

here = os.path.dirname(os.path.abspath(__file__))

OLD_MARKS = ("- Spoken marks become the mark: comma, period, question mark, exclamation point, colon, semicolon, "
             "dash (—). \"new line\" starts a new line, \"new paragraph\" a blank line.\n")
NEW_MARKS = ("- Spoken marks become the mark: comma, period, question mark, exclamation point, colon, semicolon, "
             "dash (—). The spoken word never stays, also when the speech model wrote it as its own sentence "
             "(\"Period.\") or between commas (\", comma,\"), and each word gives its own mark, never another one. "
             "\"new line\" is one line break (the next words start the next line, no blank line); \"new "
             "paragraph\" is one blank line. Line commands decide the layout: the words between them stay as "
             "they were said, as plain lines, never a list.\n")
LIST_BLANK = "write the lead-in ending in a colon, a blank line, then the items:"
LIST_NOBLANK = "write the lead-in ending in a colon, then the items:"
LIST_TAIL = "commas.\n- Punctuate"
LIST_TAIL_C = ("commas. Words separated by spoken \"new line\" or \"new paragraph\" are never a list.\n"
               "- Punctuate")

LIST_HEAD = "- Lists: when the speaker counts items off"
LIST_HEAD_I = ("- Lists (only when the speaker does not lay the text out with \"new line\" or \"new paragraph\"; "
               "then keep exactly those breaks and no list): when the speaker counts items off")

LINE_SENT = " \"new line\" starts a new line, \"new paragraph\" a blank line."
LAYOUT_L = ("- Layout, decided in this order: (1) If the speaker says \"new line\" or \"new paragraph\", they "
            "lay the text out themselves: one line break for each \"new line\", one blank line for each \"new "
            "paragraph\", exactly where said, and nothing else (no list, no added colon, no other blank line). "
            "(2) Otherwise, when the speaker counts items off")

SHOT_S_OLD = ("stuff for the bake sale flour", "Stuff for the bake sale:")
SHOT_S_NEW = ("a list for the bake sale flour", "A list for the bake sale:")

# v4c's line-command / spoken-mark shots that "e" moves to the end.
MOVE_E = ("new paragraph starting next week", "Packing for Sunday. New line.", "pack the tent comma")

# Made-up few-shots for "d": (header taken from an existing turn of the same
# app / category / style, transcript, answer).
SHOTS_D = [
    ("APP: Notes | CATEGORY: other",
     "errands new line post office new line pharmacy new line car wash",
     "Errands\nPost office\nPharmacy\nCar wash"),
    ("STYLE-CASUAL-WORK",
     "good catch comma thanks exclamation point is the fix live question mark",
     "Good catch, thanks! Is the fix live?"),
    ("APP: Notes | CATEGORY: other",
     "Call the bank, comma, then the landlord. Period. New paragraph. Pay rent on the first.",
     "Call the bank, then the landlord.\n\nPay rent on the first."),
]


def header_for(examples, tag):
    if tag != "STYLE-CASUAL-WORK":
        return tag
    for u, _ in examples:
        head = u.split("\nTRANSCRIPT:")[0]
        if head.startswith("APP: Slack | CATEGORY: work chat\nSTYLE: casual"):
            return head
    raise SystemExit("no casual work-chat turn to copy the header from")


def grams(s, n=5):
    w = re.findall(r"[a-z0-9']+", s.lower())
    return {" ".join(w[i:i + n]) for i in range(len(w) - n + 1)}


def overlap_check(texts):
    files = [os.path.join(here, "..", "formatting_eval_2026_10_24", "cases.jsonl"),
             os.path.join(here, "cases.jsonl"),
             os.path.join(here, "..", "lists_corrections_2026_11_06", "cases.jsonl")]
    cases = [json.loads(l)["input"] for f in files for l in open(f) if l.strip()]
    hits = [(t, c) for t in texts for c in cases if grams(t) & grams(c)]
    for t, c in hits:
        print(f"OVERLAP: {t[:40]!r} ~ {c[:40]!r}: {sorted(grams(t) & grams(c))}")
    print(f"overlap check: {len(texts)} shots x {len(cases)} cases, {len(hits)} hits")
    return not hits


def build(base, letters):
    system = base["system"]
    examples = [list(e) for e in base["examples"]]
    assert OLD_MARKS in system and LIST_BLANK in system and LIST_TAIL in system
    marks = NEW_MARKS if "c" in letters else OLD_MARKS
    system = system.replace(OLD_MARKS, "")
    if "a" in letters:
        system = system.replace("Do all of this in one pass:\n", "Do all of this in one pass:\n" + marks)
    else:
        system = system.replace("- \"quote ... unquote\"", marks + "- \"quote ... unquote\"", 1)
    if "b" in letters:
        system = system.replace(LIST_BLANK, LIST_NOBLANK)
        for e in examples:
            e[1] = re.sub(r":\n\n<(ol|ul)>", r":\n<\1>", e[1])
    if "c" in letters:
        system = system.replace(LIST_TAIL, LIST_TAIL_C)
    if "L" in letters:
        assert LINE_SENT in system and LIST_HEAD in system
        system = system.replace(LINE_SENT, "").replace(LIST_HEAD, LAYOUT_L)
        system = system.replace("separated by commas.\n- Punctuate",
                                "separated by commas. (3) Otherwise, plain sentences.\n- Punctuate")
        assert "(3) Otherwise" in system
    if "s" in letters:
        hit = [e for e in examples if SHOT_S_OLD[0] in e[0]]
        assert len(hit) == 1
        hit[0][0] = hit[0][0].replace(*[SHOT_S_OLD[0], SHOT_S_NEW[0]])
        hit[0][1] = hit[0][1].replace(SHOT_S_OLD[1], SHOT_S_NEW[1])
    if "i" in letters:
        assert LIST_HEAD in system
        system = system.replace(LIST_HEAD, LIST_HEAD_I)
    if "o" in letters:
        lines = system.split("\n")
        rules = [l for l in lines if l.startswith("- Self-corrections:") or l.startswith("- Lists")]
        assert len(rules) == 2, rules
        rest = [l for l in lines if l not in rules]
        at = next(i for i, l in enumerate(rest) if l.startswith("- Emails:"))
        system = "\n".join(rest[:at] + rules + rest[at:])
    if "e" in letters:
        moved = [e for e in examples if any(k in e[0] for k in MOVE_E)]
        assert len(moved) == len(MOVE_E), moved
        filler = examples.pop()
        examples = [e for e in examples if e not in moved] + moved + [filler]
    if "d" in letters:
        filler = examples.pop()
        for head, t, a in SHOTS_D:
            examples.append([f"{header_for(examples, head)}\nTRANSCRIPT: «{t}»", a])
        examples.append(filler)
    return {"system": system, "examples": examples}


def main():
    base = json.load(open(sys.argv[1]))
    out = sys.argv[2]
    os.makedirs(out, exist_ok=True)
    if not overlap_check([t for _, t, _ in SHOTS_D]):
        raise SystemExit(1)
    for v in ["a", "b", "c", "d", "e", "ab", "ac", "abc", "abcd", "acd", "cd", "bc", "bcd",
              "de", "cde", "abcde", "ade", "i", "ie", "bie", "o", "oe", "boe", "bioe",
              "L", "bL", "bLe", "bLde", "s", "bs", "bies", "bLes"]:
        p = build(base, v)
        with open(os.path.join(out, f"{v}.json"), "w") as f:
            json.dump(p, f, ensure_ascii=False, indent=1)
        print(f"{v}: system {len(p['system'])} chars, {len(p['examples'])} shots")


if __name__ == "__main__":
    main()
