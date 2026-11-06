#!/usr/bin/env python3
"""Synthetic spoken-number cases (2026-11-06 probe).

Writes `cases.jsonl` (runner input: id, input, app_category) and
`refs.json` (regex checks per case) next to this script. Every sentence is
made up; none comes from a real dictation. Inputs are Parakeet-style:
capitalised and punctuated, numbers left as words, so before D63 the
on-device path skipped every one of them as "already clean".

Groups (id prefix):
  T  clock times ("eleven forty five", "seven a m")
  M  money
  O  ordinals and dates
  P  percentages
  D  degrees
  C  counts and quantities with units
  S  digit runs (codes, numbers read digit by digit)
  N  negatives: number idioms that must stay words ("one of those",
     "a second ago", "the first time", "on second thought", "a couple")
  K  neutral tidy lines with no number (the detector must not fire; the
     row shows whether the LLM ran)
"""
import json
import os


def has(name, pat):
    return {"name": name, "re": pat}


def no(name, pat):
    return {"name": name, "re": pat, "absent": True}


CASES = [
    # T: clock times
    ("T01", "The dentist moved my appointment to eleven forty five.", [has("11:45", r"11:45")]),
    ("T02", "Let's grab coffee at nine thirty tomorrow.", [has("9:30", r"9:30")]),
    ("T03", "The bus leaves at seven ten from the north gate.", [has("7:10", r"7:10")]),
    ("T04", "Set an alarm for six a m.", [has("6 am", r"\b6 ?(am|a\.m\.|AM)")]),
    ("T05", "The webinar starts at four pm sharp.", [has("4 pm", r"\b4 ?(pm|p\.m\.|PM)")]),
    ("T06", "Dinner is booked for eight o'clock on Saturday.", [has("8", r"\b8( o'clock|:00)?")]),
    ("T07", "The pharmacy closes at five fifty five today.", [has("5:55", r"5:55")]),
    ("T08", "Our train gets in at twelve twenty.", [has("12:20", r"12:20")]),
    # M: money
    ("M01", "The repair came to two hundred and forty dollars.", [has("$240", r"\$240")]),
    ("M02", "Tickets are fifteen dollars each.", [has("$15", r"\$15")]),
    ("M03", "She lent me fifty euros for the trip.", [has("50 euros", r"(€ ?50|50 ?(euros|€))")]),
    ("M04", "The monthly plan costs nine ninety nine.", [has("9.99", r"9\.99")]),
    # O: ordinals and dates
    ("O01", "The rent is due on the first of March.", [has("1st/March 1", r"(1st of March|March 1)")]),
    ("O02", "Her birthday is on the twenty second.", [has("22nd", r"22nd")]),
    ("O03", "The workshop runs until the ninth.", [has("the 9th", r"the 9th")]),
    ("O04", "We fly out on July fourth.", [has("July 4", r"July 4")]),
    ("O05", "The package arrives on the eighteenth.", [has("18th", r"the 18th")]),
    ("O06", "We moved the party to the thirtieth, after the holiday.", [has("30th", r"the 30th")]),
    # P: percentages
    ("P01", "Sales went up by twelve percent this quarter.", [has("12%", r"12 ?(%|percent)")]),
    ("P02", "The battery was at eighty five percent.", [has("85%", r"85 ?(%|percent)")]),
    # D: degrees
    ("D01", "Bake it at one hundred and eighty degrees for twenty minutes.",
     [has("180°", r"180 ?(°|degrees)"), has("20 minutes", r"\b20 minutes")]),
    ("D02", "Tomorrow it will drop to minus four degrees.", [has("-4°", r"(-|minus )4 ?(°|degrees)")]),
    # C: counts with units
    ("C01", "The file is about thirty megabytes.", [has("30 MB", r"\b30 ?(MB|megabytes)")]),
    ("C02", "We need sixteen chairs for the hall.", [has("16 chairs", r"\b16 chairs")]),
    ("C03", "The hike is roughly twelve kilometers.", [has("12 km", r"\b12 ?(km|kilometers)")]),
    ("C04", "Room two hundred and five is on the left.", [has("205", r"\b205\b")]),
    ("C05", "Platform nine has the delayed train.", [has("platform 9", r"[Pp]latform 9")]),
    # S: digit runs
    ("S01", "The gate code is four eight one six.", [has("4816", r"4816")]),
    ("S02", "My booking reference is five two zero nine one.", [has("52091", r"52091")]),
    ("S03", "Call the front desk on extension three three seven.", [has("337", r"\b337\b")]),
    # N: idioms that must stay words
    ("N01", "I'll take one of those blue folders.", [has("one of those", r"\b[Oo]ne of those")]),
    ("N02", "He called a second ago.", [has("a second ago", r"a second ago")]),
    ("N03", "It was the first time I tried sushi.", [has("first time", r"first time")]),
    ("N04", "On second thought, let's stay home.", [has("second thought", r"[Ss]econd thought")]),
    ("N05", "We need a couple of volunteers.", [has("a couple", r"a couple")]),
    ("N06", "I have two of those, and one of them is broken.",
     [has("one of them", r"\b[Oo]ne of them"), no("1 of", r"\b1 of")]),
    ("N07", "Give me a minute, I'm on the other line.", [has("a minute", r"a minute")]),
    ("N08", "No one told me about the change.", [has("No one", r"[Nn]o one")]),
    ("N09", "Those two are best friends.", [has("those two", r"[Tt]hose two")]),
    ("N10", "Once in a while we eat out.", [has("once", r"[Oo]nce in a while")]),
    # N: ordinals used as nouns or ranks (the detector stays quiet) and a
    # lone small cardinal in an idiom (the detector fires; the model must
    # keep the words)
    ("N18", "Her office is on the fourth floor.", [has("fourth floor", r"(fourth|4th) floor")]),
    ("N19", "I reminded him for the fifth time.", [has("fifth time", r"fifth time")]),
    ("N20", "Let's pick the third option.", [has("third option", r"third option")]),
    ("N21", "He finished fourth in the race.", [has("fourth", r"finished fourth")]),
    ("N22", "The two of us went hiking.", [has("two of us", r"[Tt]wo of us")]),
    ("N23", "I'm in two minds about the trip.", [has("two minds", r"two minds")]),
    ("N24", "Respect is a two-way street.", [has("two-way", r"two-way")]),
    ("N25", "It takes two to tango.", [has("two to tango", r"two to tango")]),
    ("N26", "Give me one or two days to think.", [has("one or two", r"one or two days")]),
    ("N27", "Three's a crowd, so I'll stay home.", [has("Three's", r"Three's a crowd")]),
    # N with a number the rules leave, so the LLM runs on the idiom too
    ("N11", "One of those vans came by at ten forty.",
     [has("One of those", r"\b[Oo]ne of those"), has("10:40", r"10:40")]),
    ("N12", "He called a second ago about the call at nine fifteen.",
     [has("a second ago", r"a second ago"), has("9:15", r"9:15")]),
    ("N13", "The first time we met was on the twelfth.",
     [has("first time", r"[Ff]irst time"), has("12th", r"12th")]),
    ("N14", "On second thought, let's leave at six twenty.",
     [has("second thought", r"[Ss]econd thought"), has("6:20", r"6:20")]),
    ("N15", "A couple of us will get there around four forty.",
     [has("a couple", r"[Aa] couple of us"), has("4:40", r"4:40")]),
    ("N16", "No one else signed up before the fifth.",
     [has("No one", r"[Nn]o one"), has("5th", r"5th")]),
    ("N17", "Give me a minute, the shuttle leaves at eleven fifty.",
     [has("a minute", r"a minute"), has("11:50", r"11:50")]),
    # K: neutral tidy lines
    ("K01", "Please water the plants while I'm away.", []),
    ("K02", "The printer on the third floor is jammed again.", [has("third floor", r"(third|3rd) floor")]),
    ("K03", "Can you check whether the backup finished?", []),
    ("K04", "The new logo looks great on the website.", []),
]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "cases.jsonl"), "w") as f:
        for cid, text, _ in CASES:
            f.write(json.dumps({"id": cid, "input": text, "app_category": "other"}) + "\n")
    refs = {"_note": "synthetic spoken-number cases, see gen_cases.py",
            "clips": {cid: {"text": text, "checks": checks} for cid, text, checks in CASES}}
    with open(os.path.join(here, "refs.json"), "w") as f:
        json.dump(refs, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print(len(CASES), "cases")


if __name__ == "__main__":
    main()
