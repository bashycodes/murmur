# Spoken numbers on the on-device path, 2026-11-06 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-06-spoken-numbers.md` (D63). The product change
> lives in murmur-core (`postprocess::spoken::has_spoken_numbers`,
> `cleanup::llm_can_help`, prompt v2); nothing here is wired into the
> product.

Measures whether the on-device cleanup (fallback rules, then prompt v2 on
Qwen2.5 1.5B, as on the phone) writes spoken times, dates, money, counts and
digit runs as digits once the "already clean" skip lets it run on them, and
whether number idioms ("one of those", "a second ago") stay words.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl`, `refs.json` | 61 made-up cases: T times, M money, O ordinals/dates, P percent, D degrees, C counts, S digit runs, N idioms (10 alone, 7 next to a number the rules leave so the LLM runs on them, 4 non-date ordinals, 6 lone "two" / "three"), K tidy lines with no number |
| `he_cases.jsonl` | 8 made-up Hebrew lines (6 long enough to be cleaned, 2 short): before/after outputs show what the shared v2 prefix change does to Hebrew |
| `score_cases.py` | Passes per group, share of rows the LLM ran on, LLM p50/p95. `--refs` takes any parity reference file (only clips with checks count) |
| `overlap.py` | No few-shot of a `--dump-prompt` dump shares a 5-word run with a case file |
| `session-phone.json` | The phone's session (light, casual styles, 3 s LLM budget, skip on) |
| `runner/` | `parity-clean`, the lists/corrections copy of the parity runner (`--override`, `--dump-prompt`) plus `--language CODE` (the language the fake ASR reports, default en). Own workspace, uses this checkout's murmur-core |

## Run

    python3 gen_cases.py
    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml
    parity-clean --mode local --cases cases.jsonl --session session-phone.json \
        --llm qwen2.5-1.5b-instruct-q4_k_m.gguf --llm-lib libmurmur_llm.so [--override V.json] > out.jsonl
    python3 score_cases.py out.jsonl --fails

"Before" rows come from the same runner built against the base commit's
murmur-core (point `runner/Cargo.toml` at it). Real-speech sets stay outside the
repo; pass their case and reference files as arguments.
