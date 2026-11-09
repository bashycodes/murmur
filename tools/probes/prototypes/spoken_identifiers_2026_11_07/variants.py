#!/usr/bin/env python3
"""Prompt variants for the spoken-identifier probe (2026-11-07, D67).

usage: variants.py DUMP_DIR OUT_DIR

DUMP_DIR holds the runner's `--dump-prompt` files of the base prompts:
`dump-cloud.json` (v5) and `dump-local.json` (v2; murmur-server sends the
same). Writes `--override` JSONs per variant, `NAME-cloud.json` and
`NAME-local.json` (+ `NAME-server.json`, a copy): the base system prompt
with a code-name rule inserted, and few-shot turns added before the
closing "only filler" turn. Every example is made up; none names a
parity-eval term or shares a 4-word run with the synthetic cases' inputs
(checked at the end; the spoken format cue itself, "in caps with
underscores", is allowed).
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CAT = {"N": "other", "P": "personal chat", "W": "work chat", "D": "document"}
CASUAL = {
    "cloud": ("casual: relaxed chat punctuation, no final period (keep \"?\" and \"!\"); keep sentence capitals "
              "and every word except fillers"),
    "local": "casual: sentence case, drop optional commas, final period optional",
}

# The rule, v5 wording (inserted before the numbers rule).
RULE_V5 = (
    "- Code names: when the speaker says how a name is written (\"in caps\", \"all caps\", \"with underscores\", "
    "\"underscore\", \"snake case\", \"camel case\", \"pascal case\", or \"dash\" / \"dot\" between the words of one "
    "name), write that one name in that form and drop the format words (\"retry limit in caps with underscores\" -> "
    "RETRY_LIMIT, \"parse date in camel case\" -> parseDate, \"build dot gradle\" -> build.gradle). The name of an "
    "environment variable or a constant is written in caps with underscores even when no form is said. A technical "
    "name spoken with its number takes its usual written form (\"m p three\" -> MP3, \"w p a two\" -> WPA2). These "
    "words in their ordinary sense stay words (\"a dash of salt\", \"write it in caps\").\n")
# v2 wording: shorter, same content.
RULE_V2 = (
    "- Code names: a name said with its form (\"in caps\", \"with underscores\", \"underscore\", \"snake case\", "
    "\"camel case\", \"dash\" or \"dot\" between its words) is written in that form without the format words "
    "(\"retry limit in caps with underscores\" → RETRY_LIMIT, \"parse date in camel case\" → parseDate); an "
    "environment variable or constant name is caps with underscores even when no form is said; a technical name "
    "with a number takes its usual written form (\"m p three\" → MP3). In their ordinary sense these words stay.\n")

# (app, category, casual, transcript, output)
SHOTS = [
    ("Slack", "W", True, "the helper is called parse invoice date in camel case and it lives in utils",
     "The helper is called parseInvoiceDate and it lives in utils"),
    ("Notes", "N", False, "before the deploy export log verbosity as an environment variable set to two",
     "Before the deploy, export LOG_VERBOSITY as an environment variable set to 2."),
    ("Notes", "N", False, "our router still runs w p a two so the old tablet can't join",
     "Our router still runs WPA2, so the old tablet can't join."),
    ("Messages", "P", True, "the kids drew a snake with a dot for each eye and it's all caps and glitter",
     "The kids drew a snake with a dot for each eye and it's all caps and glitter"),
]

# More spoken-form shots (variant "shots2"): caps, snake case, spoken
# underscores; the dash one only for v5 (on v2 the rules already turned a
# spoken "dash" into " — ").
MORE_SHOTS = [
    ("Slack", "W", True, "the key is retry budget in caps with underscores and it defaults to ten",
     "The key is RETRY_BUDGET and it defaults to 10"),
    ("Notes", "N", False, "store it in a column named created by user in snake case",
     "Store it in a column named created_by_user."),
    ("Notes", "N", False, "the file is called draft underscore notes underscore old",
     "The file is called draft_notes_old."),
]
DASH_SHOT = ("Slack", "W", True, "push it to the branch hot dash fix dash menu when you're done",
             "Push it to the branch hot-fix-menu when you're done")


def turn(mode, app, cat, casual, transcript, out):
    s = f"APP: {app} | CATEGORY: {CAT[cat]}\n"
    if casual:
        s += f"STYLE: {CASUAL[mode]}\n"
    return [s + f"TRANSCRIPT: «{transcript}»", out]


def insert_rule(system, rule, before):
    i = system.index(before)
    return system[:i] + rule + system[i:]


def build(dump, mode, rule, shots):
    sysp = dump["system"]
    if rule:
        before = "- Write numbers as digits" if mode == "cloud" else "- Write numbers, money"
        sysp = insert_rule(sysp, rule, before)
    ex = list(dump["examples"])
    ex = ex[:-1] + [turn(mode, *s) for s in shots] + ex[-1:]
    return {"system": sysp, "examples": ex}


VARIANTS = {
    # name: (rule?, shots)
    "rule": (True, []),
    "shots": (True, SHOTS),
    "shots2": (True, MORE_SHOTS + SHOTS),
    # v5 sits ~660 tokens under the 4096-token context of the local
    # engine (shots2 overflowed it): the short rule and two shots only.
    "lite": ("short", SHOTS[:2]),
}


def grams(text, n=4):
    w = re.findall(r"[a-z0-9']+", text.lower())
    return {tuple(w[i:i + n]) for i in range(len(w) - n + 1)}


def main():
    dump_dir, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    dumps = {m: json.load(open(os.path.join(dump_dir, f"dump-{m}.json"))) for m in ("cloud", "local")}
    for name, (rule, shots) in VARIANTS.items():
        for mode, d in dumps.items():
            r = (RULE_V5 if mode == "cloud" else RULE_V2) if rule else ""
            if rule == "short":
                r = RULE_V2.replace("→", "->")
            s = shots + [DASH_SHOT] if mode == "cloud" and name == "shots2" else shots
            v = build(d, mode, r, s)
            for m in ([mode] if mode == "cloud" else ["local", "server"]):
                json.dump(v, open(os.path.join(out_dir, f"{name}-{m}.json"), "w"), ensure_ascii=False)
    cases = [json.loads(l)["input"] for l in open(os.path.join(HERE, "cases.jsonl"))]
    case_grams = set().union(*(grams(c) for c in cases))
    cue = {tuple("in caps with underscores".split())}
    for s in SHOTS + MORE_SHOTS + [DASH_SHOT]:
        shared = (grams(s[3]) & case_grams) - cue
        if shared:
            print("shot shares with the cases:", s[3], sorted(shared))
    print("variants:", ", ".join(VARIANTS))


if __name__ == "__main__":
    main()
