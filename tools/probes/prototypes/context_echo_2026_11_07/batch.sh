#!/usr/bin/env bash
# usage: batch.sh BASE_BIN CHECK_BIN LLAMA_3B_GGUF QWEN_3B_GGUF FLOW_LLM_LIB
#   The four runs of the findings' headline rows: base and check, on the PC
#   path (Llama-3.2-3B, prompt v2) and the AI path (Qwen2.5-3B, prompt v5).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
base="$1"; check="$2"; llama="$3"; qwen="$4"; export FLOW_LLM_LIB="$5"
BIN="$base" MODEL="$llama" "$here/run.sh" base-pc3b local
BIN="$check" MODEL="$llama" "$here/run.sh" check-pc3b local
BIN="$base" MODEL="$qwen" "$here/run.sh" base-ai-qwen3b cloud
BIN="$check" MODEL="$qwen" "$here/run.sh" check-ai-qwen3b cloud
