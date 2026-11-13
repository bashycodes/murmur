#!/usr/bin/env python3
"""Synthetic check cases for spoken line commands, spoken marks and lists.

usage: gen_cases.py      -> cases.jsonl  (set 1)
       gen_cases.py 2    -> cases2.jsonl (set 2)

Made-up dictations in the formatting-eval case shape (`id`, `category`,
`app_category`, `style`, `input`, `expected`, `accept`), so
`../ai_cleanup_bench_2026_10_26/score.py` scores them with
`FMT_CASES=cases.jsonl`. They are written fresh for this probe and no
few-shot shares a 5-word run with them (`variants2.py` checks). Inputs come in both raw (no punctuation) and the
punctuated shape Parakeet gives (commands between commas or as their own
"sentence").

Groups:
* newline (10): "new line" / "new paragraph" between sentences, between
  short items (which must not become a list), in a greeting and in an email.
* spoken_punct (9): comma, period, question mark, exclamation point, colon,
  semicolon, dash.
* list (6): counted and announced lists with no line command, so a fix
  for line commands that stops making lists shows up here.

Set 1 stopped being held out: every first-pass variant was scored on it
and the winner was picked partly on it. Set 2 (`CASES2`) was written
after that, with new shapes (longer sentences per line, addresses,
sign-offs, a short poem, "step one / step two" lines, colon, semicolon
and dash in the punctuated shape, "exclamation mark", a mark word used as
a word, an inline item run), before the final prompt was measured. Only
the final candidate and the v5 baseline are scored on it.
"""
import json
import os
import sys

O, P, W, E = "other", "personal_chat", "work_chat", "email"


def case(i, cat, app, style, text, exp, *accept):
    return {"id": i, "category": cat, "app_category": app, "style": style,
            "input": text, "expected": exp, "accept": list(accept)}


CASES = [
    # --- spoken line commands
    case("H01", "newline", O, "neutral", "things to bring new line towels new line sunscreen new line snacks",
         "Things to bring\nTowels\nSunscreen\nSnacks", "Things to bring:\nTowels\nSunscreen\nSnacks"),
    case("H02", "newline", O, "neutral", "The meeting went well. New line. Next steps are below.",
         "The meeting went well.\nNext steps are below."),
    case("H03", "newline", P, "casual", "Morning, new line, the parcel arrived today.",
         "Morning\nThe parcel arrived today", "Morning,\nThe parcel arrived today",
         "Morning.\nThe parcel arrived today"),
    case("H04", "newline", O, "neutral", "Options for dinner. New paragraph. Pasta or tacos.",
         "Options for dinner.\n\nPasta or tacos."),
    case("H05", "newline", E, "formal",
         "Dear Priya, new paragraph, the invoice is attached. New paragraph. Best, Tom.",
         "Dear Priya,\n\nThe invoice is attached.\n\nBest,\nTom"),
    case("H06", "newline", O, "neutral", "the first draft is done new line the review starts monday",
         "The first draft is done.\nThe review starts Monday.", "The first draft is done\nThe review starts Monday"),
    case("H07", "newline", O, "neutral", "Room 4B, new line, floor 2, new line, east building.",
         "Room 4B\nFloor 2\nEast building", "Room 4B,\nFloor 2,\nEast building",
         "Room 4B\nfloor 2\neast building"),
    case("H08", "newline", O, "neutral", "we shipped the fix new paragraph the next release is in may",
         "We shipped the fix.\n\nThe next release is in May."),
    case("H09", "newline", W, "casual", "Quick update, new line, the build is green, new line, the docs are next.",
         "Quick update\nThe build is green\nThe docs are next",
         "Quick update,\nThe build is green,\nThe docs are next",
         "Quick update:\nThe build is green\nThe docs are next"),
    case("H10", "newline", O, "neutral", "Pick one. New paragraph. The blue tiles, the grey tiles, or the white tiles.",
         "Pick one.\n\nThe blue tiles, the grey tiles, or the white tiles."),
    # --- spoken marks
    case("P01", "spoken_punct", P, "casual", "can you send me the file question mark",
         "Can you send me the file?"),
    case("P02", "spoken_punct", P, "casual", "We won the match exclamation point",
         "We won the match!"),
    case("P03", "spoken_punct", O, "neutral", "Bring water, comma, snacks, comma, and a jacket. Period.",
         "Bring water, snacks, and a jacket."),
    case("P04", "spoken_punct", O, "neutral", "note colon the office is closed on friday",
         "Note: the office is closed on Friday.", "Note: The office is closed on Friday."),
    case("P05", "spoken_punct", O, "neutral", "The plan dash if it works dash saves us a week.",
         "The plan — if it works — saves us a week.", "The plan - if it works - saves us a week."),
    case("P06", "spoken_punct", O, "neutral", "Lunch is ready period come down when you can period",
         "Lunch is ready. Come down when you can."),
    case("P07", "spoken_punct", W, "casual", "Did the tests pass question mark the logs look clean",
         "Did the tests pass? The logs look clean"),
    case("P08", "spoken_punct", O, "neutral", "I checked twice, semicolon, the numbers match.",
         "I checked twice; the numbers match."),
    case("P09", "spoken_punct", W, "casual", "Heads up comma the server restarts at noon",
         "Heads up, the server restarts at noon"),
    # --- lists with no line command (must still be lists)
    case("Q01", "list", O, "neutral", "a few things for the move boxes tape markers and labels",
         "A few things for the move:\n\n- Boxes\n- Tape\n- Markers\n- Labels"),
    case("Q02", "list", O, "neutral", "my plan for saturday first the gym second groceries third laundry",
         "My plan for Saturday:\n\n1. The gym\n2. Groceries\n3. Laundry"),
    case("Q03", "list", O, "neutral", "Reasons to switch, one, it's cheaper, two, it's faster, three, it's open source.",
         "Reasons to switch:\n\n1. It's cheaper\n2. It's faster\n3. It's open source"),
    case("Q04", "list", O, "neutral", "Here's a list for the store, rice, beans, lemons, and garlic.",
         "Here's a list for the store:\n\n- Rice\n- Beans\n- Lemons\n- Garlic"),
    case("Q05", "list", W, "casual", "bullet points for the retro the deploy was slow the alerts were noisy and the docs helped",
         "Bullet points for the retro:\n\n- The deploy was slow\n- The alerts were noisy\n- The docs helped",
         "For the retro:\n\n- The deploy was slow\n- The alerts were noisy\n- The docs helped"),
    case("Q06", "list", O, "neutral", "two things before friday number one renew the domain number two back up the photos",
         "Two things before Friday:\n\n1. Renew the domain\n2. Back up the photos"),
]


CASES2 = [
    # --- spoken line commands
    case("R01", "newline", W, "neutral",
         "the vendor sent the revised quote this morning new line i'll compare it with last year's numbers before lunch",
         "The vendor sent the revised quote this morning.\nI'll compare it with last year's numbers before lunch.",
         "The vendor sent the revised quote this morning\nI'll compare it with last year's numbers before lunch"),
    case("R02", "newline", P, "neutral", "Are you free on the twelfth? New paragraph. We're thinking of a small dinner at ours.",
         "Are you free on the 12th?\n\nWe're thinking of a small dinner at ours.",
         "Are you free on the twelfth?\n\nWe're thinking of a small dinner at ours."),
    case("R03", "newline", O, "neutral", "roses are red new line the sky is grey new line i forgot the umbrella new line again today",
         "Roses are red\nThe sky is grey\nI forgot the umbrella\nAgain today",
         "Roses are red,\nThe sky is grey,\nI forgot the umbrella\nAgain today"),
    case("R04", "newline", O, "neutral", "Flat 9, new line, 22 Mill Road, new line, Leeds.",
         "Flat 9\n22 Mill Road\nLeeds", "Flat 9,\n22 Mill Road,\nLeeds"),
    case("R05", "newline", W, "neutral", "see you all at the offsite new line cheers new line maya",
         "See you all at the offsite\nCheers\nMaya", "See you all at the offsite.\nCheers,\nMaya",
         "See you all at the offsite!\nCheers,\nMaya", "See you all at the offsite.\nCheers\nMaya"),
    case("R06", "newline", O, "neutral", "Summary. New paragraph. Sales rose in March. New paragraph. Costs stayed flat.",
         "Summary.\n\nSales rose in March.\n\nCosts stayed flat.", "Summary\n\nSales rose in March.\n\nCosts stayed flat.",
         "Summary:\n\nSales rose in March.\n\nCosts stayed flat."),
    case("R07", "newline", P, "casual", "running ten minutes late new line want me to grab coffee on the way",
         "Running 10 minutes late\nWant me to grab coffee on the way?",
         "Running 10 minutes late.\nWant me to grab coffee on the way?"),
    case("R08", "newline", O, "neutral", "step one comma unplug it new line step two comma wait a minute",
         "Step one, unplug it\nStep two, wait a minute", "Step one, unplug it.\nStep two, wait a minute",
         "Step 1, unplug it\nStep 2, wait a minute", "Step 1, unplug it.\nStep 2, wait a minute"),
    case("R09", "newline", O, "neutral", "Wi-Fi password, new line, blue harbor 77.",
         "Wi-Fi password\nblue harbor 77", "Wi-Fi password\nBlue harbor 77", "Wi-Fi password:\nBlue harbor 77",
         "Wi-Fi password:\nblue harbor 77"),
    case("R10", "newline", P, "neutral",
         "thanks for covering my shift yesterday new paragraph i owe you one so lunch is on me next week",
         "Thanks for covering my shift yesterday.\n\nI owe you one, so lunch is on me next week.",
         "Thanks for covering my shift yesterday.\n\nI owe you one so lunch is on me next week.",
         "Thanks for covering my shift yesterday!\n\nI owe you one, so lunch is on me next week."),
    # --- spoken marks
    case("T01", "spoken_punct", P, "neutral", "is the bakery open on sundays question mark hope so exclamation point",
         "Is the bakery open on Sundays? Hope so!"),
    case("T02", "spoken_punct", O, "neutral", "Two options, colon, train or bus.", "Two options: train or bus."),
    case("T03", "spoken_punct", O, "neutral", "the heater works again semicolon the landlord fixed it this morning",
         "The heater works again; the landlord fixed it this morning."),
    case("T04", "spoken_punct", O, "neutral", "Our neighbor, dash, the one with the boat, dash, is moving.",
         "Our neighbor — the one with the boat — is moving."),
    case("T05", "spoken_punct", W, "neutral",
         "after the meeting comma send the notes comma the slides comma and the recording",
         "After the meeting, send the notes, the slides, and the recording."),
    case("T06", "spoken_punct", O, "neutral", "The trial ends today. Period. Renew it if you want, period.",
         "The trial ends today. Renew it if you want."),
    case("T07", "spoken_punct", W, "casual", "looks good comma merging now exclamation point", "Looks good, merging now!"),
    case("T08", "spoken_punct", O, "neutral", "Can you call me back, question mark, it's about the lease.",
         "Can you call me back? It's about the lease."),
    case("T09", "spoken_punct", P, "neutral", "happy birthday exclamation mark", "Happy birthday!"),
    case("T10", "spoken_punct", O, "neutral", "put a period at the end of each line",
         "Put a period at the end of each line."),
    # --- lists with no line command (must still be lists), and one inline run
    case("U01", "list", O, "neutral", "three things for the trip one passports two chargers three snacks",
         "Three things for the trip:\n\n1. Passports\n2. Chargers\n3. Snacks"),
    case("U02", "list", O, "neutral", "a list of chores vacuum the hall water the plants and take out the bins",
         "A list of chores:\n\n- Vacuum the hall\n- Water the plants\n- Take out the bins"),
    case("U03", "list", O, "neutral", "Goals for the quarter. First, hire a designer. Second, ship the beta. Third, cut costs.",
         "Goals for the quarter:\n\n1. Hire a designer\n2. Ship the beta\n3. Cut costs"),
    case("U04", "list", P, "neutral", "we need eggs flour and butter for the cake tonight",
         "We need eggs, flour and butter for the cake tonight.", "We need eggs, flour, and butter for the cake tonight."),
]


def main():
    two = sys.argv[1:] == ["2"]
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases2.jsonl" if two else "cases.jsonl")
    cases = CASES2 if two else CASES
    with open(out, "w") as f:
        for c in cases:
            f.write(json.dumps(c) + "\n")
    print(f"wrote {len(cases)} cases to {out}")


if __name__ == "__main__":
    main()
