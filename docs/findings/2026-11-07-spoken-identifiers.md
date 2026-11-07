# 2026-11-07: spoken code names in the AI cleanup (D67)

**Question.** On the Wispr parity English set, every AI path missed both
identifier lines: an environment-variable name said as words with its form
("... in caps with underscores"), and an architecture name with a number
that the user has in their dictionary. Can a generic cleanup prompt change
(owner principle: no pre-AI rules, no lookup table, nothing named after the
eval's examples) write spoken code names in code form on the small local
models, without touching prose that uses the same words?

## Change

1. **Prompt rule** (v2 on-device and flow-server, v5 AI cleanup). A
   "Code names" rule: a name said with its form ("in caps", "with
   underscores", "underscore", "snake case", "camel case", "dash" / "dot"
   between its words) is written in that form without the format words; an
   environment variable or constant name is caps with underscores even with
   no form said; a technical name with a number takes its usual written form.
   The words in their ordinary sense stay. Its examples are invented
   (RETRY_LIMIT, parseDate, MP3).
2. **Few-shots.** v2 gets 7 invented shots (caps with underscores, snake
   case, spoken "underscore", camel case, an implicit env-var name, a
   spelled-out technical name with a number, and one prose negative that
   uses "snake", "dot" and "all caps"). v5 gets only the camel-case and
   env-var shots (`cleanup::CODE_NAME_SHOTS`): v5 sat ~650 tokens under the
   local engine's 4096-token context, and the 7-shot variant overflowed it
   (4088 tokens; every AI call failed with "prompt too long"). A test keeps
   v5's static prompt under 13 300 characters.
3. **Dictionary entries with digits** (`cleanup::terms`, amends D64). D64
   never named an entry with digits, because the transcript says the number
   as words. Selection now also matches against a copy of the transcript
   with runs of number words merged into one digit word ("velk forty two" ->
   "velk 42"; adjacent numbers join, "three twelve" -> "312"), used only for
   entries with digits; such an entry needs the same digits in the window
   ("velk fifty" does not name Velk42).
4. **Skip gate** (D27/D63). "underscore(s)", "in caps", "all caps" and the
   "... case" names wake the on-device model for a tidy line.

Every shot shares no 4-word run with the synthetic cases (the cue "in caps
with underscores" aside) or with the private parity references (checked
with the probe's `overlap.py` on the branch diff).

## Synthetic set

`tools/probes/prototypes/spoken_identifiers_2026_11_07/`: 38 invented
cases, I spoken forms (12), C names whose kind sets the form (6), V
technical names with a number (8), N prose with the same words (12); the
even-numbered half is held out (not looked at while tuning). Passes, base
-> after:

| path | total | held (19) | I | C | V | N |
|---|---|---|---|---|---|---|
| phone (v2, 1.5B, 3 s) | 21 -> 26 | 13 -> 12 | 4 -> 7 | 3 -> 4 | 3 -> 3 | 11 -> 12 |
| flow-server (v2, 1.5B) | 23 -> 28 | 14 -> 13 | 4 -> 7 | 3 -> 4 | 5 -> 5 | 11 -> 12 |
| AI cleanup (v5, 1.5B) | 20 -> 25 | 12 -> 12 | 3 -> 4 | 3 -> 5 | 2 -> 4 | 12 -> 12 |
| AI cleanup (v5, 3B) | 23 -> 25 | 12 -> 13 | 5 -> 8 | 0 -> 0 | 6 -> 6 | 12 -> 11 |

The gain is on the tuned half; the held half is flat. On the 1.5B paths
the model often keeps the words, writes the wrong form (camel case where
caps with underscores was said) or drops a word. The 3B never applies the implicit
env-var form (C 0/6) and turned a prose "snake case" into SnakeCase.

## Parity sets

English set (29 clips; dictionary in the session), base -> after:

| path | WER | terms | format |
|---|---|---|---|
| phone | 9.0 -> 8.3 % | 27 -> 28/37 | 11 -> 11/14 |
| flow-server | 9.3 -> 8.3 % | 27 -> 29/37 | 11 -> 11/14 |
| AI cleanup 1.5B | 8.7 -> 8.3 % | 29 -> 30/37 | 11 -> 11/14 |
| AI cleanup 3B | 6.6 -> 4.5 % | 30 -> 30/37 | 12 -> 13/14 |

The two identifier lines (2 lines x 3 ASR variants = 6) went 0/6 -> 3/6
(phone), 4/6 (flow-server), 4/6 (AI 1.5B), 3/6 (AI 3B). The architecture
name passes 3/3 on every path, and only through the digit-aware dictionary
naming (that change alone gave 3/6 on every path); without the dictionary
entry no path wrote it. The env-var name passed 1/3 on two paths: the ASR
damaged the word that marks it as a variable in two of the three variants,
and the 1.5B wrote camel case or lowercase. On the phone that line has no
spoken form cue, so the skip gate keeps the model off it.

Main set (13 clips): WER phone 10.2 -> 9.8 %, flow-server 10.5 -> 9.8 %,
AI 1.5B 12.2 -> 11.3 %, AI 3B 15.7 -> 14.8 %; format 18 -> 18 (v2) and
19 -> 20, 25 -> 26 (v5) of 31; terms unchanged.

**Regressions.** Spoken-numbers checks on the phone 67 -> 66/67 (a code
read digit by digit came out hyphenated). Lists and corrections on AI 1.5B
29 -> 28/39 (corrections 3 -> 2/9). 3B: one prose negative (above).

**Cost.** Prompt characters v2 6204 -> 8135 (~+530 tokens of cached
prefix), v5 12 355 -> 13 378 (~+280). LLM p50 on the synthetic set (loaded
machine, noisy): phone 1365 -> 1399 ms, flow-server 1368 -> 1394 ms, AI
1.5B 1623 -> 2054 ms, AI 3B 2981 -> 3064 ms.

## Limits

- Small models follow the rule unevenly; the held half did not improve.
  Spoken code names are a partial fix on 1.5B, not parity.
- A technical name with a number and no dictionary entry is rarely
  compacted; add it to the dictionary.
- On v2, the pre-AI rules already turn a spoken "dash" into " — ", so
  dash-joined names do not work there (the rules were left as they are).
- v5 has room for only two code-name shots under the local 4096-token
  context.
