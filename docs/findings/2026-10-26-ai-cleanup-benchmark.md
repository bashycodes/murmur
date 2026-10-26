# AI cleanup benchmark, phase 1: local ≤3B models on the home server CPU (2026-10-26, draft)

**Question.** The owner decided (2026-10-26) that *AI cleanup* (GLOSSARY) does all
formatting in one pass on the **raw** transcript, with the
*fallback rules* only when the AI cannot run or its answer is rejected, under a
loosened guard and a budget of ≤ ~1.5 s added at the median on the server route.
Can a local model on the home server (i7-6700, 4C/8T, no GPU) do that?

**Answer so far (Qwen2.5-1.5B / 3B; 7B / 8B pending).**

* **Not with ≤3B models.** Raw-in AI cleanup scores **64–69 %** on the 88-case
  formatting suite, against **89 %** for the fallback rules alone and **92 %** for today's
  production pipeline (rules → prompt v2 → guard → skip). It loses most of the spoken
  commands the rules handle exactly (AI rows: paren 1–4/6, newline 0–2/5, filler 1–3/6, negatives 12–13/16).
* **It is closer to Wispr on the 14 Wispr-target cases**: target-closeness **0.866–0.871**
  (1.5B, v4/v4b) vs 0.826 for production and 0.757 for the rules. On the owner's
  unpunctuated long dictations production returns almost raw text (its LLM times out at
  3 s and the rules do not punctuate run-ons); the 1.5B returns readable sentences.
* **Budget:** the 1.5B is at the budget's edge (LLM p50 **1.5 s**, p90 2.6 s; ≤10-word
  dictations 1.3 s, > 35 words 5.8 s). The 3B is twice that (p50 **2.9 s**; 3 of the 4
  owner examples ran past 8 s). On this CPU, 7B/8B would be roughly 2–2.5× the 3B, so
  **≤1.5 s at the median is out of reach for any model larger than 1.5B on this box.**
* **Guards:** strict and loosened rejected the same 4–7 answers per run (2 differences in
  510 judged answers) and **never rejected an answer that passed**. The loosening matters
  little for these models; their errors are omissions and literal commands, which neither
  guard is meant to catch.

## Method

Prototype: `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/` (README there).

* **Cases (102).** The 88-case formatting suite of 2026-10-24 (inputs Parakeet-style:
  punctuated, commands between commas) plus 14 Wispr targets: the 10 A/B pairs
  (`wispr_ab_2026_10_25`, input = the read script, target = Wispr's output) and the owner's
  4 examples. Those 4 are Wispr outputs only, so `gen_wispr_cases.py` synthesises a raw
  input (lowercase, no punctuation, spoken numbers, a filler or two; o2 assumes a spoken
  "in parentheses"). All 14 run as app Notes, category other (formal).
* **Rows.**
  * *rules*: fallback rules only (`spoken_cleanup` + layout + closing period + post-processing).
  * *prod*: `FlowCore::commit` as shipped (D27): rules, then prompt v2, product guard,
    `llm_skip_clean`, 3 s LLM budget, chat buckets casual.
  * *AI (v4 / v4b)*: the owner's design. Raw transcript → LLM (no rules before it, no skip,
    8 s budget so latency is measured, not cut) → guard → on a reject the fallback rules →
    post-processing (dictionary, cursor context) as in the product. One LLM call gives three
    columns: no guard, strict guard, loosened guard.
  * *v2 raw*: the product prompt v2 on the raw transcript (reference).
* **Prompt v4** (`prompt_v4.json`, our own wording; Wispr's server prompt is not in the
  client and no vendor text was used): one system prompt covering punctuation, spoken
  marks, quote/paren/bracket commands, literal command words, numbers/money/times/ages/
  short units, self-corrections, counted lists, emails, light edits (fillers, leading "so",
  tiny trims) and "never answer the transcript", plus 17 few-shots. **v4b** adds one line
  ("the speech model may already have punctuated, with commas around commands") and 4
  few-shots in that form. `score.py --overlap`: no few-shot shares a 5-word run with any
  of the 102 cases.
* **Loosened guard** (`runner/src/loose_guard.rs`): a copy of `flow_core::guard`, each
  change marked. Allows: more hedge fillers dropped ("actually", "basically", …), unit
  abbreviations ("lbs" = pound, "cm", "kg", "x" = by). Adds limits: a name the input
  capitalises must survive; at most 20 % of the input's content words may go. Unchanged:
  every output number must be backed by the input, lost amounts/times reject, added content
  words over 25 % of a sentence reject. Tried and **reverted**: tolerating two content words
  in a row gone. It let the 1.5B delete the aside of "The deadline dash which nobody likes
  dash is Friday" ("likes" is a stopword, so only 2 words counted); the trims the owner
  named ("and it weighs" → "and weighs") lose only stopwords and already pass.
* **Scoring** (`score.py`): pass = normalised match with the expected text or an accepted
  variant, **final period ignored** on both sides (owner: Wispr was in formal mode).
  target-closeness = 1 − token edit distance / max length (words and punctuation marks as
  tokens, same normalisation), mean over the 14.
* **Box.** LLM threads 4, greedy (flow-llm plugin, release build of this commit). The
  production flow-server (with its own Qwen 1.5B) stayed up; load average 3–5; every run
  under `flock` + `choom 1000` + `nice 10`, started only with ≥ model + 2000 MB available.
  Warm-up: one untimed clean, like the product's prefill at load (v4 prefix 19–23 s cold
  on the 1.5B, 37–44 s on the 3B).

## Results

| category | n | rules | prod 1.5B | prod 3B | v2 raw 1.5B | v4 1.5B | **v4b 1.5B** | v4 3B | v4b 3B | 7B | 8B | 70B (OpenRouter) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quote | 8 | 8/8 | 8/8 | 8/8 | 7/8 | 6/8 | 6/8 | 4/8 | 4/8 | pending | pending | skipped |
| paren | 6 | 6/6 | 6/6 | 6/6 | 4/6 | 1/6 | 2/6 | 2/6 | 4/6 | | | |
| spoken_punct | 9 | 9/9 | 9/9 | 9/9 | 8/9 | 7/9 | 7/9 | 6/9 | 8/9 | | | |
| newline | 5 | 5/5 | 5/5 | 5/5 | 0/5 | 0/5 | 1/5 | 2/5 | 2/5 | | | |
| correction | 7 | 5/7 | 5/7 | 6/7 | 1/7 | 4/7 | 4/7 | 1/7 | 3/7 | | | |
| itn | 8 | 8/8 | 8/8 | 8/8 | 6/8 | 7/8 | 6/8 | 7/8 | 5/8 | | | |
| list | 5 | 3/5 | 3/5 | 4/5 | 3/5 | 3/5 | 3/5 | 3/5 | 3/5 | | | |
| casual | 7 | 7/7 | 7/7 | 6/7 | 7/7 | 7/7 | 7/7 | 6/7 | 6/7 | | | |
| email | 5 | 5/5 | 5/5 | 5/5 | 4/5 | 2/5 | 5/5 | 5/5 | 5/5 | | | |
| filler | 6 | 6/6 | 6/6 | 6/6 | 4/6 | 3/6 | 3/6 | 1/6 | 2/6 | | | |
| runon | 6 | 0/6 | 3/6 | 1/6 | 4/6 | 4/6 | 4/6 | 3/6 | 3/6 | | | |
| negative | 16 | 16/16 | 16/16 | 12/16 | 15/16 | 12/16 | 13/16 | 13/16 | 12/16 | | | |
| **88-suite pass** | 88 | 89 % | **92 %** | 86 % | 72 % | 64 % | 69 % | 60 % | 65 % | | | |
| Wispr A/B exact | 10 | 1 | 2 | 0 | 3 | 4 | 3 | 1 | 2 | | | |
| owner examples exact | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | | | |
| **target-closeness (14)** | 14 | 0.757 | 0.826 | 0.750 | 0.855 | **0.871** | 0.866 | 0.838 | 0.833 | | | |
| strict guard rejects | 102 | – | 2 | 0 | 7 | 4 | 6 | 5 | 5 | | | |
| loosened guard rejects | 102 | – | – | – | 7 | 4 | 6 | 5 | 5 | | | |
| LLM ms p50 / p90 | | – | 1374 / 2746 ¹ | 2783 / 3000 ¹ | 1448 / 2579 | 1503 / 2675 | 1529 / 2597 | 2893 / 4630 | 2831 / 4681 | | | |
| LLM calls > 1500 ms | | – | 24 / 50 | 48 / 50 | 47 / 102 | 52 / 102 | 55 / 102 | 99 / 102 | 99 / 102 | | | |
| peak RSS (bench process) | | 7 MB | 1.90 GB | 3.58 GB | 1.90 GB | 1.90 GB | 1.90 GB | 3.58 GB | 3.58 GB | | | |

AI columns: strict-guard column shown (the loosened one is identical on every AI row).
¹ prod skips the LLM on 51/102 cases (`llm_skipped_clean`) and cuts it at 3 s: prod 1.5B
5 timeouts + 2 guard drops, prod 3B 23 timeouts. Its p50/p90 are over the 50 calls made.

AI-cleanup latency by dictation length (median LLM ms; n = 52 / 41 / 5 / 4):

| words | ≤ 10 | 11–20 | 21–35 | > 35 |
|---|---|---|---|---|
| Qwen2.5-1.5B (v4) | 1286 | 1802 | 2682 | 5838 |
| Qwen2.5-3B (v4) | 2501 | 3266 | 5412 | ≥ 8000 (3/4 timed out) |

### What the AI gets right and wrong

* **Better than production on the owner's style of dictation.** Production on o1:
  "So i am a 40 year old hiker coming in at about a hundred and 76 centimeters what do you
  think …" (LLM timed out, and the rules' number pass wrote "a hundred and 76", a rules
  bug worth fixing on its own). v4b 1.5B: "I am a 40-year-old hiker coming in at about
  158 cm. What do you think my boot size would be? I'm at the outlet …". It gets ages
  ("5-year-old"), times ("3:30"), "$250", "158 cm" and the correction "ten no wait eleven"
  → 11.
* **Worse than the rules on spoken commands in punctuated input.** "Grab the blue folder,
  in parentheses, the one on my desk." stays literal or loses "the one"; "New line." stays
  as words; "Um, so I was thinking" keeps the fillers; plain lines become numbered lists.
  The few-shots in v4b help the 3B on parens (2 → 4/6), the 1.5B barely (1 → 2/6).
* **It drops words the guard cannot see as harmful.** o4 "in the past while, after I asked
  …, but" → "in the past. After I asked …," ("while" and "but" gone); o2 loses "for
  instance". One content word per clause is within both guards' coverage rule.
* **No Wispr-style unit abbreviation from the 1.5B** ("4 pounds", not "4 lbs") and no
  bulleted "first thing / second thing" list; the A/B ab8 stays a sentence.

### Guard findings

* 510 judged answers over 5 AI runs: strict rejected 27, loosened 27, and both rejected
  no answer that passed. They differ on 2:
  * v2 raw o1, "I am a 40-year-old hiker, about 158 cm tall": strict rejects ("cm" and
    "tall" unsupported), loosened accepts ("cm" = centimeter, so only "tall" is added and
    the sentence stays above 75 % support). A real limitation: a single invented word in a
    long sentence passes both guards' support rule.
  * v2 raw ab8: loosened rejects (3 of 10 content words gone, the 20 % cap), strict accepts.
* Rejects that were right: "The meeting is at 10:00. Wait, don't come early." (ab6; the
  correction inverted), "The deadline is Friday." (s5, aside deleted), "Yeah, I don't think
  that will work" (k6, "going to" → "will"; arguably fine), "dropped: two" (l2).
* The owner's loosening targets (filler drops, "and it weighs" → "and weighs", "lbs")
  already pass the strict guard except the unit abbreviation, which the 1.5B never
  produced here. The loosened guard is ready for when a model does those edits; on these
  models it changes nothing measurable.

## Budget read (i7-6700, 4 LLM threads, server shared)

* The fixed cost of one call is ~1 s (user-message prefill + the first tokens): the 1.5B
  needs 1.3 s even for ≤ 10 words. Decode then adds roughly 0.05–0.1 s per output word.
* **1.5B:** median 1.5 s on this suite (mostly short dictations) is *at* the budget; the
  owner's real dictations (30–50 words) take 3–6 s.
* **3B:** 2.9 s median, 5–8 s on long dictations: twice the budget.
* **7B / 8B:** not run (they need the server stopped). From the 1.5B → 3B ratio and model
  size, expect ~2–2.5× the 3B, i.e. **~6–7 s median**, over 10 s on long dictations. Wispr's
  own fallback formatter is a fine-tuned Llama-3.1-8B
  (`research/windows/findings/by-topic/03-ai-formatting-and-features.md`), served on GPUs.
* So on this CPU the budget is reachable only with ≤ 1.5B models and short dictations.
  An 8B-class AI cleanup in ≤ 1.5 s needs a GPU or the OpenRouter route (D28; the 70B
  default there has not been measured: no key on this box).

## Pending

* **7B / 8B rows** (Qwen2.5-7B-Instruct Q4_K_M, Llama-3.1-8B-Instruct Q4_K_M): pending the
  server stop. `run_big.sh` runs prod + v4b for both; run.sh refuses to start below model
  size + 2000 MB available. Llama-3.1-8B GGUF downloaded to the models cache
  (bartowski/Meta-Llama-3.1-8B-Instruct-GGUF, 4 920 739 232 bytes, sha256
  `7b064f58…3033557c` = the repo's LFS hash). flow-llm uses the GGUF's own chat template;
  the 8B has not been loaded yet, so that is untested.
* **OpenRouter** (Llama-3.3-70B + two small ZDR models): **skipped**, no
  `~/.config/flow-clone/openrouter.key` on this box.

## Uncertain

* The 88-suite targets were written before the owner's light-edit decision; some AI
  "fails" are Wispr-like edits ("Don't forget we have dinner at 8" for "Don't forget that
  …", x6). Neither suite is a full Wispr reference; only the 14 Wispr cases are.
* The owner-example inputs are synthesised; real Parakeet output for those dictations
  would be punctuated, which favours the rules-first pipeline.
* Latency is on a shared box (flow-server with its own LLM resident, other agents); compare
  columns, not absolute numbers with other machines.
* One greedy run per configuration; no repeat runs.
