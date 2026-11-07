# Dictionary words in the AI cleanup, 2026-11-07 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-07-dictionary-in-cleanup.md` (D64). The product
> change lives in flow-core (`cleanup::terms`, `dictionary::BUILTIN_TERMS`,
> prompts v2 / v5); nothing here is wired into the product.

Measures whether the cleanup prompt, given the dictionary terms a dictation
plausibly misspells (the user's plus a small built-in list of product
names), corrects near-miss spellings to the entry on every AI path, leaves
ordinary look-alike words alone, and what a large dictionary costs in prompt
length and latency.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl`, `refs.json`, `dict-*.json` | 34 made-up cases: P near spellings of a made-up user dictionary (12), B near spellings of built-in names (8), N ordinary words that resemble an entry and must stay (12), K tidy lines with no entry near (2). `dict-small.json` is that 12-entry dictionary; `dict-20.json` / `dict-200.json` pad it with made-up names for the prompt-size runs |
| `score_cases.py` | Passes per group, rows the LLM ran on, rows whose prompt named a term, mean prompt characters, LLM p50 / p95 |
| `overlap.py` | Privacy check: no added line of the branch shares a 4-word run with the private reference files (`git diff BASE -U0 \| overlap.py REFS...`) |
| `runner/` | `wispr-parity-clean`, the numbers copy of the parity runner plus `--dictionary DICT.json`, `--no-builtin-terms` and the prompt's DICTIONARY line in each output row. Own workspace, uses this checkout's flow-core |

## Run

    python3 gen_cases.py
    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml
    wispr-parity-clean --mode local|server|cloud --cases cases.jsonl --session SESSION.json \
        --dictionary dict-small.json --llm MODEL.gguf --llm-lib libflow_llm.so [--timeout-ms MS] > out.jsonl
    python3 score_cases.py out.jsonl --fails

"Before" rows come from the same runner built against the base commit's
flow-core, with the dictionary in the session JSON (the base runner has no
`--dictionary`). Real-speech sets and the owner's dictionary stay outside the
repo; pass their case, session and reference files as arguments.
