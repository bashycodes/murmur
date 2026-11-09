# Cleanup repeats the text already in the field (2026-11-07)

Decision: D71. Probe: `tools/probes/prototypes/context_echo_2026_11_07/`.
Every example here is made up and only keeps the shape of the report.

## Report

On the Windows PC (1.1.2), a text box already held a short label ending in a
colon, for example `Gate code: `. The owner dictated only the value
("four seven one nine"). The field then read `Gate code: Gate code: 4719`:
the label appeared twice.

## Evidence (the owner's history row and debug log, kept outside the repo)

| Stage | Holds the label? |
|---|---|
| Raw STT (`raw`) | no: only the value with "colon" spoken |
| Cleanup engine | `llm_engine: local` = this PC's Llama-3.2-3B Q4_K_M, prompt v2, level high, style casual; surrounding text allowed (on-device scope is FULL) |
| Final / pasted (`formatted`, `pasted`) | yes: label + value, the label's first letter lowercased |
| Insert | one Ctrl+V; no shell step adds text |

The lowercase first letter fits this account. Cursor matching
(`postprocess::context::match_context`, module removed, D82) saw text before the cursor ending in
`": "`, which counts as mid-sentence, and lowercased the first letter of the
answer. That first letter was the model's copy of the label.

## Root cause

The cleanup prompt's per-dictation message carries
`TEXT BEFORE CURSOR: «Gate code: »` right above `TRANSCRIPT: «…»`.
Neither prompt v2 nor v5 says what that line is for. No few-shot has a
context line either. The 3B treats the field text as part of what it should
write and answers `Gate code: 4719`. The shell pastes that after the
label already in the field.

**Why twice:** the model writes the label once and the field already holds
it once. Nothing in the app joins or deduplicates text. Since D59 no check
compares the answer with the transcript, so the copy went through.

## Reproduction

`cases.jsonl` has 26 made-up rows:
- E (18): context in the field, only a value or the rest of a sentence dictated.
- N (8): negatives, where the dictation itself repeats words of the context
  (`Gate code: ` + "gate code colon four seven one nine").

The rows ran through the real `FlowCore` commit path with the PC's model
(Llama-3.2-3B Q4_K_M) and a PC session (level high, casual style). A row
passes when:
- E: the final text does not start with the end of the context (or end with
  the start of the text after the cursor) unless the speaker said those words.
- N: the dictated phrase is kept.

| Variant | E no echo | N kept | Notes |
|---|---|---|---|
| base (integration) | 5/18 | 8/8 | echoes like the report: labels, greetings, sentence starts |
| A: system rule "TEXT BEFORE/AFTER CURSOR is already typed: output only the TRANSCRIPT" | 6/18 | 8/8 | |
| B: A + 2 few-shots with a context line | 7/18 | 8/8 | |
| C: B + a reminder line after the transcript | 16/18 | 8/8 | numbers left as words ("two hundred and six"), one wrong run ("4 791") |
| D: reminder line only, "reply with the TRANSCRIPT only" | 18/18 | 7/8 | dropped dictated words ("reminder for Thursday") |
| E: reminder + "write every word of the TRANSCRIPT" (with the check) | 18/18 | 8/8 | stops formatting numbers ("four seven one nine", "oranges two") |
| F: neutral "NOTE: … is not part of your reply" | 18/18 | 5/8 | drops dictated words that match the context, even half a sentence ("at noon") |
| **check only (shipped)** | **18/18** | **8/8** | prompt and formatting as on base |

The AI cleanup path (prompt v5 on the raw transcript, the runner's `cloud`
mode) with Qwen2.5-3B echoes even more:

| AI path, Qwen2.5-3B | E no echo | N kept |
|---|---|---|
| base | 1/18 (no timeouts: 17 of 18 answers echoed) | 8/8 |
| check | 18/18 (E01, E02 hit the 30 s timeout under load; the 16 answers all had the echo removed) | 8/8 |

The check also removes an echo inside a list the model made of the context:
"Totals so far: apples 3, pears 5. " plus "oranges two" became a 3-item list,
and the check left "Oranges: 2". It does not catch a reworded echo. With
" is the new door code." after the cursor, the 3B answered "The new door
code is 5521". Only verbatim runs are removed.

The prompt only changes how much the 3B copies of the field. The framing
that stops the copying also makes it drop the speaker's own words when they
match the field (D, F) or stop formatting (C, E). A check after the model
can't bring those words back. A check after the model can be kept
conservative: it removes only words the speaker plausibly did not say.

The first version of the check removed any run the transcript didn't
contain verbatim. Review found three ways that loses dictated words:
- a number the model formatted ("three p m" -> "3 PM" after "Meeting at
  3 pm. ");
- a filler inside the dictated phrase ("next, um, step");
- a name the model corrected from speech-to-text ("acne" -> the field's
  "Acme").

The shipped check counts those words as possibly said (below). Each case
has a regression test.

## Fix (D71)

`flow_core::cleanup::context_echo::strip`, run on every cleanup answer in
`run_metered` (on-device, flow-server and cloud / custom-server answers all
pass through it on the client) after the existing scrubbing, before the
empty check:

- Leading words of the answer that equal the last words of the text before
  the cursor are removed. Words compare case-insensitively, without edge
  punctuation. The check takes the longest such run.
- Trailing words that equal the first words of the text after the cursor
  are removed the same way.
- The field text compared is the window the prompt shows: the last 300
  characters before the cursor and the first 150 after it. It is what the
  prompt's scope allowed, so with surrounding text off for a cloud route
  nothing is compared.
- A run counts only when it is at least 2 words, or the whole window
  (`Password: `).
- **Conservative by design.** A wrong cut loses dictated words; a missed one
  only leaves the echo. So a run is removed only when fewer than half of
  its words could come from the text the model was given (the transcript,
  or the rules' text on the on-device path). A word counts as possibly given
  when that text has:
  - the word itself;
  - a near spelling of it (edit similarity >= 0.7 for 4+ letters: the model
    fixing "acne corp" to the field's "Acme Corp");
  - 2-3 consecutive words that join into it ("p m" -> "pm");
  - a longer word it abbreviates, when the answer writes it with a dot
    ("Dr." for "doctor");
  - or the word holds a digit, since the model writes spoken numbers as
    digits.

  Words are matched one by one, so fillers inside a dictated phrase ("next,
  um, step") don't hide it.
- Separators left at the cut go with it: `: ; , - – —`, the Hebrew maqaf,
  full-width `：；，`, straight and curly quotes, guillemets. Zero-width
  spaces count as spaces. An answer that would become empty is left as is.
- No word lists and no per-app rules. The prompts are unchanged, so the
  cached prompt prefix and every bench set without field context are
  untouched.

## Not fixed here

- In the reported row, digits were also lost when the model wrote a spoken
  port with "colon". The made-up rows show the same: "192.168.7.21 colon
  nine one zero zero" became "192.168.7.210". That's a separate number-run
  issue on the 3B path.
- Lowercasing after a colon in the field comes from cursor matching. It is
  correct for a continuing sentence, and the parallel casing work may
  revisit it.
