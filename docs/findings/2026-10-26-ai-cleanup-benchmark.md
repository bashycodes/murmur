# AI cleanup benchmark, phase 1: local ≤3B models and OpenRouter (2026-10-26, draft)

**Phase 1d update (OpenRouter, same day).** Through OpenRouter (product `OpenRouterCleaner`,
`zdr: true`, `data_collection: deny`, prompt v4b, raw input), **Llama-3.3-70B is the only
configuration that meets the ≤ 1.5 s median budget *and* beats every local model on Wispr
closeness**: end-to-end p50 **0.75 s** / p90 1.34 s from this box, target-closeness **0.909**
(char ratio 0.98), 88-suite 76 %, $0.0013 per call. Two 8B-class ZDR models are as fast
(0.54–0.77 s) and cheaper but weaker. Total spent: **$0.18** of the $1.50 cap.
Recommendation: [§ Recommendation](#recommendation-against-the-15-s-budget).

**Question.** The owner decided (2026-10-26) that *AI cleanup* (GLOSSARY) does all
formatting in one pass on the **raw** transcript, with the
*fallback rules* only when the AI cannot run or its answer is rejected, under a
loosened guard and a budget of ≤ ~1.5 s added at the median on the server route.
Can a local model on the home server (i7-6700, 4C/8T, no GPU) do that?

**Answer (local Qwen2.5-1.5B / 3B; Llama-3.1-8B in § Local Llama-3.1-8B; Qwen 7B skipped by the owner).**

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

| category | n | rules | prod 1.5B | prod 3B | v2 raw 1.5B | v4 1.5B | **v4b 1.5B** | v4 3B | v4b 3B | 7B | 8B |
|---|---|---|---|---|---|---|---|---|---|---|---|
| quote | 8 | 8/8 | 8/8 | 8/8 | 7/8 | 6/8 | 6/8 | 4/8 | 4/8 | skipped (owner) | see § Local Llama-3.1-8B |
| paren | 6 | 6/6 | 6/6 | 6/6 | 4/6 | 1/6 | 2/6 | 2/6 | 4/6 | | |
| spoken_punct | 9 | 9/9 | 9/9 | 9/9 | 8/9 | 7/9 | 7/9 | 6/9 | 8/9 | | |
| newline | 5 | 5/5 | 5/5 | 5/5 | 0/5 | 0/5 | 1/5 | 2/5 | 2/5 | | |
| correction | 7 | 5/7 | 5/7 | 6/7 | 1/7 | 4/7 | 4/7 | 1/7 | 3/7 | | |
| itn | 8 | 8/8 | 8/8 | 8/8 | 6/8 | 7/8 | 6/8 | 7/8 | 5/8 | | |
| list | 5 | 3/5 | 3/5 | 4/5 | 3/5 | 3/5 | 3/5 | 3/5 | 3/5 | | |
| casual | 7 | 7/7 | 7/7 | 6/7 | 7/7 | 7/7 | 7/7 | 6/7 | 6/7 | | |
| email | 5 | 5/5 | 5/5 | 5/5 | 4/5 | 2/5 | 5/5 | 5/5 | 5/5 | | |
| filler | 6 | 6/6 | 6/6 | 6/6 | 4/6 | 3/6 | 3/6 | 1/6 | 2/6 | | |
| runon | 6 | 0/6 | 3/6 | 1/6 | 4/6 | 4/6 | 4/6 | 3/6 | 3/6 | | |
| negative | 16 | 16/16 | 16/16 | 12/16 | 15/16 | 12/16 | 13/16 | 13/16 | 12/16 | | |
| **88-suite pass** | 88 | 89 % | **92 %** | 86 % | 72 % | 64 % | 69 % | 60 % | 65 % | | |
| Wispr A/B exact | 10 | 1 | 2 | 0 | 3 | 4 | 3 | 1 | 2 | | |
| owner examples exact | 4 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | | |
| **target-closeness (14)** | 14 | 0.757 | 0.826 | 0.750 | 0.855 | **0.871** | 0.866 | 0.838 | 0.833 | | |
| strict guard rejects | 102 | – | 2 | 0 | 7 | 4 | 6 | 5 | 5 | | |
| loosened guard rejects | 102 | – | – | – | 7 | 4 | 6 | 5 | 5 | | |
| LLM ms p50 / p90 | | – | 1374 / 2746 ¹ | 2783 / 3000 ¹ | 1448 / 2579 | 1503 / 2675 | 1529 / 2597 | 2893 / 4630 | 2831 / 4681 | | |
| LLM calls > 1500 ms | | – | 24 / 50 | 48 / 50 | 47 / 102 | 52 / 102 | 55 / 102 | 99 / 102 | 99 / 102 | | |
| peak RSS (bench process) | | 7 MB | 1.90 GB | 3.58 GB | 1.90 GB | 1.90 GB | 1.90 GB | 3.58 GB | 3.58 GB | | |

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

## Phase 1d: OpenRouter

* **Setup.** `run_or.sh` → the runner's `--openrouter MODEL` uses the product's
  `flow_cloud::OpenRouterCleaner` unmodified: streamed chat completion, `temperature 0`,
  `provider {zdr: true, data_collection: "deny", sort: "latency", allow_fallbacks: true}`,
  minimal prompt scope (no app id, as D28), prompt v4b, both guards, fallback rules on a
  reject, 8 s budget. The key is read from the key file into the process env only. Latency
  = request start to last token, measured on this box (home connection), so it is the
  client's real added latency, not the model's speed alone. Cost = each call's
  `usage.cost`; a cap on the summed cost over all runs ($1.50) stops a run.
* **Model picks** (live ZDR endpoint list, `GET /api/v1/endpoints/zdr`, 2026-10-26):
  * `meta-llama/llama-3.3-70b-instruct`: the D28 cleanup default (asked for).
  * `meta-llama/llama-3.1-8b-instruct`: the base of Wispr's own fallback formatter (a
    fine-tuned Llama-3.1-8B, research 03), the same model as our local 8B row, and
    5 ZDR providers including Groq (LPU, low latency) at $0.02–0.22 / M tokens.
  * `mistralai/ministral-8b-2512`: a second, non-reasoning 8B family with a first-party
    ZDR endpoint (Mistral), $0.15 / M. Skipped: Qwen3 / Qwen3.5 (reasoning models, extra
    latency), Gemma-3-4B/12B (fewer ZDR providers with uptime data).
* **Suite note.** Integration (5d0747b) added 4 "regression" cases to the formatting suite;
  three reuse the A/B ids ab5/ab7/ab8. `score.py` keeps the original 88 + 14 so all rows
  compare; the cloud runs made 106 calls (cost below counts all of them). The fallback
  rules after 5d0747b: 88-suite 89 % (unchanged), target-closeness 0.784 (was 0.757).

| | rules (5d0747b) | prod 1.5B | v4b 1.5B (local) | **Llama-3.3-70B** | Llama-3.1-8B | Ministral-8B |
|---|---|---|---|---|---|---|
| quote / paren / spoken_punct | 8/8 · 6/6 · 9/9 | 8/8 · 6/6 · 9/9 | 6/8 · 2/6 · 7/9 | 6/8 · 5/6 · 8/9 | 6/8 · 6/6 · 8/9 | 5/8 · 5/6 · 7/9 |
| newline / correction / itn | 5/5 · 5/7 · 8/8 | 5/5 · 5/7 · 8/8 | 1/5 · 4/7 · 6/8 | 5/5 · 6/7 · 8/8 | 3/5 · 5/7 · 8/8 | 1/5 · 6/7 · 6/8 |
| list / casual / email | 3/5 · 7/7 · 5/5 | 3/5 · 7/7 · 5/5 | 3/5 · 7/7 · 5/5 | 1/5 · 2/7 · 5/5 | 2/5 · 4/7 · 5/5 | 3/5 · 7/7 · 5/5 |
| filler / runon / negative | 6/6 · 0/6 · 16/16 | 6/6 · 3/6 · 16/16 | 3/6 · 4/6 · 13/16 | 5/6 · 3/6 · 13/16 | 6/6 · 4/6 · 10/16 | 6/6 · 1/6 · 12/16 |
| **88-suite pass** | 89 % | **92 %** | 69 % | 76 % | 76 % | 73 % |
| Wispr exact (10 A/B + 4 owner) | 2 + 0 | 2 + 0 | 3 + 0 | **4 + 1** | 3 + 0 | 2 + 0 |
| **target-closeness (14)** | 0.784 | 0.826 | 0.866 | **0.909** | 0.886 | 0.843 |
| Wispr char ratio (14) | 0.856 | 0.867 | 0.875 | **0.979** | 0.906 | 0.885 |
| strict / loosened rejects (of 102) | – | 2 / – | 6 / 6 | 1 / 1 | 2 / 2 | 9 / 9 |
| good answers rejected | – | 0 | 0 | 0 | 0 | 0 |
| LLM ms p50 / p90 (end-to-end) | – | 1374 / 2746 | 1529 / 2597 | **746 / 1336** | 772 / 1208 | 539 / 1074 |
| calls > 1500 ms | – | 24 / 50 | 55 / 102 | 3 / 102 | 1 / 102 | 5 / 102 |
| time to first token p50 | – | – | – | 571 ms | 686 ms | 350 ms |
| cost per call (mean) | – | – | – | $0.00127 | $0.00036 | $0.00005 |
| run total (106 calls) | – | – | – | $0.1346 | $0.0383 | $0.0055 |

Spend: summed `usage.cost` $0.181 including a 2-call smoke test ($0.0025); the key's own
usage counter read $0.178 after the last run (it lags by seconds). No call errored, timed
out or needed an estimated cost.

What the cloud models do:

* **Llama-3.3-70B** writes what Wispr writes on the owner's dictations: o2 is exact
  ("… run it on the server (for instance, if that would give me even more accurate, better,
  and faster transcriptions)."); o1 differs only in "hiking boots; some of them say 32 by 32".
  Its 88-suite misses are mostly conventions, not damage:
  * the casual STYLE line ("sentence case, drop optional commas") made it lowercase or drop
    "Okay," / "Yeah," (k4 "haha that's hilarious I cant believe he did that"; k5, k6) —
    the style wording needs a rewrite for big models;
  * numbered lists without the blank line after the intro (l1–l3, l5);
  * but also two real errors: literal "in parentheses" / "in brackets" turned into marks
    (x13 "The formula is (on the board).", x15). **The guard cannot catch this**: it strips
    command words from both sides before comparing, so a wrongly applied command looks the
    same as a correct one.
* **Llama-3.1-8B**: good on parens/fillers/ITN, but 10/16 negatives (applies commands to
  literal uses) and answered the all-filler f6 with "Nothing to format" (rejected by the
  guard, correctly).
* **Ministral-8B**: fastest and cheapest, but replies with meta-text on filler-only or odd
  inputs ("(no reply)*", "(No reply—transcript contained only filler…)" on x7/x9, which are
  real sentences) and rewrites (o3 "I can speak super fast, whisper very low, or …"). The
  guard caught 9 of these; the fallback rules then produced the text.
* **Guards on cloud output:** strict and loosened agree on all 306 cloud answers.

## Local Llama-3.1-8B (v4b)

Run by the orchestrator with flow-server stopped (`run.sh v4b_l8 ai
Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf prompt_v4b.json`, 8 s cap, 4 threads): wall 16 min 34 s,
**peak RSS 8.4 GB**, model load 135 s, warm-up prefill of the v4b prefix 107 s.

| | rules (5d0747b) | v4b 1.5B | v4b 3B | **v4b 8B local** | Llama-3.1-8B OpenRouter | Llama-3.3-70B OpenRouter |
|---|---|---|---|---|---|---|
| quote / paren / spoken_punct | 8/8 · 6/6 · 9/9 | 6/8 · 2/6 · 7/9 | 4/8 · 4/6 · 8/9 | 7/8 · 6/6 · 9/9 | 6/8 · 6/6 · 8/9 | 6/8 · 5/6 · 8/9 |
| newline / correction / itn | 5/5 · 5/7 · 8/8 | 1/5 · 4/7 · 6/8 | 2/5 · 3/7 · 5/8 | 4/5 · 6/7 · 8/8 | 3/5 · 5/7 · 8/8 | 5/5 · 6/7 · 8/8 |
| list / casual / email | 3/5 · 7/7 · 5/5 | 3/5 · 7/7 · 5/5 | 3/5 · 6/7 · 5/5 | 3/5 · 5/7 · 5/5 | 2/5 · 4/7 · 5/5 | 1/5 · 2/7 · 5/5 |
| filler / runon / negative | 6/6 · 0/6 · 16/16 | 3/6 · 4/6 · 13/16 | 2/6 · 3/6 · 12/16 | 6/6 · 2/6 · 12/16 | 6/6 · 4/6 · 10/16 | 5/6 · 3/6 · 13/16 |
| **88-suite pass** | 89 % | 69 % | 65 % | **83 %** ¹ | 76 % | 76 % |
| Wispr exact (14) | 2 | 3 | 2 | 1 | 3 | 5 |
| **target-closeness (14)** | 0.784 | 0.866 | 0.833 | **0.773** ¹ | 0.886 | 0.909 |
| strict / loosened rejects (of 102) | – | 6 / 6 | 5 / 5 | 42 / 42 ² | 2 / 2 | 1 / 1 |
| LLM ms p50 / p90 | – | 1529 / 2597 | 2831 / 4681 | **7462 / 8000** | 772 / 1208 | 746 / 1336 |
| calls > 1500 ms | – | 55 / 102 | 99 / 102 | 102 / 102 | 1 / 102 | 3 / 102 |
| peak RSS | – | 1.9 GB | 3.6 GB | 8.4 GB | – | – |

¹ 41 of the 102 calls hit the 8 s cap and fell back to the rules. The 88-suite score is
therefore mostly the rules' score. 12 of the 14 Wispr cases timed out (only ab3 and ab6
finished), so its closeness is the rules' (0.784) and says nothing about the 8B itself. On the 61 calls that
finished, the 8B's own answers pass 48 (79 %), median 6.5 s.
² 41 of the 42 rejects are those timeouts ("no answer"); one real reject (s7, "Quick
question" dropped). Of the 42, 25 are counted "good rejected" only because, after a
timeout, the no-guard column also falls back to the rules, which pass. No answered case was
wrongly rejected. Strict and loosened agree on every answer.

The 8B's quality is between the 3B and the cloud models (best local on parens, quotes,
fillers and ITN), but at 7.5 s p50 it is 5× the budget on this CPU.

## Recommendation against the 1.5 s budget

1. **On the home server CPU, no local model meets the budget at useful quality.** The 1.5B
   sits at p50 1.5 s on mostly short dictations (3–6 s on the owner's 30–50-word ones) and
   is worse than the fallback rules on the suite; 3B is 2.9 s; the 8B is 7.5 s, needs 8.4 GB
   RAM (only with flow-server stopped) and timed out on 40 % of the cases. Its answers are
   good when they arrive (79 % pass on those), but it is 5× over the budget. Local AI
   cleanup on this box is closed for now; a GPU would reopen it.
2. **If the owner accepts cloud text cleanup (D28 opt-in, consent per call), use OpenRouter
   Llama-3.3-70B as the AI cleanup engine**, with the fallback rules on a guard reject or
   the 2.5 s cloud timeout. It is the only measured configuration under budget at the median
   (0.75 s, p90 1.34 s) and the closest to Wispr. Cost ≈ $1.30 per 1 000 dictations at the
   v4b prompt size (~2 k prompt tokens; most of it is the static prefix, so prompt caching
   would cut it). D28 keeps cloud cleanup off the server route, so this is a client-side
   call; the budget then applies to the client's release→text.
3. Before that ships: fix the casual STYLE wording (big models over-apply it), add the
   blank line after a list intro to the prompt, and close the guard gap for literal command
   words (compare command words, not only strip them: a command word the input has as
   plain text must not become a mark).
4. **Without cloud: keep today's production pipeline** (rules → v2 → guard → skip, 92 %)
   and use the local 1.5B only where its p50 fits; the all-in-one raw-in design is not
   reachable locally on this CPU.

## Budget read (i7-6700, 4 LLM threads, server shared)

* The fixed cost of one call is ~1 s (user-message prefill + the first tokens): the 1.5B
  needs 1.3 s even for ≤ 10 words. Decode then adds roughly 0.05–0.1 s per output word.
* **1.5B:** median 1.5 s on this suite (mostly short dictations) is *at* the budget; the
  owner's real dictations (30–50 words) take 3–6 s.
* **3B:** 2.9 s median, 5–8 s on long dictations: twice the budget.
* **8B (measured, server stopped):** p50 **7.5 s**; 41 of 102 calls hit the 8 s cap; the
  calls that finished had a median of 6.5 s. That is in line with the ~6–7 s the
  1.5B → 3B ratio predicted. Wispr's own fallback formatter is a fine-tuned Llama-3.1-8B
  (`research/windows/findings/by-topic/03-ai-formatting-and-features.md`), served on GPUs.
  Qwen 7B skipped (owner).
* So on this CPU the budget is reachable only with ≤ 1.5B models and short dictations.
  An 8B-class AI cleanup in ≤ 1.5 s needs a GPU or the OpenRouter route, where the same
  Llama-3.1-8B answers in 0.77 s p50 and the 70B in 0.75 s (§ Phase 1d).

## Status of the remaining rows

* **Qwen2.5-7B (local): skipped (owner).** Also skipped: the production-pipeline row with
  the 8B (`prod_l8`).
* **Llama-3.1-8B (local, v4b): done**, run by the orchestrator with the server stopped
  (§ Local Llama-3.1-8B). GGUF: bartowski/Meta-Llama-3.1-8B-Instruct-GGUF Q4_K_M,
  4 920 739 232 bytes, sha256 `7b064f58…3033557c` = the repo's LFS hash; the GGUF's own chat
  template works through flow-llm.
* ~~OpenRouter~~: done in phase 1d (above).

## Uncertain

* The 88-suite targets were written before the owner's light-edit decision; some AI
  "fails" are Wispr-like edits ("Don't forget we have dinner at 8" for "Don't forget that
  …", x6). Neither suite is a full Wispr reference; only the 14 Wispr cases are.
* The owner-example inputs are synthesised; real Parakeet output for those dictations
  would be punctuated, which favours the rules-first pipeline.
* Latency is on a shared box (flow-server with its own LLM resident, other agents); compare
  columns, not absolute numbers with other machines.
* One greedy run per configuration; no repeat runs.
