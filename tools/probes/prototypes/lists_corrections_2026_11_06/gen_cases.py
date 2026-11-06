#!/usr/bin/env python3
"""Synthetic spoken-list and self-correction cases (2026-11-06 probe).

Writes `cases.jsonl` (runner input: id, input, app_category) and
`refs.json` (the wispr_parity `score.py` reference shape: intended text,
regex checks) next to this script. Every sentence is made up; none comes
from a real dictation. Inputs are lowercase and unpunctuated (whisper-less
raw speech) or Parakeet-style (pause commas), as dictation arrives.

Groups (id prefix):
  L  spoken lists: counted (first/second/third, one/two/three, number one),
     announced ("bullet points", "a few things", "make a list", "list:")
  C  self-corrections: only the correction stays
  X  a list and a correction in one dictation
  N  negatives: no list and nothing dropped ("no", "not", "first",
     "sorry", "actually", items named inside a sentence that goes on)
"""
import json
import os

NO_LIST = {"name": "no list", "re": r"(?m)^\s*(\d+[.)]|[-•*])\s", "absent": True}


def numbered(*items):
    pat = r"(?m)" + r".*\n".join(f"^{i + 1}[.)] ?{w}" for i, w in enumerate(items))
    return {"name": "numbered list", "re": pat, "flags": "i"}


def bullets(*items):
    pat = r"(?m)" + r".*\n".join(f"^[-•*] ?{w}" for w in items)
    return {"name": "bulleted list", "re": pat, "flags": "i"}


def intro(words):
    return {"name": "lead-in kept", "re": r"^\s*" + words, "flags": "i"}


def has(name, pat):
    return {"name": name, "re": pat, "flags": "i"}


def lacks(name, pat):
    return {"name": name, "re": pat, "flags": "i", "absent": True}


# (id, category, input, intended text, checks)
CASES = [
    # --- lists ---------------------------------------------------------
    ("L01", "other",
     "for the launch we still need to first update the docs second email the beta testers and third tag the release",
     "For the launch we still need to:\n\n1. Update the docs\n2. Email the beta testers\n3. Tag the release",
     [numbered("update the docs", "email the beta", "tag the release"), intro("for the launch")]),
    ("L02", "other",
     "the steps are one preheat the oven two grease the pan three pour in the batter",
     "The steps are:\n\n1. Preheat the oven\n2. Grease the pan\n3. Pour in the batter",
     [numbered("preheat", "grease", "pour"), intro("the steps")]),
    ("L03", "document",
     "Agenda for tomorrow, bullet points, budget review, hiring plan, office move.",
     "Agenda for tomorrow:\n\n- Budget review\n- Hiring plan\n- Office move",
     [bullets("budget review", "hiring plan", "office move"), intro("agenda for tomorrow"),
      lacks("no literal 'bullet points'", r"bullet points?")]),
    ("L04", "other",
     "a few things for the weekend clean the gutters fix the gate and call the plumber",
     "A few things for the weekend:\n\n- Clean the gutters\n- Fix the gate\n- Call the plumber",
     [bullets("clean the gutters", "fix the gate", "call the plumber"), intro("a few things")]),
    ("L05", "other",
     "Make a list of what to pack, sunscreen, towels, a paperback, and snacks.",
     "What to pack:\n\n- Sunscreen\n- Towels\n- A paperback\n- Snacks",
     [bullets("sunscreen", "towels", "a paperback", "snacks")]),
    ("L06", "personal_chat",
     "two reasons I'm leaving early number one the traffic number two my cousin is visiting",
     "Two reasons I'm leaving early:\n\n1. The traffic\n2. My cousin is visiting",
     [numbered("the traffic", "my cousin"), intro("two reasons")]),
    ("L07", "other",
     "Things to do today. First, renew the passport. Second, book the dentist. Third, pay the water bill.",
     "Things to do today:\n\n1. Renew the passport\n2. Book the dentist\n3. Pay the water bill",
     [numbered("renew the passport", "book the dentist", "pay the water bill"), intro("things to do today")]),
    ("L08", "work_chat",
     "my concerns are firstly the cost secondly the timeline and thirdly the staffing",
     "My concerns are:\n\n1. The cost\n2. The timeline\n3. The staffing",
     [numbered("the cost", "the timeline", "the staffing"), intro("my concerns")]),
    ("L09", "other",
     "shopping list bullet point milk bullet point eggs bullet point spinach",
     "Shopping list:\n\n- Milk\n- Eggs\n- Spinach",
     [bullets("milk", "eggs", "spinach"), intro("shopping list"), lacks("no literal 'bullet point'", r"bullet point")]),
    ("L10", "other",
     "Here's a list for the party, cups, plates, napkins, and ice.",
     "Here's a list for the party:\n\n- Cups\n- Plates\n- Napkins\n- Ice",
     [bullets("cups", "plates", "napkins", "ice"), intro("here's a list")]),
    ("L11", "document",
     "the plan for monday is first we meet the client at nine second we review the contract together and third we sign if it all looks good",
     "The plan for Monday is:\n\n1. We meet the client at 9\n2. We review the contract together\n3. We sign if it all looks good",
     [numbered("(we )?meet the client", "(we )?review the contract", "(we )?sign"), intro("the plan for monday")]),
    ("L12", "other",
     "I need to remember these points the gym closes early on sundays the pool is shut for repairs and the locker code changed",
     "I need to remember these points:\n\n- The gym closes early on Sundays\n- The pool is shut for repairs\n- The locker code changed",
     [bullets("the gym closes", "the pool is shut", "the locker code"), intro("i need to remember")]),
    # --- self-corrections ---------------------------------------------
    ("C01", "other", "buy two, no, three apples",
     "Buy 3 apples", [has("three kept", r"\b(3|three) apples"), lacks("two dropped", r"\b(2|two)\b")]),
    ("C02", "work_chat", "let's meet at three sorry four",
     "Let's meet at 4", [has("4 kept", r"\b(4|four)\b"), lacks("3 dropped", r"\b(3|three)\b")]),
    ("C03", "other", "the call is on tuesday no wait wednesday",
     "The call is on Wednesday", [has("wednesday kept", r"wednesday"), lacks("tuesday dropped", r"tuesday")]),
    ("C04", "work_chat", "Send the draft to Mark, scratch that, send it to Priya.",
     "Send the draft to Priya", [has("priya kept", r"priya"), lacks("mark dropped", r"\bmark\b")]),
    ("C05", "other", "the budget is five hundred I mean six hundred dollars",
     "The budget is $600", [has("600 kept", r"600|six hundred"), lacks("500 dropped", r"500|five hundred")]),
    ("C06", "personal_chat", "we need four chairs actually make that five",
     "We need 5 chairs", [has("5 kept", r"\b(5|five)\b"), lacks("4 dropped", r"\b(4|four)\b")]),
    ("C07", "other", "Book a table for six, no, eight people at the noodle place.",
     "Book a table for 8 people at the noodle place",
     [has("8 kept", r"\b(8|eight)\b"), lacks("6 dropped", r"\b(6|six)\b")]),
    ("C08", "personal_chat", "turn left at the bank sorry at the bakery",
     "Turn left at the bakery", [has("bakery kept", r"bakery"), lacks("bank dropped", r"\bbank\b")]),
    ("C09", "other", "the flight leaves at seven forty five no wait eight fifteen",
     "The flight leaves at 8:15", [has("8:15 kept", r"8:15"), lacks("7:45 dropped", r"7:45|seven forty")]),
    # --- list + correction ---------------------------------------------
    ("X01", "other",
     "a few things to grab at the store eggs whole milk no wait oat milk bread and coffee",
     "A few things to grab at the store:\n\n- Eggs\n- Oat milk\n- Bread\n- Coffee",
     [bullets("eggs", "oat milk", "bread", "coffee"), lacks("whole milk dropped", r"whole milk")]),
    ("X02", "document",
     "the order is first the salad second the soup sorry the pasta and third dessert",
     "The order is:\n\n1. The salad\n2. The pasta\n3. Dessert",
     [numbered("the salad", "the pasta", "dessert"), lacks("soup dropped", r"soup")]),
    # --- negatives ------------------------------------------------------
    ("N01", "personal_chat", "no I don't think we should go out tonight",
     "No, I don't think we should go out tonight", [NO_LIST, has("'no' kept", r"^\s*no\b")]),
    ("N02", "other", "there are two not three reasons this design works",
     "There are two, not three, reasons this design works",
     [NO_LIST, has("contrast kept", r"not (3|three)"), has("two kept", r"\b(2|two)\b")]),
    ("N03", "personal_chat", "the first time I tried sushi I hated it",
     "The first time I tried sushi I hated it", [NO_LIST, has("'first' kept", r"first time")]),
    ("N04", "other", "first we'll grab lunch and then we can head to the museum",
     "First we'll grab lunch and then we can head to the museum", [NO_LIST, has("lunch kept", r"lunch")]),
    ("N05", "personal_chat", "I packed socks shirts and a jacket so I should be fine",
     "I packed socks, shirts and a jacket so I should be fine",
     [NO_LIST, has("items inline", r"socks, shirts,? and a jacket")]),
    ("N06", "work_chat", "sorry I missed your call I was in a meeting",
     "Sorry I missed your call, I was in a meeting", [NO_LIST, has("'sorry' kept", r"^\s*sorry")]),
    ("N07", "personal_chat", "the movie was actually pretty good",
     "The movie was actually pretty good", [NO_LIST, has("'actually' kept", r"actually")]),
    ("N08", "personal_chat", "I mean it when I say thank you for everything",
     "I mean it when I say thank you for everything", [NO_LIST, has("'I mean it' kept", r"i mean it")]),
    ("N09", "work_chat", "no problem see you at five",
     "No problem, see you at 5", [NO_LIST, has("'no problem' kept", r"no problem")]),
    ("N10", "other", "she counted one two three and then jumped into the lake",
     "She counted one, two, three and then jumped into the lake",
     [NO_LIST, has("count kept inline", r"counted (one|1),? (two|2),? (three|3)")]),
    ("N11", "personal_chat", "never mind the rain we're going anyway",
     "Never mind the rain, we're going anyway", [NO_LIST, has("rain kept", r"rain")]),
    ("N12", "work_chat", "give me a second to find the file",
     "Give me a second to find the file", [NO_LIST, has("'second' kept", r"a second")]),
    ("N13", "other",
     "we talked about the budget the hiring plan and the office move before lunch and then went back to work",
     "We talked about the budget, the hiring plan and the office move before lunch and then went back to work",
     [NO_LIST, has("items inline", r"budget, the hiring plan,? and the office move")]),
    ("N14", "work_chat", "it's due on friday not monday",
     "It's due on Friday, not Monday", [NO_LIST, has("contrast kept", r"not monday")]),
    ("N15", "personal_chat", "she's my number one fan",
     "She's my number one fan", [NO_LIST, has("'number one' kept", r"number (one|1) fan")]),
    ("N16", "other",
     "I can bring things like plates cups and napkins and anything else you need for the party",
     "I can bring things like plates, cups and napkins and anything else you need for the party",
     [NO_LIST, has("items inline", r"plates, cups,? and napkins")]),
]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(here, "cases.jsonl"), "w") as f:
        for cid, cat, inp, _, _ in CASES:
            f.write(json.dumps({"id": cid, "input": inp, "app_category": cat}) + "\n")
    refs = {"_note": "Synthetic cases (gen_cases.py); made-up sentences only.",
            "clips": {cid: {"text": text, "terms": [], "checks": checks, "instruction": False}
                      for cid, _, _, text, checks in CASES}}
    with open(os.path.join(here, "refs.json"), "w") as f:
        json.dump(refs, f, indent=1)
        f.write("\n")
    print(len(CASES), "cases")


if __name__ == "__main__":
    main()
