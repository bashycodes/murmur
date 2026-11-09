# Context echo in cleanup, 2026-11-07 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-07-context-echo.md` (D71). The product change is
> `murmur_core::cleanup::context_echo`; nothing here is wired into the product.

Measures whether a cleanup answer repeats the text already in the field
(the text before / after the cursor), and whether dictated repeats survive.

## Files

| File | What |
|---|---|
| `cases.jsonl` | 26 made-up rows. Each has `before` / `after` (the field), `input` (the transcript) and `group`: E (18) context echo candidates, N (8) dictations that repeat context words themselves (`keep` = the phrase that must stay) |
| `session-pc.json` | The PC's cleanup session (Windows, level high, casual), 30 s LLM budget so load doesn't time rows out |
| `runner/` | `parity-clean`, the Hebrew runner plus per-case `before` / `after` / `selected` / `app_id`. Own workspace, uses this checkout's murmur-core |
| `variants/` | Prompt overrides tried (`--override`): A system rule, B rule + two context few-shots |
| `run.sh`, `score.py` | One pass + score: E rows without echo, N rows kept, LLM p50 |
| `batch.sh` | The headline rows: base and check on the PC path (Llama-3.2-3B, v2) and the AI path (Qwen2.5-3B, v5) |
| `results/` | The runs in the findings table. `base-*` / `check-*` are from the final cases and code. Variants A-F ran before E12's address was changed to a documentation range (192.0.2.x); their files were relabelled to match, otherwise unchanged. E ran with the first version of the check |

## Run

    CARGO_TARGET_DIR=... cargo build --release --offline -j 2 --manifest-path runner/Cargo.toml
    BIN=.../parity-clean MODEL=Llama-3.2-3B-Instruct-Q4_K_M.gguf MURMUR_LLM_LIB=libmurmur_llm.so \
        ./run.sh LABEL local [--override variants/X.json] [--user-suffix TEXT]

"Base" rows come from the runner built against the base commit's murmur-core.
`--user-suffix` adds a line after the transcript of every prompt; it stood
in for a per-dictation reminder line (variants C-F).
