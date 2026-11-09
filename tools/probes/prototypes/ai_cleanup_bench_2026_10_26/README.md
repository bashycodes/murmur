# AI cleanup benchmark, 2026-10-26 (dated prototype)

> **Status:** dated probe, research only. Not a product tool; nothing here is
> wired into flow-core. Findings: `docs/findings/2026-10-26-ai-cleanup-benchmark.md`.
> Promote to a permanent path only together with the AI-cleanup implementation.

Benchmarks the owner's 2026-10-26 design (GLOSSARY "AI cleanup"): one LLM pass
does all formatting and light edits on the **raw** transcript; the
fixed rules ("fallback rules") run only when the AI cannot run or its answer is
rejected.

## Files

| File | What |
|---|---|
| `prompt_v4.json` | All-in-one AI-cleanup prompt, our own wording; 17 few-shots, none sharing a 5-word run with a test case (`score.py --overlap`) |
| `prompt_v4b.json` | v4 + one line about pre-punctuated input + 4 few-shots in Parakeet's punctuated form (commands between commas); the better of the two |
| `prompt_v4c.json` | Generated, do not edit: flow-core's prompt v4c (`crates/flow-core/src/cleanup/prompt_v4c.rs`), dumped by `ai-cleanup-bench --prompt-version v4c --dump-prompt prompt_v4c.json` so `score.py --overlap` covers it. Runs use the flow-core definition (`run_or.sh` with `PROMPT_VERSION=v4c`) |
| `runner/` | Probe binary (own workspace, flow-core unmodified), derived from `../formatting_eval_2026_10_24/runner`. Modes `prod` (shipped pipeline: rules + prompt v2 + guard + `llm_skip_clean`), `rules` (fallback rules only), `ai` (raw → LLM → strict and loosened guard → fallback rules on reject) |
| ~~`runner/src/loose_guard.rs`~~ | Loosened guard prototype (fillers, unit abbreviations allowed; names and a 20 % total-deletion cap enforced; number rules unchanged). **Promoted** (D30) to `flow_core::guard::check_loose` / `Strictness::Loose`; the runner calls that now |
| `score.py` | Per-category pass (final period ignored), guard reject rates, LLM p50/p90; `closeness()` is reused by `tools/replay/score.py` |
| `run.sh` / `run_all.sh` | One run / all phase-1 local runs. Memory gate: model size + 2000 MB available; `flock` + `choom 1000` + `nice` |
| `run_or.sh` | Phase 1d: one OpenRouter model through the product's `OpenRouterCleaner` (zdr + data_collection deny), prompt v4b; key from `~/.config/flow-clone/openrouter.key` into the env only; summed `usage.cost` over all `results/or_*` capped at `$CAP` (1.50) |
| `run_big.sh` | Qwen2.5-7B / Llama-3.1-8B rows; only after the home flow-server is stopped (not run in phase 1) |
| `fetch_llama8b.sh` | Llama-3.1-8B-Instruct Q4_K_M download with sha256 check |
| `results/quote_*` | 2026-10-31 "quote unquote" smart-scope check (9 cases, `quote_cases.jsonl`): Qwen2.5-1.5B, prompt v4c before (`quote_prompt_v4c_before.json`, the pre-change dump) vs after, plus the rules. Raw answer right 3/9 → 7/9; final text after guard + fallback 7/9 → 9/9 |
| `results/units_*` | 2026-11-01 clause aside / degrees / "A M" check: 12 synthetic cases (`units_cases.jsonl`), Qwen2.5-1.5B, flow-core before (integration `dbcffad`) vs after (`feat/format-units`), prompts v4c and v2, plus the rules. Final text right: rules 0/12 → 6/12, v4c 3/12 → 10/12, v2 4/12 → 10/12. `units_suite_*`: the suite cases (88-suite + 3 quote) before vs after, v4c strict, scored with `FMT_CASES` = the suite without the new cases: 75 % → 77 %; `_after_a` is the first (longer) prompt wording, 74 %. The first 3-5 calls after model load timed out on the shared box in every run (8 s limit) |
| `runner/src/bin/command_bench.rs` + `command_he_cases.jsonl` → `results/command_he_*` | 2026-11-01 Command Mode language check (`docs/findings/2026-11-01-command-mode-hebrew.md`): 14 synthetic cases (10 Hebrew edits, 3 Hebrew→English translations, 1 English control), Qwen2.5-1.5B. `--mode before` = pre-fix prompt, one call; `--mode after` = `flow_core::command::run`. Expected script kept: 5/14 → 14/14 (Hebrew edits 1/10 → 10/10). Build: `cargo build --release --bin command_bench --manifest-path runner/Cargo.toml` |
| `results/` | Raw rows (`*.jsonl`) and `/usr/bin/time -v` output (`*.time`, peak RSS). `v4_q15` was judged by an earlier loose guard (a reverted 2-word-run rule and a "USB-C" name bug); score `*_rejudged`, which re-judges the same LLM answers with the final guards (for the other runs it is identical) |

## Reproduce

```sh
B=tools/probes/prototypes/ai_cleanup_bench_2026_10_26
CARGO_TARGET_DIR=target/ai-bench cargo build --release --manifest-path $B/runner/Cargo.toml
MODELS=<gguf dir> FLOW_LLM_LIB=<libflow_llm.so> $B/run_all.sh
python3 $B/score.py rules=$B/results/rules.jsonl prod=$B/results/prod_q15.jsonl \
  v4=$B/results/v4_q15.jsonl:out_loose --fails v4
```

Prompt v4c on OpenRouter (`v4c_cases.jsonl` = `all_cases.jsonl` without the `regression` rows):

```sh
PROMPT_VERSION=v4c SPENT_GLOB='or_v4c*' CAP=0.50 CASES=$B/results/v4c_cases.jsonl \
  $B/run_or.sh or_v4c_l70_r2 meta-llama/llama-3.3-70b-instruct
python3 $B/score.py v4c=$B/results/or_v4c_l70_r2.jsonl:out_strict --fails v4c
```

`libflow_llm.so` = the release `flow-llm` cdylib of the same commit (no
flow-llm or `llama.rs` change since 2026-10-24).

The cache default is `~/.cache/flow-clone`; set CACHE=<dir> to reuse an existing cache.
