# AI cleanup benchmark, 2026-10-26 (dated prototype)

> **Status:** dated probe, research only. Not a product tool; nothing here is
> wired into flow-core. Findings: `docs/findings/2026-10-26-ai-cleanup-benchmark.md`.
> Promote to a permanent path only together with the AI-cleanup implementation.

Benchmarks the owner's 2026-10-26 design (GLOSSARY "AI cleanup"): one LLM pass
does all formatting on the **raw** transcript; the
fixed rules ("fallback rules") run only when the AI cannot run or its answer is
rejected.

## Files

| File | What |
|---|---|
| `prompt_v4.json` | All-in-one AI-cleanup prompt, our own wording; 17 few-shots, none sharing a 5-word run with a test case (`score.py --overlap`) |
| `prompt_v4b.json` | v4 + one line about pre-punctuated input + 4 few-shots in Parakeet's punctuated form (commands between commas); the better of the two |
| `gen_wispr_cases.py` → `wispr_cases.jsonl` | 14 Wispr-target cases: the 10 A/B pairs of `../wispr_ab_2026_10_25/` (input `spoken`, target `wispr`) + the owner's 4 examples (Wispr outputs; raw inputs synthesised, see the script) |
| `runner/` | Probe binary (own workspace, flow-core unmodified), derived from `../formatting_eval_2026_10_24/runner`. Modes `prod` (shipped pipeline: rules + prompt v2 + guard + `llm_skip_clean`), `rules` (fallback rules only), `ai` (raw → LLM → strict and loosened guard → fallback rules on reject) |
| `runner/src/loose_guard.rs` | Loosened guard prototype: a copy of `flow_core::guard` with each change marked `LOOSE:` (fillers, unit abbreviations, 2-word trims allowed; names and a 20 % total-deletion cap enforced; number rules unchanged) |
| `score.py` | Per-category pass (final period ignored), target-closeness on the 14, guard reject rates, LLM p50/p90 |
| `run.sh` / `run_all.sh` | One run / all phase-1 local runs. Memory gate: model size + 2000 MB available; `flock` + `choom 1000` + `nice` |
| `run_big.sh` | Qwen2.5-7B / Llama-3.1-8B rows; only after the home flow-server is stopped (not run in phase 1) |
| `fetch_llama8b.sh` | Llama-3.1-8B-Instruct Q4_K_M download with sha256 check |
| `results/` | Raw rows (`*.jsonl`) and `/usr/bin/time -v` output (`*.time`, peak RSS). `v4_q15` was judged by an earlier loose guard (a reverted 2-word-run rule and a "USB-C" name bug); score `*_rejudged`, which re-judges the same LLM answers with the final guards (for the other runs it is identical) |

## Reproduce

```sh
B=tools/probes/prototypes/ai_cleanup_bench_2026_10_26
CARGO_TARGET_DIR=target/ai-bench cargo build --release --manifest-path $B/runner/Cargo.toml
MODELS=<gguf dir> FLOW_LLM_LIB=<libflow_llm.so> $B/run_all.sh
python3 $B/score.py rules=$B/results/rules.jsonl prod=$B/results/prod_q15.jsonl \
  v4=$B/results/v4_q15.jsonl:out_loose --fails v4
```

`libflow_llm.so` = the release `flow-llm` cdylib of the same commit (no
flow-llm or `llama.rs` change since 2026-10-24).
