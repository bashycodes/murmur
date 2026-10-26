#!/usr/bin/env bash
# 7B/8B rows of the 2026-10-26 bench. NOT run in phase 1: each model needs
# ~5 GB + 2 GB headroom, so the home flow-server must be stopped first (the
# orchestrator's job; this script never touches it). run.sh still refuses to
# start when MemAvailable < model size + 2000 MB. env: MODELS, FLOW_LLM_LIB.
# Same prompts and cases as run_all.sh; existing results are kept.
set -uo pipefail
cd "$(dirname "$0")"
Q7=Qwen2.5-7B-Instruct-Q4_K_M.gguf; L8=Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf
run() { if [[ -s results/$1.jsonl ]]; then echo "== have $1"; else ./run.sh "$@"; fi; }
run prod_q7 prod $Q7
run v4b_q7 ai $Q7 prompt_v4b.json
run prod_l8 prod $L8
run v4b_l8 ai $L8 prompt_v4b.json
echo "== done $(date +%T)"
