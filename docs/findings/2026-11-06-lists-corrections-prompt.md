# 2026-11-06: spoken lists and "X, no, Y" self-corrections in the AI cleanup prompt (v5)

**Question.** The Wispr parity eval (2026-11-05) lost format points on dictated
lists (no bullets or numbers, lead-in dropped) and on spoken self-corrections
(both versions kept). Can the AI-cleanup prompt alone fix this on the ≤3B
models we run, with no pre-AI rules (owner principle: text quality belongs in
the prompt and the model)?

**Answer.**

* **3B: yes, partly.** Prompt v5 lifts the parity main set from 23 to 25/31
  format on Parakeet greedy (beam + dictionary 22 → 23/31), with instructions
  kept 3/3, on the list and correction clips. WER moves 13.9 → 14.4 % (greedy),
  13.7 → 14.4 % (beam), almost all of it from one clip where v5 keeps a spoken
  clause that the Wispr reference drops (v4c dropped it too). That is a WER cost
  against the reference, not a word lost from what was said.
* **1.5B: lists yes, corrections no.** On the synthetic cases, lists go
  6 → 11/12. Corrections stay at 1–3/9 for every prompt we tried, including a
  short prompt with only these two rules and their examples. The 1.5B copies
  both versions of a correction. More examples or rewording did not change that,
  so we stop there and do not add a pre-AI rule. The phone's cleanup runs the
  1.5B on the v2 on-device path, which this change does not touch (see below).
* **No new false lists.** On the 16 negative cases ("no problem", "first time",
  "not Monday", items named inside a running sentence) the 1.5B stays at 15/16.
  The 3B drops to 14/16 from 15/16: once, it removed a contrast ("due Friday,
  not Monday") as if it were a correction. On the main set no clip became a
  list.

## What changed (prompt v5, `crates/flow-core/src/cleanup/prompt_v5.rs`)

v4c with:

1. **Rewritten correction rule.** A correction word (no, no wait, sorry, I mean,
   actually, scratch that, make that) followed by a replacement keeps only the
   replacement. "No", "not", "sorry" and "actually" used in their normal sense
   stay.
2. **Rewritten list rule.** Counted items (first/second, one/two, number one) or
   an announced list ("a list", "bullet points", "a few things") of 3 or more
   items become the lead-in, a colon, a blank line, then `<ol>` for counted
   items or `<ul>` for announced ones. The lead-in words stay, and the counting
   words and "bullet point" go. Items inside a sentence that goes on stay inline.
3. **Both rules moved to the top of the rule list.** This was the best variant
   on both model sizes.
4. **8 new made-up few-shots**:
   * an announced list and a counted list;
   * a correction inside a list;
   * three comma-style corrections and one "I mean" correction;
   * two negatives: an inline item run, and "not X" used as a contrast.

None of the examples shares a 5-word run with a test case; the probe checks
this.

The D30 AI-cleanup path (cloud and custom server, raw transcript) now sends
v5. v4c stays selectable in flow-replay and the bench. The on-device and
flow-server paths keep prompt v2 on the rules-processed text, so the phone's
1.5B is unchanged.

Also fixed on the way (`unwrap_output_for`): a dictated lead-in such as
"Here's a list for the party:" was stripped as a model preamble. The preamble
strip is now skipped when the transcript itself starts with those words.

## Measurements

All runs use the real `FlowCore` commit path (parity runner) with Qwen2.5
Q4_K_M on this shared box. v4c-r is v4c re-run with the same binary, so the
two sides differ only in the prompt.

**Synthetic cases** (39, `tools/probes/prototypes/lists_corrections_2026_11_06/`):

| prompt | model | lists /12 | corrections /9 | both /2 | negatives /16 | all /39 | LLM p50 |
|---|---|---|---|---|---|---|---|
| v4c | 3B | 9 | 7 | 0 | 15 | 31 | 4.4 s |
| **v5** | 3B | 10 | 8 | 0 | 14 | **32** | 5.2 s |
| v4c | 1.5B | 6 | 1 | 0 | 15 | 22 | 3.4 s |
| **v5** | 1.5B | 11 | 2 | 0 | 15 | **28** | 2.9 s |
| short prompt, only these rules (diagnostic) | 1.5B | 7 | 3 | 0 | 15 | 25 | 2.0 s |
| v2 on-device path (reference) | 1.5B | 4 | 3 | 0 | 16 | 23 | 1.8 s |

The "both" cases (a correction inside a list) fail with every prompt on both
models.

**Parity main set** (13 clips, 31 format checks, Qwen2.5-3B standing in for the cloud model):

| ASR | prompt | format | WER | instructions kept | LLM p50 |
|---|---|---|---|---|---|
| Parakeet greedy | v4c | 23/31 | 13.9 % | 3/3 | 7.5 s |
| Parakeet greedy | **v5** | **25/31** | 14.4 % | 3/3 | 8.6 s |
| beam + dictionary | v4c | 22/31 | 13.7 % | 3/3 | 8.5 s |
| beam + dictionary | **v5** | **23/31** | 14.4 % | 3/3 | 8.7 s |

On greedy, the list/correction clips changed as follows:

| clip | format | what happened |
|---|---|---|
| s02 | 0/3 → 2/3 | bullets and the correction now pass; the third check fails on an ASR mishearing |
| s03 | 0/1 → 0/1 | the list is now made, but the counting words stay inside the items |
| s04 | 4/4 → 4/4 | no false list |
| s12 | 1/3 → 1/3 | unchanged |

On beam, s04 drops to 3/4: v5 quoted an inline item run instead of leaving it
plain. It still made no list.

**1.5B main set** (greedy; the first v5 run hit 4 back-to-back timeouts while
the box was busy, so both sides were run again):

| prompt | format | WER | instructions kept | LLM p50 |
|---|---|---|---|---|
| v4c | 18/31 | 11.3 % | 3/3 | 4.1 s |
| **v5** | **21/31** | 10.9 % | 3/3 | 5.2 s |

On the 1.5B, s02 goes 0/3 → 2/3. s03, s04, s08 and s12 are unchanged.

**113-case AI-cleanup suite** (`ai_cleanup_bench_2026_10_26`, synthetic plus the Wispr pairs):

| model | prompt | suite pass | corrections /7 | lists /5 | newline /5 | spoken marks /9 | negatives /19 |
|---|---|---|---|---|---|---|---|
| 3B | v4c | 72 % | 2 | 4 | 3 | 8 | 15 |
| 3B | v5 | 71 % | **6** | 4 | **1** | **6** | 15 |
| 1.5B | v4c | 72 % | 3 | 3 | 1 | 7 | 16 |
| 1.5B | v5 | 73 % | **5** | 4 | 1 | 7 | 17 |

**Regression: spoken "new line" / "new paragraph" and spoken marks on the 3B.** Spoken marks ("comma", "period", "question mark") fall from 8 to 6/9: v5 left a spoken "Period." in the text, and once it wrote a colon where "comma" was said. v5 also turns
"<lead-in>. New paragraph. A or B." into a bulleted list, which is a false
list, and writes "new line" as a blank line. This is the blank line the list
format uses. Two follow-ups did not fix it, so neither shipped:

* **v5b:** one more rule sentence, "line commands are not a list; 'or'
  choices are not a list". Newline stayed at 1/5, and main greedy WER rose to
  15.5 %.
* **v5c:** one more made-up few-shot, a "new paragraph" followed by an "or"
  choice. Newline stayed at 1/5 and main greedy WER rose to 16.8 %.

The model treats the list format as a strong pull. Fixing this needs a
different list-rule shape or a bigger model, not more text.

**Cost.**
* The static prefix grows from about 9,460 to 12,025 characters (+27 %, about
  +650 tokens).
* The prefix is prefilled once at model load (KV cache). The warm-up on this
  shared box took 72 s for v4c and 101 s for v5 on the 3B; on the 1.5B the
  difference was lost in noise.
* Per-dictation LLM time: 3B +0.8 s p50 on synthetic, +1.1 s on the main set.
  1.5B: no slowdown.
* On cloud models the extra prefix tokens cost per call unless the provider
  caches them.

## Not done / open

* A correction inside a list (X cases) fails everywhere.
* "two, not three" (case N02) is ambiguous and fails everywhere.
* The phone bench (debug `BenchPlan.kt` / `crates/flow-ffi/src/bench.rs`) still runs only `prod` and `v4c`. It has no v5 mode, so v5 has not been timed on the phone.
* 1.5B self-corrections would need a bigger model or fine-tuning, not more
  prompt text.
