#!/usr/bin/env python3
"""Synthetic spoken-identifier cases for the AI cleanup (dated probe, D67).

Writes next to this script:
  cases.jsonl  runner cases (id, input, app_category)
  refs.json    checks per case (wispr_parity reference shape) plus "split"

Groups (first letter of the id):
  I  the speaker says how to write a name: "in caps with underscores",
     "snake case", "camel case", "underscore", "dash", "dot"
  C  a code name with no spoken format, where the kind of name sets the
     form (an environment variable or constant is caps with underscores)
  V  a technical name spoken with its number ("python three twelve",
     "utf eight", "risc five"), which has a usual compact written form
  N  ordinary prose with the same words ("in caps", "underscore", "dash",
     "camel", "snake", "dot", "version", numbers): nothing becomes code

Every second case of a group is "held": it was not looked at while the
prompt was tuned (score_cases.py reports dev and held apart). Every
sentence and name is invented; nothing comes from a real dictation or the
parity eval (checked with overlap.py).
"""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))


def has(*pats):
    return [{"name": p, "re": p} for p in pats]


def lacks(*pats):
    return [{"name": "no " + p, "re": p, "absent": True} for p in pats]


# (id, app_category, input, checks)
CASES = [
    # I: spoken format words.
    ("I01", "other", "set max retry count in caps with underscores to five",
     has(r"\bMAX_RETRY_COUNT\b")),
    ("I02", "other", "the constant is default page size all caps with underscores",
     has(r"\bDEFAULT_PAGE_SIZE\b")),
    ("I03", "other", "call the function load user profile in snake case",
     has(r"\bload_user_profile\b")),
    ("I04", "other", "name the variable order total in camel case",
     has(r"\borderTotal\b")),
    ("I05", "other", "the new class is invoice printer in pascal case",
     has(r"\bInvoicePrinter\b")),
    ("I06", "other", "check out the branch fix dash login dash timeout",
     has(r"\bfix-login-timeout\b")),
    ("I07", "other", "the setting is cache underscore ttl underscore seconds",
     has(r"\bcache_ttl_seconds\b")),
    ("I08", "other", "open config dot local dot json and change the port",
     has(r"\bconfig\.local\.json\b")),
    ("I09", "other", "run it again with the flag dash dash dry dash run",
     has(r"--dry-run\b")),
    ("I10", "other", "rename it to user id in caps with underscores please",
     has(r"\bUSER_ID\b")),
    ("I11", "other", "the field is called last seen at in snake case",
     has(r"\blast_seen_at\b")),
    ("I12", "other", "make the method name is ready for review in camel case",
     has(r"\bisReadyForReview\b")),
    # C: the kind of name sets the form.
    ("C01", "other", "export the environment variable queue depth limit before you start the worker",
     has(r"\bQUEUE_DEPTH_LIMIT\b")),
    ("C02", "other", "the env var poll interval millis is missing on staging",
     has(r"\bPOLL_INTERVAL_MILLIS\b")),
    ("C03", "other", "add a constant called max upload bytes at the top of the module",
     has(r"\bMAX_UPLOAD_BYTES\b")),
    ("C04", "other", "set the environment variable sentry dsn on the build server",
     has(r"\bSENTRY_DSN\b")),
    ("C05", "other", "we read the database host from the env var called db host",
     has(r"\bDB_HOST\b")),
    ("C06", "other", "the environment variable feature beta search is set to true in production",
     has(r"\bFEATURE_BETA_SEARCH\b")),
    # V: technical names with a number.
    ("V01", "other", "we still build with python three twelve on the old runner",
     has(r"\bPython 3\.12\b")),
    ("V02", "other", "save the file as utf eight before you commit it",
     has(r"\bUTF-8\b")),
    ("V03", "other", "the camera only records h two six four video",
     has(r"\bH\.?264\b")),
    ("V04", "other", "the new board has a risc five chip",
     has(r"\bRISC-V\b")),
    ("V05", "other", "the server only listens on I P V six",
     has(r"\bIPv6\b")),
    ("V06", "other", "the docker image is built for arm sixty four only",
     has(r"\b(arm|ARM)64\b")),
    ("V07", "other", "bump the package to version two point four point one",
     has(r"\b2\.4\.1\b")),
    ("V08", "other", "that old laptop is x eighty six so the new build won't run on it",
     has(r"\b[xX]86\b")),
    # N: prose, nothing becomes code.
    ("N01", "other", "write your name in caps at the top of the form",
     has(r"in caps") + lacks(r"_", r"\bNAME\b")),
    ("N02", "other", "the underscore key on my keyboard keeps sticking",
     has(r"[Uu]nderscore key") + lacks(r"_")),
    ("N03", "other", "she added a dash of lemon to the soup and it was perfect",
     has(r"dash of lemon") + lacks(r"_")),
    ("N04", "other", "the camel walked slowly across the sand at sunset",
     has(r"camel walked") + lacks(r"[a-z][A-Z]", r"_")),
    ("N05", "other", "the zoo moved the snake case closer to the entrance",
     has(r"snake case") + lacks(r"_")),
    ("N06", "other", "dot the i's and cross the t's before you send it",
     has(r"[Dd]ot the i") + lacks(r"\.i\b")),
    ("N07", "other", "the team ordered new caps with the club logo on them",
     has(r"new caps") + lacks(r"_", r"CLUB")),
    ("N08", "other", "if you type in all caps people think you are shouting",
     has(r"all caps") + lacks(r"_")),
    ("N09", "other", "his version of the story was very different from hers",
     has(r"version of the story") + lacks(r"_")),
    ("N10", "other", "we drove eighty six miles to see the lake on sunday",
     has(r"86 miles") + lacks(r"x86", r"_")),
    ("N11", "other", "the server crashed again last night so I restarted it this morning",
     has(r"server crashed") + lacks(r"_", r"`")),
    ("N12", "other", "can you underline the title and make a dash for the bus after",
     has(r"make a dash") + lacks(r"_")),
]


def main():
    clips = {}
    with open(os.path.join(HERE, "cases.jsonl"), "w") as f:
        for n, (cid, cat, text, checks) in enumerate(CASES):
            f.write(json.dumps({"id": cid, "input": text, "app_category": cat}) + "\n")
            split = "held" if int(re.sub(r"\D", "", cid)) % 2 == 0 else "dev"
            clips[cid] = {"text": text, "checks": checks, "split": split}
    json.dump({"_note": "synthetic spoken-identifier cases, see gen_cases.py", "clips": clips},
              open(os.path.join(HERE, "refs.json"), "w"), indent=1)
    print(f"{len(CASES)} cases")


if __name__ == "__main__":
    main()
