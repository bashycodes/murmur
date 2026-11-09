# 2026-11-06: spoken numbers, times and money on the on-device path (D63)

**Question.** On the phone, the parity English set (30 lines) wrote
clock times, ordinals and digit runs as words on most lines (like "eleven
forty", "the twelfth"): format 6/14. Can the AI cleanup fix this without pre-AI number
rules (owner principle: text quality belongs in the prompt and the model)?

**Root cause.** The on-device cleanup did not run. The phone path is the
fallback rules, then prompt v2 on Qwen2.5 1.5B, but D27's "already clean"
skip (`cleanup::llm_can_help`) passed over every short, capitalised,
punctuated line. On the 30 lines, the model ran on 2. Number words were
left out of the skip's cue list on purpose (D27: the rules write numbers,
and the LLM once dropped "that" from "Don't forget that we have dinner at
eight"). The rules do write money and degrees ("two hundred and forty
dollars", "minus four degrees"), but not clock times ("eleven forty"),
ordinals ("the twelfth"), digit runs ("four eight one six") or a number
after a noun ("gate three").

**Answer.** Yes, on the 1.5B. Two changes, both English-only in effect:

1. **Skip gate.** `llm_can_help` now says the model can help when an English
   dictation still holds a number spoken as words
   (`postprocess::spoken::has_spoken_numbers`). This is a detector only: it
   decides whether the model runs and rewrites nothing. Idioms do not fire it:
   a lone "one" / "zero" counts only next to another number word or a unit
   ("one pm", "one percent"), and an ordinal counts only in a date ("May
   third", "the second of June", "on the twelfth" ending a phrase). So "one
   of those", "a second ago", "the first time", "the fourth floor", "for
   the fifth time" and "the third option" keep the skip. A lone "two" and
   up does fire it ("the two of us", "two minds"): there the model runs and,
   on the 1.5B, left all 5 such idioms as words (below), so it was not
   restricted further. Digits already written by the
   rules do not fire it. Non-English dictations keep the old gate.
2. **Prompt v2.** The digits rule now carries examples: times (eleven forty →
   11:40), dates (the fifth → the 5th), and a code read digit by digit as one
   run (four one seven two → 4172). There are also two new made-up few-shots:
   a digit-run code next to a count, and a time and a date next to "one of"
   and "a second", which stay words. No few-shot shares a 5-word run with any
   test case (`overlap.py`).

## Measurements

All runs use the real `MurmurCore` commit path (the parity runner, local mode
= phone pipeline, phone session: light, casual styles, 3 s budget) with
Qwen2.5-1.5B Q4_K_M on this shared 4-core box. "Before" is the same runner
built against integration `4839345`.

**Parity English set** (30 lines, Parakeet unified greedy as on the phone):

| | format | WER | CER | LLM ran |
|---|---|---|---|---|
| before | 6/14 | 8.3 % | 2.9 % | 2/30 |
| **after** | **12/14** | **8.0 %** | 3.6 % | 8/30 |

(Two runs of the final prompt: WER 7.6 % and 8.0 %, format 12/14 both
times; the table shows the later run.)

All 12 number and time checks now pass. The two left are not numbers:
an ASR mishearing of a word and an item list where the ASR split one item
in two. The number gains came step by step:

| variant | format |
|---|---|
| skip gate only, v2 unchanged | 11/14 |
| + digits-rule examples only | 11/14 |
| + 2 few-shots only | 11/14 |
| + both (shipped) | 12/14 |
| per-dictation "NUMBERS:" hint line in the user message instead | 10/14 |

The 1.5B wrote a five-digit number read digit by digit with dashes, in the
style of the phone-number few-shot, until the rule and the few-shots were
both in.
The hint-line variant was dropped. It needs about 70 extra uncached tokens
on every number dictation, it timed out once at 3 s, and it dropped "a
second ago" from a synthetic idiom case.

**Synthetic cases** (61, `tools/probes/prototypes/numbers_2026_11_06/`):

| | times /8 | money /4 | dates /6 | % /2 | ° /2 | counts /5 | digit runs /3 | idioms /27 | tidy /1 | all /58 | LLM ran |
|---|---|---|---|---|---|---|---|---|---|---|---|
| before | 4 | 3 | 2 | 2 | 2 | 4 | 1 | 20 | 1 | 39 | 2/61 |
| **after** | **8** | **4** | **6** | 2 | 2 | **5** | **3** | **27** | 1 | **58** | 27/61 |

All 27 idiom cases keep their idiom. On the 7 where the LLM now runs
because of a nearby time or date, the number is converted and the idiom
stays ("One of those vans came by at 10:40"). Before, those 7 failed only
on the number. Of the 10 later idioms, the 4 non-date ordinals ("the
fourth floor", "for the fifth time", "the third option", "finished
fourth") keep the skip; the 6 with a lone "two" / "three" ("the two of
us", "in two minds", "takes two to tango", "one or two days", "three's a
crowd") wake the model on 5, and it left every one as words.

**Hebrew.** The gate never fires on a non-English dictation (unit test, and
on 8 synthetic Hebrew lines the same 6 are cleaned and the same 2 are
skipped before and after). The system prompt and few-shots are the prefix
every language shares and the model caches, so a Hebrew dictation that is
cleaned now sees the two English number shots: 2 to 3 of the 6 cleaned
synthetic lines changed wording (two runs). A separate English-only prompt would have
cost Hebrew its prefix cache instead. **Pre-existing, not from this
change:** the 1.5B translates long Hebrew dictations into English on the v2
on-device path. That happened on 6 of 6 cleaned lines both before and after.
The language line in the user message does not stop it, and the guard lets
it through.

**Other format checks** (before → after, same model):

* Lists/corrections probe (39): lists 2 → 2–3/12, corrections 3 → 4/9,
  negatives 16 → 16/16. Two more lines are cleaned.
* 113-case AI-cleanup suite: 91 % → 90–91 % (two runs; run-ons 3 → 3–4/7).
  Quotes, parens, spoken marks, newline, ITN, casual, email and fillers are
  unchanged. Negatives go 18 → 17/19: "Don't forget that we have dinner at
  eight" is now cleaned, and the 1.5B drops "that" in both runs. This is the
  D27 example, and it is the one real regression. Reference closeness on the 14
  reference pairs goes 0.863 → 0.848. One pair hit the 3 s timeout, and others
  changed commas.
* Parity main set (13 clips): format 18 → 18/31, WER 10.5 → 10.0–10.2 %,
  instructions kept 3/3.

**Latency** (this box, 4 threads; the phone's 1.5B costs ~1.3 s p50 per
cleaned line on a 4-core CPU):

| set | lines cleaned | commit p50 / p95 / mean, all lines | LLM p50 / p95, cleaned lines |
|---|---|---|---|
| English set before | 2/30 (7 %) | 0 / 475 / 50 ms | 475 / 1032 ms |
| English set after | 8/30 (27 %) | 0 / 1905 / 380 ms | 1542 / 1938 ms |
| synthetic before | 2/61 | 0 / 0 / 38 ms | 1160 ms |
| synthetic after | 27/61 (44 %) | 2 / 1499 / 535 ms | 1161 / 1640 ms |
| suite before | 50/113 | mean 795 ms | 1645 / 3000 ms |
| suite after | 56/113 | mean 890 ms | 1699 / 3000 ms |

End to end on the English set (ASR + commit), p50 goes 453 → 479 ms and the
slowest line 1.7 → 2.4–2.5 s. A line with a spoken number now waits for the
model, about 1.3–1.5 s more. Every other line is unchanged. The prompt
prefix grows by about 520 characters (about 130 tokens), prefilled once
at model load.

## Not done / open

* "Don't forget that we have dinner at eight" loses "that" on the 1.5B (the
  D27 case): a model-size limit.
* Not measured: prompt v2 is also what murmur-server's cleanup sends (with
  whatever model the server runs) and what the Windows cloud-cleanup test
  button builds (`app.rs` `build_prompt`). Both get the new rule and the two
  new few-shots. Only the D27 skip gate is on-device-only (D43). The cloud /
  custom-server AI-cleanup path (prompt v5, D30) is unchanged.
* The label "v2" now names a changed prompt. Bench rows labelled v2 before
  this finding (2026-11-06) measured the pre-D63 v2.
* A multi-line dictation keeps the D27 skip (the 1.5B turns lines into
  bullets) even with spoken numbers.
* Hebrew dictation cleanup on the 1.5B translates to English (pre-existing;
  worth its own ticket: a script lock like Command Mode's would catch it).
* Measured on this box, not on the phone.
