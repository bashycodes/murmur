#!/usr/bin/env python3
"""Synthetic dictionary-term cases for the AI cleanup (dated probe, D64).

Writes next to this script:
  cases.jsonl     runner cases (id, input, app_category)
  refs.json       checks per case (wispr_parity reference shape)
  dict-small.json the made-up user dictionary the cases use (12 entries)
  dict-20.json, dict-200.json  the same 12 plus made-up filler names, for
                  the prompt length / latency runs

Groups (first letter of the id): P a near spelling of a user entry, B a
near spelling of a built-in name (dictionary::BUILTIN_TERMS), N an ordinary
word that resembles an entry and must stay, K a tidy line with no entry
near (the prompt must stay the no-dictionary prompt). Every sentence and
name is invented; nothing comes from a real dictation.
"""
import json
import os
import random
import re

HERE = os.path.dirname(os.path.abspath(__file__))

USER_DICT = ["Quillmate", "Marbleford", "Zentrova", "Nadira", "Talvik", "Brightloom",
             "Oskarsson", "Darrow", "Kubernetes", "PostgreSQL", "Wispr", "Velloria Bay"]

# (id, input, must contain, must not contain regex or None)
CASES = [
    ("P01", "Could you pass the quill mate notes to the whole team?", "Quillmate", None),
    ("P02", "We're driving up to Marbelford on Saturday morning.", "Marbleford", None),
    ("P03", "The Zentroba contract renews at the end of March.", "Zentrova", None),
    ("P04", "Ask Nadeera if the slides are ready for the review.", "Nadira", None),
    ("P05", "The Talvic report still needs one more chart.", "Talvik", None),
    ("P06", "Bright loom shipped the new boxes yesterday afternoon.", "Brightloom", None),
    ("P07", "Mr. Oscarson called about the lease on the shop.", "Oskarsson", None),
    ("P08", "We moved the billing service to Cubernetes last week.", "Kubernetes", None),
    ("P09", "The postgres sequel backup finished at midnight.", "PostgreSQL", None),
    ("P10", "Please ask Daro to sign the form before Friday.", "Darrow", None),
    ("P11", "We booked a cabin near Valoria Bay for the long weekend.", "Velloria Bay", None),
    ("P12", "um so I think the quillmate export is broken again", "Quillmate", None),
    ("B01", "I pushed the fix to get hub right before lunch.", "GitHub", None),
    ("B02", "Add the dentist to tick tick so I don't forget it.", "TickTick", None),
    ("B03", "I'll message you on whats app later tonight.", "WhatsApp", None),
    ("B04", "Ask chat GPT for a rough first draft of the intro.", "ChatGPT", None),
    ("B05", "The power point deck is on the shared drive.", "PowerPoint", None),
    ("B06", "I wrote this whole note with whisper flow on my laptop.", "Wispr Flow", None),
    ("B07", "She posted the clip on you tube this morning.", "YouTube", None),
    ("B08", "Send the refund through pay pal when you get a chance.", "PayPal", None),
    ("N01", "Lean over and whisper the answer so nobody hears.", r"\bwhisper\b", r"Wispr"),
    ("N02", "Marble floors look great in the long hallway.", r"Marble floors", r"Marbleford"),
    ("N03", "He shot the arrow right past the old barn.", r"\barrow\b", r"Darrow"),
    ("N04", "Nadia from accounting called about the invoice.", r"Nadia\b", r"Nadira"),
    ("N05", "Oscar's son starts school in September.", r"Oscar's son", r"Oskarsson"),
    ("N06", "What's up? I'll call you once I land.", r"What's up", r"WhatsApp"),
    ("N07", "Tick the box and sign at the bottom of the page.", r"Tick the box", r"TickTick"),
    ("N08", "The river's flow slowed down after the storm.", r"river's flow", r"Wispr"),
    ("N09", "I'll get the hub cap from the garage.", r"[Hh]ub ?cap", r"GitHub"),
    ("N10", "The quill and the ink are in the top drawer.", r"quill and (the )?ink", r"Quillmate"),
    ("N11", "Bright lights make it hard to sleep at night.", r"Bright lights", r"Brightloom"),
    ("N12", "We talked about the trip, the weather, and the food.", r"talked about", r"Talvik"),
    ("K01", "The meeting moved to the small room upstairs.", r"small room upstairs", None),
    ("K02", "Thanks for the soup, it was perfect.", r"soup", None),
]


def filler_names(n, seed=7):
    """Made-up two-syllable-plus names, none near a case word."""
    rnd = random.Random(seed)
    on = ["br", "c", "d", "f", "gr", "h", "j", "k", "l", "m", "n", "pr", "r", "s", "st", "t", "v", "z"]
    vo = ["a", "e", "i", "o", "u", "ai", "ou"]
    co = ["l", "n", "r", "s", "x", "th", "nd", "lm", "rk"]
    out, seen = [], set(w.lower() for w in USER_DICT)
    while len(out) < n:
        w = "".join(rnd.choice(on) + rnd.choice(vo) for _ in range(rnd.choice([2, 3]))) + rnd.choice(co)
        w = w.capitalize()
        if w.lower() not in seen:
            seen.add(w.lower())
            out.append(w)
    return out


def entries(words):
    return [{"word": w} for w in words]


def main():
    with open(os.path.join(HERE, "cases.jsonl"), "w") as f:
        for cid, text, _, _ in CASES:
            f.write(json.dumps({"id": cid, "input": text, "app_category": "other"}) + "\n")
    clips = {}
    for cid, text, want, avoid in CASES:
        checks = [{"name": want, "re": want if cid[0] in "NK" else re.escape(want)}]
        if avoid:
            checks.append({"name": "no " + avoid, "re": avoid, "absent": True})
        clips[cid] = {"text": text, "checks": checks}
    json.dump({"_note": "synthetic dictionary-term cases, see gen_cases.py", "clips": clips},
              open(os.path.join(HERE, "refs.json"), "w"), indent=1)
    json.dump(entries(USER_DICT), open(os.path.join(HERE, "dict-small.json"), "w"), indent=1)
    for n in (20, 200):
        json.dump(entries(USER_DICT + filler_names(n - len(USER_DICT))),
                  open(os.path.join(HERE, f"dict-{n}.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
