# Hebrew stays Hebrew on the on-device cleanup, 2026-11-07 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-07-hebrew-stays-hebrew.md` (D65). The product change
> lives in murmur-core (`cleanup::HEBREW_EXAMPLE`, the script check in
> `cleanup::run_metered`); nothing here is wired into the product.

Measures whether the on-device cleanup (fallback rules, then prompt v2 on
Qwen2.5 1.5B, as on the phone) keeps a long Hebrew dictation in Hebrew,
which prompt changes fix it, and that English dictations come out the same.

## Files

| File | What |
|---|---|
| `he_cases.jsonl` | 28 made-up Hebrew lines: 22 long (L, cleaned), 4 medium (M), 2 short (S). L01-L06 are the numbers probe's Hebrew lines. Used to pick the prompt variant |
| `he_holdout.jsonl` | 14 more made-up long Hebrew lines, written after the variant was picked |
| `variants/shot_he.json` | The Hebrew few-shot as an `--extra-shots` turn (what `cleanup::HEBREW_EXAMPLE` renders to) |
| `score_he.py` | Rows the LLM ran on; Hebrew model answers (`raw`) and final texts; words of the input kept; LLM p50/p95 |
| `run.sh` | One locked, low-priority runner pass into `results/LABEL.jsonl` (needs >= 4000 MB available) |
| `batch.sh dev\|holdout\|product\|english\|guard` | The rows of the findings doc. `dev` / `holdout`: prompt variants on the base commit's runner; `product`: the shipped prompt and check (`TAG=before\|after`, Hebrew, then `english` = the numbers probe's 61 English cases); `guard`: this branch's runner with the base prompt (`results/prompt_he_before.json`, dumped by `run.sh he_before he_cases.jsonl he --dump-prompt ...` on the base runner), so only the script check acts |
| `session-phone.json` | The phone's session (light, casual styles, 3 s LLM budget, skip on) |
| `runner/` | `parity-clean`, the numbers probe's runner plus `--user-suffix TEXT`, `--extra-shots FILE` and a `raw` field (the model's answer before the core's post-processing). Own workspace, uses this checkout's murmur-core |

## Run

    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml
    BIN=.../parity-clean MODEL=qwen2.5-1.5b-instruct-q4_k_m.gguf MURMUR_LLM_LIB=libmurmur_llm.so \
        ./batch.sh dev
    python3 score_he.py he_cases.jsonl results/he_*_t20.jsonl

"Before" rows come from the runner built against the base commit's murmur-core
(integration `12fa4ad`). `_t20` rows use a 20 s budget: on this shared 4-core
box a long Hebrew line takes 4-6 s on the 1.5B, so with the phone's 3 s most
lines time out and the row would show the box's speed, not the model's
language.
