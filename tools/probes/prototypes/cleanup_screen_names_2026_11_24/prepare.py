#!/usr/bin/env python3
"""Build the bench cases for the cleanup screen-names probe (D108).

Every screen, name and sentence is invented: the screens are the English
ones of crates/murmur-core/fixtures/screen-names/chat-screens.json, and the
transcripts below are their `said` sentences as a speech model might
misspell the planted name (respaced, a vowel or consonant off). Two
variants per sentence:

* short  the screen as it is;
* full   the same screen at the bottom of a 2000-character capture, with
         the other screens' entries above it (what a long chat gives).

    prepare.py OUT.jsonl
"""

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[4]
FIXTURE = ROOT / "crates/murmur-core/fixtures/screen-names/chat-screens.json"
FULL_CHARS = 2000

# said sentence -> what the transcript says instead (the name misspelled).
HEARD = {
    "Tell Taviel Marrow that noon works fine for me.":
        "Tell Tavial Marrow that noon works fine for me.",
    "I will pick up Brisa Velloran on the way to the gym.":
        "I will pick up Brisa Valoran on the way to the gym.",
    "Corwen Ashgrove already paid the deposit for the cabin.":
        "Corwin Ashgrove already paid the deposit for the cabin.",
    "Can Ilsabet Varga bring the roof rack on Friday?":
        "Can Elsabet Varga bring the roof rack on Friday?",
    "Odessa Quillfeather is driving four of us up to the lake.":
        "Odessa Quill Feather is driving four of us up to the lake.",
    "Ask Dunstan Pryce to split the migration into two batches.":
        "Ask Dunston Price to split the migration into two batches.",
    "Kellan Fross will hold the release notes until Thursday.":
        "Kellan Frost will hold the release notes until Thursday.",
    "Marisol Okafor-Lind wants the staging build fixed today.":
        "Marisol Okafor Lind wants the staging build fixed today.",
    "Please forward the invoice from Wendeline Okoro to accounts.":
        "Please forward the invoice from Wendeline Akoro to accounts.",
    "I need to reply to Fenwick Adebayo-Sorel about the contract.":
        "I need to reply to Fenwick Adebayo Sorrel about the contract.",
    "Lunch on Sunday is at Severine's house this week.":
        "Lunch on Sunday is at Severin's house this week.",
    "Can you ask Bartholo about the blue casserole dish?":
        "Can you ask Bartolo about the blue casserole dish?",
    "Jorvik is grabbing the dish from the garage.":
        "Jorvic is grabbing the dish from the garage.",
    "Tell Peregrine Szabo that two o'clock is perfect.":
        "Tell Peregrin Sabo that two o'clock is perfect.",
    "Ansgar Teodorescu can keep the ladder until next week.":
        "Ansgar Teodoresco can keep the ladder until next week.",
    "Rosalind Achterberg is bringing the snacks on Friday.":
        "Rosalind Ackterberg is bringing the snacks on Friday.",
    "Delphine Marchetti-Rowe promised only two extra people.":
        "Delphine Marketti Rowe promised only two extra people.",
    "Quentavius says we have enough chairs.":
        "Quintavius says we have enough chairs.",
    "I would like to schedule a call with Ingrid Solvang-Pereira next week.":
        "I would like to schedule a call with Ingrid Solvang Pereira next week.",
    "Cassius Oyelaran asked about the schema registry setup.":
        "Cassius Oyelaren asked about the schema registry setup.",
}


def full_screen(screens, i):
    """Screen i at the bottom, other screens' entries above, oldest first,
    trimmed from the top to FULL_CHARS characters."""
    own = screens[i]["screen"]
    above = [e for j, s in enumerate(screens) if j != i for e in s["screen"]]
    out = list(own)
    n = sum(len(e) + 1 for e in out)
    for e in reversed(above):
        if n + len(e) + 1 > FULL_CHARS:
            break
        out.insert(0, e)
        n += len(e) + 1
    return out


def main():
    screens = [s for s in json.loads(FIXTURE.read_text())["screens"] if s.get("said")]
    rows = []
    for i, s in enumerate(screens):
        for k, said in enumerate(s["said"]):
            planted = [n for n in s["names"] if n in said]
            assert planted, said
            heard = HEARD[said]
            assert all(p not in heard for p in planted), heard
            for variant, screen in (("short", s["screen"]), ("full", full_screen(screens, i))):
                rows.append({
                    "id": f"{s['id']}-{k}",
                    "variant": variant,
                    "input": heard,
                    "said": said,
                    "planted": planted,
                    "screen": screen,
                })
    assert len(rows) == 2 * len(HEARD)
    with open(sys.argv[1], "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"{len(rows)} cases", file=sys.stderr)


if __name__ == "__main__":
    main()
