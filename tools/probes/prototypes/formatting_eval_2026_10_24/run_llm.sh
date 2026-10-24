#!/usr/bin/env bash
# usage: run_llm.sh LABEL MODEL.gguf SESSION.json [PROMPT.json|-] [CASES.jsonl] [--tuned-guard]
# Runs the eval through flow-core + the flow-llm plugin (threads 4). Paths
# come from env: MODELS (gguf dir), FLOW_LLM_LIB (libflow_llm.so).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/../../../../target/formatting-eval/release/formatting-eval"
label="$1"; model="$2"; session="$3"; prompt="${4:-}"; cases="${5:-$here/cases.jsonl}"; extra="${6:-}"
args=(--cases "$cases" --session "$session" --llm "$MODELS/$model" --llm-lib "$FLOW_LLM_LIB" --threads 4)
[[ -n "$prompt" && "$prompt" != - ]] && args+=(--prompt "$prompt")
[[ -n "$extra" ]] && args+=("$extra")
mkdir -p "$here/results"
nice "$bin" "${args[@]}" > "$here/results/$label.jsonl"
