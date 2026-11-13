# Spoken line commands and spoken marks under prompt v5, 2026-11-12 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-12-newline-punct-prompt.md`. Whatever prompt it
> produces lives in murmur-core (`crates/murmur-core/src/cleanup/prompt_v6.rs`,
> D89).

Prompt v5 (D60) made the 3B model turn spoken "new line" / "new paragraph"
into a list or a blank line and drop some spoken marks. This probe measures
prompt-only variants of v5 (no pre-AI rules) on Qwen2.5 3B and 1.5B.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl` | Set 1: 25 made-up cases in the formatting-eval shape: newline (10), spoken_punct (9), list (6). Written before any variant, but scored on every variant while the prompt was tuned, so it is **not** a held-out set any more: read its numbers as a second tuning set |
| `gen_cases.py 2` -> `cases2.jsonl` | Set 2: 24 made-up cases written after the prompt was settled, with other shapes (lengths, domains, command mixes): newline (10), spoken_punct (10), list (4). Scored only on the final prompt (v6) and the 1.3.0 v5 baseline, once |
| `variants.py` | First pass: builds `--override` prompts from a `--dump-prompt` of today's v5: `a` marks rule first, `b` no blank line asked before lists (the core adds it), `c` reworded line-command / spoken-mark rules, `d` three made-up few-shots, `e` v4c's layout shots moved to the end, `i` / `L` list rule opens with / is replaced by a layout order, `s` bake-sale shot announces its list, `o` v4c rule order; letters combine (see the docstring). `cde` was the first pass |
| `variants2.py` | Second pass (1.4.0): reorders the shots of the first-pass prompt (`cde`, from a `--dump-prompt`): `X` correction shots last, `A` corrections and layout alternate, `Y`, `V` (written, not run), and `N` = `X` with its three made-up layout shots replaced by shapes unlike any suite case (shipped as prompt v6, D89); `Nr` / `Nt` trim `N` for prompt size (not shipped). Checks that no shot shares a 5-word run with a suite, set 1, set 2 or list/correction case |
| `budget_probe.py` | Measures the longest v6 prompt in tokens on the local engine (worst-case user message: 20 DICTIONARY terms, full cursor windows, casual STYLE line) from two over-long cases the engine refuses before any prefill |
| `run.sh` | One run of the context-echo `parity-clean` runner (`../context_echo_2026_11_07/runner`, real `MurmurCore` commit path, cloud mode) on `suite`, `held` (set 1), `held2` (set 2), `syn` (lists/corrections) or `main` (private parity set) |

## Run

    CARGO_TARGET_DIR=... cargo build --release --manifest-path ../context_echo_2026_11_07/runner/Cargo.toml
    BIN=.../parity-clean LIB=.../libmurmur_llm.so MODELS=<gguf dir> SUITE=<113 cases> OUT=<dir> \
        DUMP=v5.json ./run.sh held 3b base
    python3 variants.py v5.json VARIANTS/
    ./run.sh suite 3b abc VARIANTS/abc.json
    FMT_CASES=cases.jsonl python3 ../ai_cleanup_bench_2026_10_26/score.py base=OUT/held__3b__base.jsonl:text

Optional env: `SUFFIX` appends a line to the user message (variant `U`),
`HOLDS_LOCK=1` skips the build-lock `flock` when the caller already holds it.
Do not edit `run.sh` while runs are in flight (bash reads it as it goes).

The suite is scored with `../ai_cleanup_bench_2026_10_26/score.py`
(`FMT_CASES` unset), the lists/corrections set with
`../lists_corrections_2026_11_06/score_cases.py`.
