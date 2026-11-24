# 2026-11-24: A filtered SCREEN NAMES line for cleanup; 1.5B vs 7B (D108)

**Question.** With "Send on-screen words" on, every screen entry (up to 200)
was appended to the cleanup prompt's DICTIONARY line. Does a short line of
on-screen names the transcript comes near fix planted names on the server's
qwen2.5-1.5b without the `llm_timeout`s, and is qwen2.5-7b worth it?

**Answer.** The filtered line ends the timeouts: on the server's 1.5B,
`llm_timeout` falls from **90 % to 0 %** and planted names come out right in
**23/40** cases instead of 2/40 (0/40 with no screen words). p95 `llm_ms` is
**2126 ms** (2329 ms on the short screens alone), under the 2500 ms target.
The **80 % name target is missed** (58 %): 2 of 20 names never reach the line
(see Misses) and the 1.5B leaves 6 to 7 of the other 18 as heard. The **7B is
not worth it** on the server's CPU: at the server's budget it times out on
68 % of dictations with the new prompt (100 % with the old one); given 30 s it
spells 24/40 names right ignoring case (17/40 exactly, it often lowercases the
whole answer), about what the 1.5B gets, at a p50 of 6.3 s.

Targets (at least 80 % names right, `llm_timeout` at most 5 %, p95 `llm_ms` at
most 2500): the 1.5B with D108 meets the two latency targets and misses the
name target; the 7B misses all three at the server's budget.

## Change (D108)

* Screen words no longer go on the DICTIONARY line. `build_prompt_versioned`
  derives `screen_names::name_candidates` from the raw screen words, keeps the
  ones `terms::select_terms` names for the transcript (the D64 rules), drops
  those already on the DICTIONARY line, and puts at most 20 on a
  `SCREEN NAMES:` line, followed by one line: "(Names shown on screen: use one
  of these spellings only where the TRANSCRIPT clearly refers to that name.)"
* Both lines are in the per-dictation user message and only when a name is
  near, so the cached system prompt and few-shots of every prompt version are
  unchanged: no `PromptVersion` bump and no prefix rebuild.
* The request keeps the raw screen words (`CleanRequest.ctx`), so the D65
  Hebrew check still knows a Latin word only a bubble shows.
* Server log: `dict_terms_in_prompt` counts only dictionary entries;
  `screen_names_in_prompt=` follows it on the `result` and `cleaned` lines.

## Setup

* Probe: `tools/probes/prototypes/cleanup_screen_names_2026_11_24/` (runner,
  `prepare.py`, `run.sh`, `score.py`, result rows).
* Cases: the 20 English sentences of
  `crates/murmur-core/fixtures/screen-names/chat-screens.json` (all invented),
  each with its planted name misspelled as a speech model might write it
  ("Tavial Marrow", "Quill Feather", "Dunston Price", "Solvang Pereira"), on
  its screen as it is (`short`) and at the bottom of a 2000-character capture
  (`full`): 40 cases. A case is right when every planted name is in the
  output exactly.
* Path: `MurmurCore::commit` as murmur-server runs it (rules, then prompt v2
  on llama.cpp), 4 threads (the server's), light level, budget 3000 ms + 15 ms
  per character (the server's default), cleanup never skipped.
* Arms: `new` (this branch), `old` (the same prompt rewritten to main's: raw
  screen words on the DICTIONARY line), `none` (no screen words).
* Models: qwen2.5-1.5b-instruct q4_k_m (the server's), qwen2.5-7b-instruct
  q4_k_m, one at a time on a shared machine that was also serving.
* Not as the server: the runner loads the model without the server's draft
  setting (`[model] llm_draft`, default 4, D69), which speeds up decoding
  answers that copy the prompt. Prefill, where the old prompt lost its time,
  is not affected, but the server's `llm_ms` should be a little lower than
  here on every arm.
* Counts: the `old` arm's rows give `dict_terms_in_prompt` as the entries of
  its rewritten DICTIONARY line (15 to 55 here), not the core's count of the
  prompt it built before the rewrite.

## Results

| run | cases right | any case | names right | llm_timeout | other fallback | p50 llm_ms | p95 llm_ms | max llm_ms | avg user msg chars | max prompt tokens |
|---|---|---|---|---|---|---|---|---|---|---|
| 1.5B new | 23/40 | 23/40 | 58 % | 0/40 (0 %) | 0 | 1678 | 2126 | 2465 | 314 | 1849 |
| 1.5B new, short only | 12/20 | 12/20 | 60 % | 0/20 (0 %) | 0 | 1874 | 2329 | 2465 | 314 | 1849 |
| 1.5B old | 2/40 | 2/40 | 5 % | 36/40 (90 %) | 0 | 3810 | 3915 | 4050 | 1497 | 2364 |
| 1.5B none | 0/40 | 0/40 | 0 % | 0/40 (0 %) | 0 | 985 | 1385 | 1558 | 188 | 1815 |
| 7B new | 2/40 | 4/40 | 5 % | 27/40 (68 %) | 1 | 3750 | 3915 | 4050 | 314 | 1849 |
| 7B old | 0/40 | 0/40 | 0 % | 40/40 (100 %) | 0 | 3810 | 3915 | 4050 | 1497 | 2343 |
| 7B none | 0/40 | 0/40 | 0 % | 25/40 (62 %) | 0 | 3723 | 3915 | 4050 | 188 | 1815 |
| 7B new, 30 s budget | 17/40 | 24/40 | 42 % | 0/40 (0 %) | 0 | 6309 | 8603 | 10237 | 314 | 1849 |

Why the old prompt times out: each dictation's DICTIONARY line differs, so
its user message (about 130 to 570 tokens here, against 1 to 73 with the new
prompt) is prefilled uncached at about 65 to 115 tokens/s on 4 threads,
which alone takes longer than the ~3.7 s budget. The old arm's 2/40 names
are not a model-quality figure: 36 of its 40 dictations fell back to the
rules' text, and the 2 right cases are among the 4 that did not.

"any case" ignores case: the 7B often lowercased the whole answer in these
casual-style chats. Each `full` case runs right after its `short` twin with
the same SCREEN NAMES line, so its prefill is mostly cached. The short-only
row is the fair latency figure for the new prompt.

## Misses

* **Not on the line (2/20).** "Elsabet Varga" for Ilsabet Varga: D64 needs
  the same opening sound. "Bartolo" for Bartholo: the screen has it only
  inside a sentence ("ask Uncle Bartholo if ..."), so the candidate is
  "Uncle Bartholo", which no window of "ask Bartolo about" comes near.
* **On the line, left as heard by the 1.5B (6 short, 7 full).** Kellan
  Frost (an ordinary word), Severin's, Peregrine Sabo (half fixed),
  Ackterberg, Quintavius, Solvang Pereira (the hyphen), Valoran on one full
  screen.
* **Noise.** A capitalised screen label that the transcript has in lower case
  ("Batches", "Invoice", "release-planning") is also named: D64 names a term
  the transcript spells only in another case. The model never wrote one of
  them into an answer here.

## Recommendation

Keep the server on qwen2.5-1.5b. On this CPU the 7B cannot answer within the
dictation budget, even with the short prompt, and given unlimited time it
spells about as many names right as the 1.5B.

## Follow-up ideas (each needs its own bench)

* Match each word of a multi-word candidate on its own ("Uncle Bartholo" →
  "Bartholo"), so a name met only inside a sentence reaches the line.
* A few-shot turn with a SCREEN NAMES line on the 1.5B, to show the model
  respelling a near name (Frost → Fross, Quintavius → Quentavius).
* D64's first-sound rule: "Elsabet" never names Ilsabet because the opening
  sound differs; a vowel-for-vowel opening could be allowed for names from
  the screen, at the cost of more noise.
