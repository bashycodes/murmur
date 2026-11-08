# 2026-11-07: faster flow-server cleanup on long clips (D69)

**Question.** On the parity main set, flow-server's commit→result was
4.9 s p50 / 9.4 s max without the dictionary and 7.9 / 12.8 s with it
(1.5B cleanup on the home server's CPU). Target: p50 ≤ 3 s on the home
server, no quality loss on the parity scorecard.

**Setup.** Probe `tools/probes/prototypes/server_cleanup_speed_2026_11_07/`
(scripts only; clips, cases and outputs stay private). Home server = this
box (i7-6700, 4 cores / 8 threads, shared with other builds at load 5–24,
so single numbers are noisy: variants run interleaved, 2–3 rounds).
Qwen2.5-1.5B-Instruct Q4_K_M, `threads` 4, prompt v2 as flow-server sends
it (never skipped), the session's budget (3 s + 15 ms/char, cap 8 s).
Two harnesses:

- **runner**: the 13 main-set transcripts (parakeet unified, greedy)
  straight into the real `FlowCore` commit with the plugin, LLM time only.
- **live**: a local flow-server build (own port, state dir and token, same
  models and config as the home server) fed the 13 clips by the parity
  eval's `run_paths.py server` at 4× real time, as the ticket's numbers.

Quality: the parity `score.py` (WER, CER, dictionary terms, format, kept
instructions) against the Wispr references.

## Where the time goes

commit→result = ASR tail + cleanup. Without the dictionary the cleanup is
most of it; the dictionary adds mostly **ASR** (hotword beam, asr p50
0.8 → 1.8 s at 4× pacing), not cleanup.

Cleanup per clip (runner, baseline, 3 rounds): LLM p50 3.9–4.2 s, max
7.2–7.5 s. Plugin counters (medians): prompt 1842 tokens, of which 55 are
prefilled (KV prefix reuse already keeps the system prompt and few-shots),
1.2 s; generation 38 tokens, 2.9 s (~80 ms/token, greedy, one token per
forward pass). So generation is ~70 % of the cleanup, prefill ~30 %.

## Options, cheapest first

| option | result | verdict |
|---|---|---|
| KV prefix reuse | already on; a warm commit prefills only its user message | kept |
| prefill-ahead | on, but it prefilled the **raw** partial while the commit prompt carries the rules-cleaned text, so the reuse stopped at the transcript's first token | fixed (below) |
| threads 6 / 8 | 6 ≈ 4; 8 far slower (11 of 13 hit the budget) | rejected |
| smaller model (0.5B) | earlier parity run: WER 9.8 → 12.0 %, format 18 → 17/31 | rejected |
| Llama-3.2-1B (+ drafting 4) | LLM p50 2.53 s, max 3.9 s; terms 9 → 8/15, WER 12.2 % | not adopted (quality) |
| chunking the transcript | splits one prefill + decode into several; generation tokens stay the same | not tried (no gain in the model) |
| budget scaling | exists (15 ms/char, cap 8 s); it bounds the max, does not cut p50 | unchanged |
| **prompt-lookup drafting, 4 tokens** | gen 2.9 → 1.5 s, LLM p50 3.9–4.2 → 2.58–2.65 s | **chosen** |
| drafting 3 / 6 / 8, min-match 2–3, adaptive length | 2.65 / 3.0 / 3.3 s; adaptive 4 = 2.56 s (no better than plain 4) | rejected |

Cleanup output is mostly the transcript copied with edits, so the next
tokens are usually in the prompt: drafting proposes up to 4 tokens that
follow the last matched n-gram of the prompt and checks them in one batch.
An earlier probe on 8 short dictations found no gain (`llm_ab.sh`, the
reason it was off); on the parity clips (long, mostly copied) it halves
generation.

## Quality

Runner, 13 clips, round 2 (round 1 agrees):

| variant | WER | CER | terms | format | instr. kept | LLM p50 | LLM max |
|---|---|---|---|---|---|---|---|
| no drafting | 12.0 % | 9.9 % | 9/15 | 18/31 | 3/3 | 3905 | 7176 |
| drafting 4 | 12.0 % | 9.7 % | 9/15 | 18/31 | 3/3 | 2576 | 6590 |

The final text is identical on 12 of 13 clips (11–12 across rounds).
Drafting is greedy up to rounding: a batched check computes the same
logits with different float summation, which flipped one near-tie token
on s04 / s05.

## Prefill-ahead on the rules text

`maybe_prefill` now runs the same spoken rules (`spoken_cleanup_keeping`
+ `layout`, or the whitespace tidy for non-English) on the partial that
the commit runs on the final transcript (`engine::spoken_rules`, shared),
so the prefilled tokens are the commit prompt's own up to where the
partial ends. Two cases still differ, costing reuse only: with
auto-detect and a non-English dictation the hint (which only knows the
forced language) gets the English rules, and a whole-utterance snippet
runs no LLM. No prompt wording or few-shot changes. Test:
`llm_prefill_hint_is_the_rules_text_the_commit_prompt_starts_with`
(fails before, passes after).

## Live

Local flow-server build, same models and config as the home server,
13 clips, 4× pacing (as the ticket), 2 interleaved rounds; base = the
integration build (no drafting, raw-partial prefill-ahead). Box load 5–24.

| run | commit→result p50 | max | ASR p50 | cleanup p50 | cleanup max |
|---|---|---|---|---|---|
| base, dictionary | 5.8 / 6.0 s | 11.4 / 10.8 s | 1.26 s | 3.6–3.7 s | 7.4 s |
| new, dictionary | 4.3 / 4.8 s | 8.2 / 8.1 s | 1.37 s | 2.5–2.6 s | 6.0 s |
| base, no dictionary | 5.2 / 5.3 s | 8.8 / 8.7 s | 0.81 s | 3.7–3.8 s | 7.1 s |
| new, no dictionary | 3.8 / 4.1 s | 8.3 / 8.2 s | 0.83 s | 2.5 s | 6.8–6.9 s |

Live quality (round 1): dictionary WER 10.7 → 10.7 %, terms 8/15, format
19/31, instructions 3/3, all unchanged; no dictionary WER 11.5 → 11.1 %.

Real-time pacing (1 round, load 20–25 during it, so noisy): base 4.6 s
(dict) / 5.2 s (no dict, 2 cleanup timeouts); new 4.6 s (dict, 1 timeout)
/ 3.3 s (no dict).

Plugin counters per commit (a local build that logged them): prefill-ahead
completes (0 preempted) and cuts the commit's prefill from ~55 to ~25
tokens on multi-piece clips, but on this loaded CPU 25 tokens still take
0.7–1.1 s; drafting accepts ~70 % of generated tokens (34 tokens in 7
batched steps of ~190 ms vs ~80 ms per single step).

**Target not met.** The closest is 3.3 s, without the dictionary at
real-time pacing. What is left: cleanup ~2.5 s (prefill ~1 s + generation
~1.5 s, both CPU-bound on a box other jobs load) plus the ASR tail
(0.8 s, 1.3 s with the dictionary's hotword beam). Further options:
Llama-3.2-1B with drafting (runner: LLM p50 2.53 s, max 6.6 → 3.9 s, but
terms 9 → 8/15, WER 12.0 → 12.2 %; not adopted), moving the dictionary
line after the transcript so prefill-ahead survives a changed term list
(a prompt-layout change, left to D67), a faster dictionary ASR path.

## Change

- flow-llm: drafting length is per engine (`Engine::set_draft`), exported
  as the optional ABI-1 symbol `flow_llm_set_draft`; `FLOW_LLM_DRAFT`
  still overrides; `flow_llm_info` reports `draft`.
- flow-core: `ModelConfig::llm_draft` (absent = the plugin's default,
  off); the loader calls `set_draft` when the plugin has it.
- flow-server: `[model] llm_draft` defaults to 4; `0` turns it off.
- Phone and PC unchanged (drafting stays off; not measured there).

**Deploy.** Rebuild and install flow-server and `libflow_llm.so` on the
home server (`tools/server/install.sh --llm`), restart. No config change.
