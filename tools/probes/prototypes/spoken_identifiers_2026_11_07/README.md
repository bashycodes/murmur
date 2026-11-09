# Spoken identifiers in the AI cleanup, 2026-11-07 (dated prototype)

> **Status:** dated probe, research only; evidence for D67
> (`docs/findings/2026-11-07-spoken-identifiers.md`). Only the
> dictionary part shipped (`cleanup::terms` names entries with digits when
> the number is said as words). The prompt rule and few-shots measured
> here (variants `rule`, `shots`, `shots2`, `lite`) were rejected: the
> held-out half was flat. Nothing here is wired into the product.

Measures whether the cleanup prompt writes spoken code names in code form
("retry limit in caps with underscores" -> RETRY_LIMIT, "utf eight" ->
UTF-8) on every AI path without touching prose that uses the same words,
and whether a dictionary entry with digits is named when its number was
said as words.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl`, `refs.json` | 38 made-up cases: I spoken forms (12), C code names whose kind sets the form, no form said (6), V technical names with a number (8), N prose with the same words (12). Every even-numbered case is "held" (not looked at while tuning) |
| `variants.py` | Writes `--override` prompt variants from `--dump-prompt` dumps of the base v5 / v2 prompts: `rule`, `shots`, `shots2`, `lite`; checks the shots share no 4-word run with the cases (the cue "in caps with underscores" aside). The candidate (rejected) was `shots2` for v2 and `lite` for v5 |
| `score_cases.py` | Passes per group and on the held half, rows the LLM ran on, mean prompt characters, LLM p50 / p95. `--refs` takes any parity reference file |
| `overlap.py` | Privacy check: no added line of the branch shares a 4-word run with the private reference files (`git diff BASE -U0 \| overlap.py REFS...`) |

The runner is `../dictionary_2026_11_07/runner/` unchanged (`--override`,
`--dump-prompt`, `--dictionary`); build it against this checkout.

## Run

    python3 gen_cases.py
    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path ../dictionary_2026_11_07/runner/Cargo.toml
    parity-clean --mode local|server|cloud --cases cases.jsonl --session SESSION.json \
        --llm MODEL.gguf --llm-lib libflow_llm.so [--timeout-ms MS] [--override V.json] > out.jsonl
    python3 score_cases.py out.jsonl --fails

"Before" rows come from the same runner built against the base commit's
flow-core. Real-speech sets stay outside the repo; pass their case and
reference files as arguments.
