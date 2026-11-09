# Spoken lists and self-corrections, 2026-11-06 (dated prototype)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-11-06-lists-corrections-prompt.md`. The prompt it
> produced lives in flow-core (`crates/flow-core/src/cleanup/prompt_v5.rs`);
> nothing here is wired into the product.

Measures whether the AI-cleanup prompt alone (no pre-AI rules) can turn a
dictated list into a bulleted or numbered list with its lead-in, and keep only
the correction of a spoken self-correction ("at 5, no, 6" -> "at 6"), on
Qwen2.5 3B and 1.5B.

## Files

| File | What |
|---|---|
| `gen_cases.py` -> `cases.jsonl`, `refs.json` | 39 made-up cases: L lists (12), C corrections (9), X both (2), N negatives (16: "no", "not", "first", "sorry", "actually", items inside a running sentence) |
| `score_cases.py` | Pass rate per group from a runner output (`--fails` lists the failing checks) |
| `variants.py` | Builds prompt variants (`--override` JSON) from a dump of prompt v4c: rules only, few-shots only, both, wording and rule-order changes, and a short diagnostic prompt. Checks no example shares a 5-word run with a case |

The runner is the parity eval's `parity-clean` (real `FlowCore` commit
path) with an `--override` flag that swaps the system prompt and few-shots,
kept with the private eval outputs. Variant `g_order` is what became v5.

## Run

    python3 gen_cases.py
    python3 variants.py PROMPT_V4C_DUMP.json OUT_DIR
    parity-clean --mode cloud --cases cases.jsonl --session SESSION.json \
        --llm MODEL.gguf --llm-lib libflow_llm.so [--override OUT_DIR/g_order.json] > out.jsonl
    python3 score_cases.py out.jsonl
