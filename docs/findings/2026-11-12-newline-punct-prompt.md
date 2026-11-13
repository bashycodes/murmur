# 2026-11-12: spoken "new line" / spoken marks under AI cleanup prompt v5 → v6

**Question.** D60 recorded a cost of prompt v5 (lists and self-corrections)
on the 3B model: spoken layout commands dropped (suite newline 3 → 1/5) and
spoken marks dropped (8 → 6/9). Can a prompt-only change bring "new line",
"new paragraph" and spoken marks back to v4c's level on the 3B and the 1.5B,
while keeping v5's list and correction gains? The owner principle rules out
pre-AI rewrite rules.

**Answer.** On the 3B, mostly. Prompt v6 (D89) brings "new line" back to
v4c's level and keeps v5's corrections, lists and suite pass. Spoken marks
stay where v5 had them, and on a fresh check set written after the prompt
was settled, both improve. On the 1.5B nothing moves "new line": it fails
under every prompt, v4c included (1/15 for v4c, v5 and v6). That is a model
limit. Its spoken marks equal v4c's (12/18). The cost is prompt size: v6 is
about 340 tokens longer than v5, which leaves little room for long dictations
on the 4096-token local engine (below).

v6 is a new prompt version. 1.3.0's v5 is unchanged and stays selectable
for bench and replay, like v4c. v6 differs from v5 in three ways:

1. **Layout shots after the list shots.** v5 put its list and correction
   shots right before the transcript, so the 3B read nearby "new line" items
   as a list. v4c's three layout shots (new paragraph, new line, spoken
   commas) now come after v5's list shots. Three made-up layout shots
   follow them. Their shapes match no suite case:
   * two sentences on two lines, with a code in digits (personal chat);
   * colon, semicolon and dash in one raw sentence (document);
   * the punctuated shape, with "exclamation point." as its own sentence,
     one "new line" and a ", comma," (work chat).
2. **Correction shots last.** v5's three self-correction shots ("I mean",
   "sorry", "no") are the last turns before the closing "only filler" turn.
   In the first pass six layout turns stood between them and the
   transcript, and the 3B started keeping "X, I mean Y".
3. **Reworded marks rule.**
   * "new line" is one line break and "new paragraph" is one blank line.
   * Line commands decide the layout and are never a list.
   * A spoken mark word never stays, even when written as its own sentence
     or between commas.
   * The list rule ends with the same "never a list" exception.

## Check sets

* **Suite:** the 113-case AI-cleanup suite. Variants were tuned on it.
* **Set 1** (`tools/probes/prototypes/newline_punct_2026_11_12/cases.jsonl`,
  25 made-up cases). It was written before any variant. Every variant was
  then scored on it and the winner was picked partly on it, so it is a
  second tuning set, not a held-out one.
* **Set 2** (`cases2.jsonl`, 24 made-up cases). It was written after the
  prompt was settled, with new shapes: longer lines, addresses, sign-offs,
  "step one / step two", marks in the punctuated shape, "exclamation mark",
  a mark word used as a word, and an inline run of items. Only v6 and 1.3.0's
  v5 were scored on it, once.

No few-shot shares a 5-word run with any case in the suite, set 1, set 2 or
the lists/corrections set (`variants2.py` checks this).

## Before / after

All runs use the parity runner's real `MurmurCore` commit path in cloud mode
with Qwen2.5 Q4_K_M and greedy decoding, on this shared box. The columns:

* **v4c**: re-run with the same binary, except the cells marked D60.
* **v5 (1.3.0)**: v5 as shipped (D60 plus the D64 dictionary rule).
* **X**: the first version of this fix (parts 1 to 3, but with made-up
  layout shots that copied suite shapes). Withdrawn after review.
* **v6**: X with those three shots replaced.

**3B (stand-in for the cloud model)**

| set | check | v4c | v5 (1.3.0) | X | **v6** |
|---|---|---|---|---|---|
| suite | newline /5 | 3 | 1 | 3 | 2 |
| set 1 | newline /10 | 4 | 4 | 5 | 5 |
| suite + set 1 | newline /15 | 7 | 5 | 8 | **7** |
| set 2 | newline /10 | – | 4 | – | **5** |
| suite | spoken marks /9 | 8 | 8 | 9 | 8 |
| set 1 | spoken marks /9 | 6 | 7 | 7 | 7 |
| suite + set 1 | spoken marks /18 | 14 | 15 | 16 | **15** |
| set 2 | spoken marks /10 | – | 6 | – | **8** |
| suite | corrections /7 | 2 | 6 | 6 | **6** |
| suite | lists /5 | 4 | 4 | 4 | 4 |
| set 1 | list /6 | 2 | 5 | 6 | 5 |
| set 2 | list /4 | – | 3 | – | 4 |
| suite | quote /11 | 8 | 7 | 6 | 7 |
| suite | run-on /7 | 5 | 5 | 5 | 5 |
| suite | pass (99 scored) | 72 % | 74 % | 75 % | **75 %** |
| parity main | format /31 | 23 (D60) | 25 | 25 | **25** |
| parity main | WER | 13.9 % (D60) | 15.7 % | 15.7 % | 14.4 % |
| lists/corrections (39) | all | 31 (D60) | 32 | 34 | **33** |
| lists/corrections | lists /12 · corrections /9 · negatives /16 | 9 · 7 · 15 | 10 · 8 · 14 | 10 · 8 · 16 | 10 · 7 · 16 |
| suite | LLM p50 | 2.8 s | 3.1 s | 3.3 s | 3.7 s |

With shapes that do not mirror the suite, X's extra case on suite newline
and suite spoken marks went away. v6 meets the bar (newline ≥ 7/15, main
≥ 25/31, corrections 6/7, pass ≥ 74 %), and on set 2 it beats v5 on all
three groups. Side effects on the 3B:
* parity main WER is 14.4 %, above v4c's 13.9 % (D60) and below v5's 15.7 %;
* suite quotes are 7/11 under v5 and v6 (X had 6), against 8/11 under v4c;
* the synthetic set loses one correction case against X.

**1.5B**

| set | check | v4c | v5 (1.3.0) | X | **v6** |
|---|---|---|---|---|---|
| suite | newline /5 | 1 | 1 | 0 | 1 |
| set 1 | newline /10 | 0 | 0 | 0 | 0 |
| suite + set 1 | newline /15 | 1 | 1 | 0 | **1** |
| set 2 | newline /10 | – | 1 | – | 1 |
| suite | spoken marks /9 | 7 | 7 | 6 | 7 |
| set 1 | spoken marks /9 | 5 | 5 | 6 | 5 |
| suite + set 1 | spoken marks /18 | 12 | 12 | 12 | **12** |
| set 2 | spoken marks /10 | – | 3 | – | 4 |
| suite | corrections /7 | 3 | 4 | 4 | 5 |
| suite | pass | 72 % | 67 % | 66 % | **72 %** |
| parity main | format /31 | 18 (D60) | 19 | 21 | **21** |
| parity main | WER | 11.3 % (D60) | 12.2 % | 11.1 % | 10.7 % |
| lists/corrections | all /39 | 22 (D60) | 28 | 27 | 28 |

The 1.5B gets "new line" right in 1 of 15 cases under v4c, v5 and v6 alike.
It writes "new line" items as a bulleted list, or a single "new line" as a
blank line. No prompt we tried changed that: a model limit. Its spoken marks
are 12/18 under every prompt, so v6 is not below v4c. v6 brings its suite
pass back to v4c's 72 % and keeps v5's main-set gain.

## Prompt size on the local engine

The local engine (murmur-llm, also behind "Your server") has a 4096-token
context. It refuses a prompt within 64 tokens of it. It caps the answer at
the context left after the prompt. `budget_probe.py` measures the prompt in
real tokens: two over-long cases are refused before any prefill, and the
refusal names the token count. The worst-case user message carries the
casual STYLE line, 20 DICTIONARY terms (the most a prompt carries), and the
full 300/150-character windows around the cursor.

| prompt | longest prompt, dictation aside | left for the dictation and its answer |
|---|---|---|
| v5 (1.3.0) | ~3.58k tokens (13.1k chars) | ~510 tokens |
| v6 | ~3.91k tokens (14.4k chars, 3.68 chars a token) | ~170 tokens |

The dictation and its cleaned text are about the same length, so in this
worst case v6 handles a dictation of about 85 tokens (some 60 words) before
the answer is cut short. v5 handled about 250. Without dictionary terms and
cursor text the prompt is about 215 tokens shorter, which leaves room for
about 190 tokens of dictation. The test `v6_longest_prompt_stays_at_its_measured_size`
renders this worst case for every style and pins it at 14.45k characters.
Any growth needs a new measurement.

We tried two trims of v6 to win back room. Both broke something, so neither
shipped:
* **v5's rule wording** (the shot changes only, ~115 tokens less): suite
  corrections 6 → 4/7 and pass 75 → 71 %.
* **Without v4c's two counted-list shots** (~165 tokens less): suite lists
  4 → 1/5 and pass 69 %.

Neither cut is free, so the room for long dictations on the local engine is
a decision for the owner. The options are to accept it, to raise the
engine's context, or to drop the dictionary/cursor lines when a dictation
is long.

## What we tried (3B, all prompt-only)

`tools/probes/prototypes/newline_punct_2026_11_12/variants.py` (first pass)
and `variants2.py` (second pass) build the variants from a dump of v5.
Their docstrings explain each letter.

| variant | suite newline /5 | suite marks /9 | suite corrections /7 | suite pass | set-1 newline /10 | set-1 marks /9 | set-1 list /6 | main /31 |
|---|---|---|---|---|---|---|---|---|
| v4c | 3 | 8 | 2 | 72 % | 4 | 6 | 2 | 23 (D60) |
| v5 (1.3.0) | 1 | 8 | 6 | 74 % | 4 | 7 | 5 | 25 |
| a+b+c: marks rule first, no blank line before lists, reworded rules | 2 | 7 | 6 | 72 % | 4 | 7 | 6 | |
| a+b+c+d | 2 | 7 | 4 | 70 % | 4 | 6 | 6 | |
| b+i+e: list rule opens with "not when laid out by new line" | 2 | 8 | 5 | 73 % | 2 | 7 | 6 | |
| b+L+e: one ordered "Layout" rule | 2 | 8 | 5 | 72 % | 2 | 7 | 6 | |
| b+i+e+s: bake-sale shot announces its list | 2 | 9 | 5 | 74 % | 1 | 7 | 6 | |
| U: a reminder line appended to the user message | 1 | 5 | – | 48 % | – | – | – | |
| e: v4c layout shots last | 2 | 8 | 5 | 73 % | 2 | 7 | 5 | 25 |
| d: three made-up layout shots | 2 | 7 | 4 | 69 % | 3 | 7 | 6 | |
| d+e | 2 | 8 | 5 | 70 % | 5 | 7 | 6 | 24 |
| c+d+e (first pass) | 2 | 8 | 4 | 69 % | 5 | 7 | 6 | 25 |
| c+d+e, A: correction and layout shots alternate | 3 | 8 | 6 | 74 % | 4 | 6 | 5 | |
| c+d+e, X: correction shots last | 3 | 9 | 6 | 75 % | 5 | 7 | 6 | 25 |
| **X, N: new shapes for the made-up shots (v6)** | **2** | **8** | **6** | **75 %** | **5** | **7** | **5** | **25** |
| N, Nr: v5's rule wording | 2 | 8 | 4 | 71 % | | | | |
| N, Nt: without v4c's two counted-list shots | 2 | 9 | 5 | 69 % (lists 1/5) | | | | |

What we learned:

* **Shot order matters more than rule wording.** Rewording the marks and list
  rules, putting the marks rule first, or replacing both with one ordered
  "Layout" rule changed suite newline by at most one case. Only the layout
  shots placed after the list shots, together with made-up ones (d+e),
  moved set 1. Either half alone made set-1 newline worse. Yet taking the
  rewording back out of the final prompt (Nr) cost corrections, so once
  the shots are in place the wording holds something up.
* **Whatever kind of shot comes last is what the 3B copies.** With lists
  last it made lists of "new line" items. With layout last it dropped
  corrections ("X, I mean Y" kept). With corrections last and lists kept
  away from the transcript, both hold.
* **Made-up shots that copy suite shapes inflate the suite score.** X's
  shots mirrored suite cases n1, n4, s1 and s7/s9. With genuinely different
  shapes (v6), suite newline and marks each lost the case X had gained.
  Set 2, written later, still shows v6 ahead of v5.
* **Dropping the blank line before lists (b) hurts set-1 newline**
  (4 → 1–2/10). The model then writes "new line" items as a tight bulleted
  list more often.
* **A reminder in the user message (U) breaks other things.** Suite pass
  fell from 74 to 48 %, and numbers and lists broke. The fix therefore stays
  in the static prompt.
* **On set 1, v4c is no better than v5 at "new line"** (4/10 each). The
  suite's 3 → 1 drop came partly from those five cases. Under every prompt,
  v4c included, n1 ("... list new line ...") becomes a list.

## Known limits

* The 3B still writes some "new line" runs as a bulleted list when the
  first line reads like a heading ("things to bring", "quick update"). It
  also writes some single "new line"s as a blank line. The 1.5B does both
  nearly every time. This is a limit of model size, not a missing rule.
* Moving shots shifts unrelated greedy outputs by a case or two (quotes,
  ordinal dates). These flips are scattered and follow no single pattern.
* The prompt is about 340 tokens longer than v5 (see above), and the 3B
  suite's LLM p50 goes up by about 0.5 s.
