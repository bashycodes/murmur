#!/usr/bin/env python3
"""Measure the AI-cleanup prompt's token count on the local engine (4096 ctx).

usage: budget_probe.py OUTDIR   -> OUTDIR/session.json, OUTDIR/cases.jsonl
       budget_probe.py OUTDIR ROWS.jsonl   -> the measurement

The cases are over-long on purpose: the engine refuses a prompt over its
context before any prefill ("prompt too long (N tokens)"), so each case costs
no inference. Two cases differ only in how many copies of one padding
sentence end the transcript; the token counts give the padding's cost per
copy and so the prompt's size without it (S = 2*N1 - N2): the system prompt,
the few-shots with their rendered headers, and a worst-case user message
(casual STYLE line, 20 DICTIONARY terms all named in the transcript, the
full 300/150-character windows around the cursor).

Run it with run.sh's environment, e.g.
  $BIN --mode cloud --cases OUTDIR/cases.jsonl --session OUTDIR/session.json \
       --llm MODEL.gguf --llm-lib $LIB --override V6.json > ROWS.jsonl
All names and text are made up.
"""
import json
import os
import re
import sys

# Made-up names; the transcript says each one split in two ("quill mate"),
# the shape the DICTIONARY selection (D64) picks up as a misspelling.
HALVES = [("Quill", "mate"), ("Marble", "ford"), ("Zen", "trova"), ("Bracken", "moor"),
          ("Fern", "leigh"), ("Corvid", "ian"), ("Tams", "worth"), ("Vell", "oran"),
          ("Kestrel", "byte"), ("Oak", "hurst"), ("Pell", "grove"), ("Sand", "ovra"),
          ("Thistle", "comb"), ("Wren", "field"), ("Amber", "vale"), ("Lumi", "castra"),
          ("North", "wyck"), ("Glimmer", "dock"), ("River", "slate"), ("Hollow", "way")]
TERMS = [a + b for a, b in HALVES]
PAD = " The crate of pears sat by the old blue door."
KS = (300, 600)


def write(out):
    os.makedirs(out, exist_ok=True)
    session = {"cleanup_level": "light", "raw_mode": False, "platform": "android",
               "spoken_cleanup": True, "vad": False, "hallucination_guard": False,
               "llm_timeout_ms": 3000,
               "styles": {"personal": "casual", "work": "casual", "email": "formal", "other": "formal"},
               "dictionary": [{"word": t, "replacement": None, "is_snippet": False} for t in TERMS]}
    json.dump(session, open(os.path.join(out, "session.json"), "w"), indent=1)
    head = "please loop in " + " and ".join(f"{a.lower()} {b}" for a, b in HALVES) + " on the thread"
    before = ("Notes from the planning call went out late, so here is the short version for anyone "
              "who missed it. ") * 4
    after = "Thanks again, see everyone on the next call. " * 4
    with open(os.path.join(out, "cases.jsonl"), "w") as f:
        for k in KS:
            f.write(json.dumps({"id": f"pad{k}", "app_category": "personal_chat", "before": before,
                                "after": after, "input": head + "." + PAD * k}) + "\n")


def measure(rows):
    n, chars = {}, {}
    for line in open(rows):
        r = json.loads(line)
        m = re.search(r"prompt too long \((\d+) tokens\)", json.dumps(r))
        if not m:
            sys.exit(f"{r['id']}: no 'prompt too long' in the row: {line[:300]}")
        n[r["id"]] = int(m.group(1))
        chars[r["id"]] = r["prompt_chars"]
    n1, n2 = n[f"pad{KS[0]}"], n[f"pad{KS[1]}"]
    per = (n2 - n1) / (KS[1] - KS[0])
    s = n1 - per * KS[0]
    print(f"N({KS[0]})={n1} N({KS[1]})={n2} padding {per:.2f} tok/copy")
    c = chars[f"pad{KS[0]}"] - len(PAD) * KS[0]
    print(f"prompt without the padding: {s:.0f} tokens, {c} chars ({c / s:.2f} chars/token); "
          f"the engine refuses past {4096 - 64}")
    room = 4096 - s
    print(f"room for the dictation, its answer (2x input + 32) and the margin: {room:.0f} tokens "
          f"-> a dictation of up to ~{(room - 64 - 32) / 3:.0f} tokens")


if __name__ == "__main__":
    measure(sys.argv[2]) if len(sys.argv) > 2 else write(sys.argv[1])
