# Dictionary words in the AI cleanup, 2026-11-07 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-07-dictionary-in-cleanup.md` (D64). The product
> change lives in murmur-core (`cleanup::terms`, prompts v2 / v5, the rules'
> stutter step); nothing here is wired into the product.

Measures whether the cleanup prompt, given the dictionary terms a dictation
plausibly misspells, corrects near-miss spellings to the entry on every AI path, leaves
ordinary look-alike words alone, and what a large dictionary costs in prompt
length and latency.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl`, `refs.json`, `dict-*.json` | 34 made-up cases: P near spellings of a made-up user dictionary (12), B near spellings of public product names that are not in the dictionary (8; a built-in list of them was tried and removed), N ordinary words that resemble an entry and must stay (12), K tidy lines with no entry near (2). `dict-small.json` is that 12-entry dictionary; `dict-20.json` / `dict-200.json` pad it with made-up names for the prompt-size runs |
| `score_cases.py` | Passes per group, rows the LLM ran on, rows whose prompt named a term, mean prompt characters, LLM p50 / p95 |
| `overlap.py` | Privacy check: no added line of the branch shares a 4-word run with the private reference files (`git diff BASE -U0 \| overlap.py REFS...`) |
| `runner/` | `parity-clean`, the numbers copy of the parity runner plus `--dictionary DICT.json` and the prompt's DICTIONARY line in each output row. Own workspace, uses this checkout's murmur-core |

## Run

    python3 gen_cases.py
    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml
    parity-clean --mode local|server|cloud --cases cases.jsonl --session SESSION.json \
        --dictionary dict-small.json --llm MODEL.gguf --llm-lib libmurmur_llm.so [--timeout-ms MS] > out.jsonl
    python3 score_cases.py out.jsonl --fails

"Before" rows come from the same runner built against the base commit's
murmur-core, with the dictionary in the session JSON (the base runner has no
`--dictionary`). Real-speech sets and the owner's dictionary stay outside the
repo; pass their case, session and reference files as arguments.
