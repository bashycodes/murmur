#!/usr/bin/env python3
"""Formatting-gap eval set (2026-10-24). Writes cases.jsonl next to this file.

Each case: input = a realistic Parakeet-style transcript (Parakeet TDT
punctuates and capitalises, often wraps a spoken command word in commas);
expected = what the user wants typed. `accept` lists other outputs a
reasonable user would also take (quote-punctuation placement, 4 vs four).

Style is implied by the app: chat apps (personal_chat / work_chat) expect
Wispr "casual" (sentence case, final period dropped, contractions OK),
email expects formal layout, "other" (notes/docs) expects formal punctuation.
App ids are Android packages the shell really sends; Android never sends
category "document", so docs apps go out as "other".
"""
import json
import pathlib

WA, SLACK, GMAIL, DOCS, KEEP = (
    "com.whatsapp",
    "com.Slack",
    "com.google.android.gm",
    "com.google.android.apps.docs.editors.docs",
    "com.google.android.keep",
)
CAT = {WA: "personal_chat", SLACK: "work_chat", GMAIL: "email", DOCS: "other", KEEP: "other"}
STYLE = {"personal_chat": "casual", "work_chat": "casual", "email": "formal", "other": "neutral"}

# (id, category, app, input, expected, [accept...])
C = [
    # --- quote / unquote ---------------------------------------------------
    ("q1", "quote", DOCS, "He called it, quote, unquote, innovative.", 'He called it "innovative."', ['He called it "innovative".']),
    ("q2", "quote", WA, "She said, quote, I'll be there soon, unquote, and then never showed up.", 'She said, "I\'ll be there soon," and then never showed up', ['She said "I\'ll be there soon" and then never showed up']),
    ("q3", "quote", GMAIL, "The client wrote, open quote, we need this by Friday, close quote, so let's prioritize it.", 'The client wrote, "We need this by Friday," so let\'s prioritize it.', ['The client wrote, "we need this by Friday," so let\'s prioritize it.', 'The client wrote "we need this by Friday" so let\'s prioritize it.']),
    ("q4", "quote", SLACK, "Can you change the button text to, quote, Get started, end quote?", 'Can you change the button text to "Get started"?', []),
    ("q5", "quote", KEEP, "Quote, the only way out is through, unquote. Robert Frost.", '"The only way out is through." Robert Frost.', ['"The only way out is through." Robert Frost']),
    ("q6", "quote", WA, "It was, quote unquote, fine.", 'It was "fine"', ['It was "fine."']),
    ("q7", "quote", DOCS, "The error message says, quote, file not found, unquote.", 'The error message says "file not found."', ['The error message says, "File not found."', 'The error message says "file not found".', 'The error message says, "file not found."']),
    ("q8", "quote", WA, "He literally just said, quote, whatever.", 'He literally just said "whatever"', ['He literally just said, "whatever"', 'He literally just said "whatever."']),
    # "quote unquote" scope (2026-10-31): one word mid-sentence, the whole sentence at its start.
    ("q9", "quote", KEEP, "quote unquote the new menu is a big improvement", '"The new menu is a big improvement."', ['"The new menu is a big improvement"']),
    ("q10", "quote", WA, "We tried the new place. Quote unquote, best tacos in the city. Not even close.", 'We tried the new place. "Best tacos in the city." Not even close', ['We tried the new place. "Best tacos in the city." Not even close.']),
    ("q11", "quote", WA, "he called the plan quote unquote genius and walked off", 'He called the plan "genius" and walked off', ['He called the plan "genius" and walked off.']),
    # --- parenthetical -----------------------------------------------------
    ("p1", "paren", KEEP, "I'll bring the charger in parentheses the USB-C one to the meeting.", "I'll bring the charger (the USB-C one) to the meeting.", []),
    ("p2", "paren", KEEP, "I'll bring the charger, in parentheses, the USB-C one, to the meeting.", "I'll bring the charger (the USB-C one) to the meeting.", []),
    ("p3", "paren", DOCS, "The deadline, open paren, which is flexible, close paren, is Friday.", "The deadline (which is flexible) is Friday.", []),
    ("p4", "paren", DOCS, "Add the new field, in brackets, optional, to the form.", "Add the new field [optional] to the form.", []),
    ("p5", "paren", WA, "Grab the blue folder, in parentheses, the one on my desk.", "Grab the blue folder (the one on my desk)", ["Grab the blue folder (the one on my desk)."]),
    ("p6", "paren", SLACK, "The new build, open parenthesis, version two point three, close parenthesis, is live.", "The new build (version 2.3) is live", ["The new build (version 2.3) is live."]),
    # Clause asides without pause commas (2026-11-01): close before the main verb.
    ("p7", "paren", KEEP, "my bike in parentheses which is the green one is locked outside the library", "My bike (which is the green one) is locked outside the library.", []),
    ("p8", "paren", DOCS, "The new intern in parentheses, who started on Monday, will shadow the support team.", "The new intern (who started on Monday) will shadow the support team.", []),
    # --- spoken punctuation ------------------------------------------------
    ("s1", "spoken_punct", KEEP, "Buy milk, comma, eggs, comma, and bread. Period.", "Buy milk, eggs, and bread.", []),
    ("s2", "spoken_punct", WA, "Are you coming tonight question mark", "Are you coming tonight?", []),
    ("s3", "spoken_punct", WA, "That's amazing exclamation point", "That's amazing!", []),
    ("s4", "spoken_punct", DOCS, "Agenda colon budget, hiring, and launch.", "Agenda: budget, hiring, and launch.", []),
    ("s5", "spoken_punct", DOCS, "The deadline dash which nobody likes dash is Friday.", "The deadline — which nobody likes — is Friday.", ["The deadline - which nobody likes - is Friday.", "The deadline, which nobody likes, is Friday."]),
    ("s6", "spoken_punct", GMAIL, "Thanks for the update period I'll review it tomorrow period", "Thanks for the update. I'll review it tomorrow.", []),
    ("s7", "spoken_punct", SLACK, "Quick question comma do we have a staging server question mark", "Quick question, do we have a staging server?", []),
    ("s8", "spoken_punct", DOCS, "We tested it twice semicolon both runs failed.", "We tested it twice; both runs failed.", []),
    ("s9", "spoken_punct", WA, "Did you see the game last night question mark it was insane exclamation point", "Did you see the game last night? It was insane!", []),
    # --- new line / paragraph ---------------------------------------------
    ("n1", "newline", KEEP, "Shopping list new line apples new line bananas new line coffee", "Shopping list\nApples\nBananas\nCoffee", ["Shopping list\napples\nbananas\ncoffee"]),
    ("n2", "newline", GMAIL, "Hi team, new paragraph, the release is moving to Thursday. New paragraph. Thanks, Dana.", "Hi team,\n\nThe release is moving to Thursday.\n\nThanks,\nDana", []),
    ("n3", "newline", DOCS, "First point is done. New line. Second point is pending.", "First point is done.\nSecond point is pending.", []),
    ("n4", "newline", KEEP, "Ideas for the weekend. New paragraph. Hiking or a movie.", "Ideas for the weekend.\n\nHiking or a movie.", []),
    ("n5", "newline", WA, "Hey, new line, are we still on for lunch?", "Hey\nAre we still on for lunch?", ["Hey,\nAre we still on for lunch?"]),
    # --- self-corrections --------------------------------------------------
    ("c1", "correction", WA, "Let's meet at the cafe at three, scratch that, at four.", "Let's meet at the cafe at 4", ["Let's meet at the cafe at four", "Let's meet at the cafe at 4."]),
    ("c2", "correction", GMAIL, "Send it to John, no wait, send it to Sarah.", "Send it to Sarah.", []),
    ("c3", "correction", SLACK, "The meeting is on Tuesday, actually I mean Wednesday.", "The meeting is on Wednesday", ["The meeting is on Wednesday."]),
    ("c4", "correction", DOCS, "We need three, sorry, four more chairs.", "We need four more chairs.", ["We need 4 more chairs."]),
    ("c5", "correction", WA, "Grab some milk. Never mind, I already bought some.", "I already bought some", ["Never mind, I already bought some"]),
    ("c6", "correction", KEEP, "Book flights for June tenth. Scratch that. Book flights for June twelfth.", "Book flights for June 12th.", ["Book flights for June 12."]),
    ("c7", "correction", SLACK, "Push the fix to main, I mean to the release branch.", "Push the fix to the release branch", ["Push the fix to the release branch."]),
    # --- numbers / dates / times / currency --------------------------------
    ("i1", "itn", KEEP, "The total is forty two dollars and fifty cents.", "The total is $42.50.", []),
    ("i2", "itn", WA, "Meet me at seven thirty tonight.", "Meet me at 7:30 tonight", ["Meet me at 7:30 tonight."]),
    ("i3", "itn", GMAIL, "The invoice is due on March fifth.", "The invoice is due on March 5th.", ["The invoice is due on March 5."]),
    ("i4", "itn", DOCS, "Revenue grew by twelve percent.", "Revenue grew by 12%.", []),
    ("i5", "itn", SLACK, "We have about two hundred and fifty users on the beta.", "We have about 250 users on the beta", ["We have about 250 users on the beta."]),
    ("i6", "itn", KEEP, "My number is five five five, one two three, four five six seven.", "My number is 555-123-4567.", []),
    ("i7", "itn", WA, "I'll be there in like ten minutes.", "I'll be there in 10 minutes", ["I'll be there in like 10 minutes"]),
    ("i8", "itn", DOCS, "The meeting is on the twenty first of October at two PM.", "The meeting is on October 21st at 2 pm.", ["The meeting is on October 21st at 2 PM.", "The meeting is on the 21st of October at 2 PM.", "The meeting is on October 21st at 2:00 PM."]),
    # Degrees and split "A M" (2026-11-01): "21°", lowercase "am" as Wispr writes it.
    ("i9", "itn", KEEP, "Set the thermostat to twenty one degrees before we get home.", "Set the thermostat to 21° before we get home.", []),
    ("i10", "itn", WA, "The bakery opens at 7 A M on Saturdays.", "The bakery opens at 7 am on Saturdays", ["The bakery opens at 7 am on Saturdays."]),
    # --- lists -------------------------------------------------------------
    ("l1", "list", KEEP, "For the trip I need to pack, first, my passport, second, chargers, and third, a rain jacket.", "For the trip I need to pack:\n\n1. My passport\n2. Chargers\n3. A rain jacket", []),
    ("l2", "list", DOCS, "Our priorities are one, hiring, two, onboarding, three, retention.", "Our priorities are:\n\n1. Hiring\n2. Onboarding\n3. Retention", []),
    ("l3", "list", GMAIL, "Hi Sam, a few things for tomorrow. First, book the room. Second, print the slides. Third, order lunch. Thanks, Alex.", "Hi Sam,\n\nA few things for tomorrow:\n\n1. Book the room\n2. Print the slides\n3. Order lunch\n\nThanks,\nAlex", ["Hi Sam,\n\nA few things for tomorrow.\n\n1. Book the room.\n2. Print the slides.\n3. Order lunch.\n\nThanks,\nAlex"]),
    ("l4", "list", KEEP, "Grocery list, eggs, milk, bread, and butter.", "Grocery list:\n\n- Eggs\n- Milk\n- Bread\n- Butter", ["Grocery list: eggs, milk, bread, and butter."]),
    ("l5", "list", SLACK, "Three things for the release, update the docs, fix the login bug, and tag the build.", "Three things for the release:\n\n1. Update the docs\n2. Fix the login bug\n3. Tag the build", ["Three things for the release: update the docs, fix the login bug, and tag the build"]),
    # --- casual chat -------------------------------------------------------
    ("k1", "casual", WA, "Hey, are you free tonight?", "Hey, are you free tonight?", []),
    ("k2", "casual", WA, "I'm running a bit late. I'll be there soon.", "I'm running a bit late. I'll be there soon", []),
    ("k3", "casual", SLACK, "Sounds good. Let's do it.", "Sounds good. Let's do it", []),
    ("k4", "casual", WA, "Haha, that's hilarious. I can't believe he did that.", "Haha that's hilarious. I can't believe he did that", ["Haha, that's hilarious. I can't believe he did that"]),
    ("k5", "casual", WA, "Okay, I will call you when I get home.", "Okay, I'll call you when I get home", ["Okay, I will call you when I get home"]),
    ("k6", "casual", SLACK, "Yeah, I do not think that is going to work.", "Yeah, I don't think that's going to work", ["Yeah, I do not think that is going to work"]),
    ("k7", "casual", WA, "Thank you so much. See you tomorrow.", "Thank you so much. See you tomorrow", []),
    # --- formal email ------------------------------------------------------
    ("e1", "email", GMAIL, "Hi Maria, I wanted to follow up on the proposal. Could you send me the final numbers by Friday? Best regards, Tom.", "Hi Maria,\n\nI wanted to follow up on the proposal. Could you send me the final numbers by Friday?\n\nBest regards,\nTom", []),
    ("e2", "email", GMAIL, "Dear Mr. Patel, thank you for your time yesterday. I've attached the contract for your review. Sincerely, Laura Chen.", "Dear Mr. Patel,\n\nThank you for your time yesterday. I've attached the contract for your review.\n\nSincerely,\nLaura Chen", []),
    ("e3", "email", GMAIL, "Hey John, just a quick note. The meeting moved to three PM. Thanks.", "Hey John,\n\nJust a quick note. The meeting moved to 3 pm.\n\nThanks", ["Hey John,\n\nJust a quick note. The meeting moved to 3 pm.\n\nThanks,", "Hey John,\n\nJust a quick note. The meeting moved to 3 pm.\n\nThanks.", "Hey John,\n\nJust a quick note. The meeting moved to 3 PM.\n\nThanks", "Hey John,\n\nJust a quick note. The meeting moved to 3 PM.\n\nThanks,", "Hey John,\n\nJust a quick note. The meeting moved to 3 PM.\n\nThanks."]),
    ("e4", "email", GMAIL, "Please find the report attached. Let me know if you have any questions.", "Please find the report attached. Let me know if you have any questions.", []),
    ("e5", "email", GMAIL, "Hi everyone, um, just a reminder that the office is closed on Monday. Thanks, Priya.", "Hi everyone,\n\nJust a reminder that the office is closed on Monday.\n\nThanks,\nPriya", []),
    # --- filler removal ----------------------------------------------------
    ("f1", "filler", WA, "Um, so I was thinking, uh, we could go to the beach.", "I was thinking we could go to the beach", ["So I was thinking we could go to the beach", "I was thinking, we could go to the beach"]),
    ("f2", "filler", SLACK, "So, like, the build is, you know, kind of broken right now.", "The build is kind of broken right now", ["So the build is kind of broken right now"]),
    ("f3", "filler", GMAIL, "Um, I just wanted to, uh, check in about the invoice.", "I just wanted to check in about the invoice.", []),
    ("f4", "filler", DOCS, "The results were, um, basically the same as last quarter.", "The results were basically the same as last quarter.", []),
    ("f5", "filler", WA, "I I think the the problem is with the router.", "I think the problem is with the router", []),
    ("f6", "filler", KEEP, "Uh, hmm.", "", []),
    # --- run-ons: comma / period placement ---------------------------------
    ("r1", "runon", KEEP, "so I talked to the landlord and he said the repair guy is coming tomorrow morning but he doesn't know what time so I might work from home", "I talked to the landlord and he said the repair guy is coming tomorrow morning, but he doesn't know what time, so I might work from home.", ["So I talked to the landlord and he said the repair guy is coming tomorrow morning, but he doesn't know what time, so I might work from home."]),
    ("r2", "runon", WA, "yeah that works for me what time should I come over", "Yeah, that works for me. What time should I come over?", []),
    ("r3", "runon", GMAIL, "I reviewed the draft it looks good overall but the second section needs more detail", "I reviewed the draft. It looks good overall, but the second section needs more detail.", []),
    ("r4", "runon", DOCS, "Before we start, however we need to agree on the scope.", "Before we start, however, we need to agree on the scope.", []),
    ("r5", "runon", SLACK, "can you take a look at the PR when you get a chance thanks", "Can you take a look at the PR when you get a chance? Thanks", ["Can you take a look at the PR when you get a chance? Thanks!"]),
    ("r6", "runon", WA, "ok cool see you at eight", "Ok cool, see you at 8", ["Okay cool, see you at 8", "Ok cool see you at 8", "OK cool, see you at 8"]),
    # Comma before "and" + a clause opening with a time phrase (2026-11-01).
    ("r7", "runon", KEEP, "The bins go out on Tuesday and at six the truck comes past.", "The bins go out on Tuesday, and at 6 the truck comes past.", ["The bins go out on Tuesday, and at six the truck comes past."]),
    # --- fallback-rule regressions (2026-10-26, ticket 6aded21e) -----------
    # Inputs: the owner's Wispr A/B script (../wispr_ab_2026_10_25/pairs.jsonl
    # ab5, ab7, ab8) and the 158 cm height from the AI-cleanup benchmark.
    ("ab8", "regression", KEEP, "okay first thing send the invoice second thing call the landlord and third thing please remind me about the dentist", "Okay:\n\n1. Send the invoice\n2. Call the landlord\n3. Please remind me about the dentist", ["1. Send the invoice\n2. Call the landlord\n3. Please remind me about the dentist", "- Send the invoice.\n- Call the landlord.\n- Please remind me about the dentist."]),
    ("ab5", "regression", KEEP, "he told me and I quote I'm never doing that again unquote and then he did it the next day", 'He told me, "I\'m never doing that again," and then he did it the next day.', []),
    # ab7 pins the currency spacing only: the bare "in parentheses not …"
    # aside and the implied "$60" are AI-cleanup work (out of the ticket).
    ("ab7", "regression", KEEP, "we spent about two hundred and fifty dollars on the trip in parentheses not counting gas which was another sixty", "We spent about $250 on the trip (not counting gas, which was another $60).", ["We spent about $250 on the trip in parentheses not counting gas which was another sixty."]),
    ("h1", "regression", KEEP, "he is about a hundred and fifty eight cm tall", "He is about 158 cm tall.", []),
    # --- negatives: command words used literally ---------------------------
    ("x1", "negative", DOCS, "The word quote comes from Latin.", "The word quote comes from Latin.", ['The word "quote" comes from Latin.']),
    ("x2", "negative", GMAIL, "Can you send me a quote for the new roof?", "Can you send me a quote for the new roof?", []),
    ("x3", "negative", DOCS, "The trial period ends next week.", "The trial period ends next week.", []),
    ("x4", "negative", DOCS, "In British English, a period is called a full stop.", "In British English, a period is called a full stop.", []),
    ("x5", "negative", DOCS, "Always use the Oxford comma in lists.", "Always use the Oxford comma in lists.", []),
    ("x6", "negative", WA, "Don't forget that we have dinner at eight.", "Don't forget that we have dinner at 8", ["Don't forget that we have dinner at eight"]),
    ("x7", "negative", DOCS, "Put a comma after the greeting.", "Put a comma after the greeting.", []),
    ("x8", "negative", SLACK, "There's no wait at the new place, let's go there.", "There's no wait at the new place, let's go there", []),
    ("x9", "negative", DOCS, "They launched a new line of sneakers.", "They launched a new line of sneakers.", []),
    ("x10", "negative", DOCS, "The question mark at the end looks wrong.", "The question mark at the end looks wrong.", []),
    ("x11", "negative", DOCS, "I mean it when I say this is our best quarter.", "I mean it when I say this is our best quarter.", []),
    ("x12", "negative", WA, "Quote me on that.", "Quote me on that", []),
    ("x13", "negative", DOCS, "The formula is in parentheses on the board.", "The formula is in parentheses on the board.", []),
    ("x14", "negative", DOCS, "Put the area code in parentheses.", "Put the area code in parentheses.", []),
    ("x15", "negative", DOCS, "The answer is in brackets at the bottom of the page.", "The answer is in brackets at the bottom of the page.", []),
    ("x16", "negative", SLACK, "Can you get me a quote by end of day?", "Can you get me a quote by end of day?", []),
    # Literal "in parentheses which" and "degrees" (2026-11-01).
    ("x17", "negative", DOCS, "Write the date in parentheses, which keeps the table tidy.", "Write the date in parentheses, which keeps the table tidy.", []),
    ("x18", "negative", DOCS, "The arm has six degrees of freedom.", "The arm has six degrees of freedom.", ["The arm has 6 degrees of freedom."]),
    ("x19", "negative", WA, "My brother has two degrees in history.", "My brother has two degrees in history", ["My brother has two degrees in history.", "My brother has 2 degrees in history"]),
]


def main() -> None:
    out = pathlib.Path(__file__).with_name("cases.jsonl")
    ids = set()
    with out.open("w") as f:
        for cid, cat, app, inp, exp, acc in C:
            assert cid not in ids, cid
            ids.add(cid)
            ac = CAT[app]
            f.write(json.dumps({
                "id": cid, "category": cat, "app_id": app, "app_category": ac,
                "style": STYLE[ac], "input": inp, "expected": exp, "accept": acc,
            }, ensure_ascii=False) + "\n")
    print(f"{len(C)} cases -> {out.name}")


if __name__ == "__main__":
    main()
