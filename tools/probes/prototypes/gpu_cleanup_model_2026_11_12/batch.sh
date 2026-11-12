#!/usr/bin/env bash
# The rows of docs/model-benchmarks.md "GPU cleanup model": the current
# default (1.5B) as the same-harness baseline, then the three candidates,
# smallest first. env as run.sh plus M (models dir, default
# ~/.cache/murmur/models) and Q15 (the 1.5B GGUF). MURMUR_LLM_DRAFT=4 as
# murmur-server sets it (llm_draft); it changes speed, not the answer.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
M=${M:-$HOME/.cache/murmur/models}
export MURMUR_LLM_DRAFT=${MURMUR_LLM_DRAFT:-4}
[ -n "${Q15:-}" ] && "$here/run.sh" qwen2.5-1.5b "$Q15"
"$here/run.sh" qwen2.5-7b "$M/Qwen2.5-7B-Instruct-Q4_K_M.gguf"
"$here/run.sh" gemma-3-12b "$M/google_gemma-3-12b-it-Q4_K_M.gguf"
"$here/run.sh" qwen2.5-14b "$M/Qwen2.5-14B-Instruct-Q4_K_M.gguf"
