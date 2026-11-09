#!/usr/bin/env bash
# Phase 1 local runs (2026-10-26), one model loaded at a time. env: MODELS,
# MURMUR_LLM_LIB. A label whose results file exists is skipped (captures are not
# overwritten). 7B/8B are NOT run here: they need the home murmur-server stopped
# for RAM, which the orchestrator does separately.
set -uo pipefail
cd "$(dirname "$0")"
Q15=qwen2.5-1.5b-instruct-q4_k_m.gguf; Q3=qwen2.5-3b-instruct-q4_k_m.gguf
run() { if [[ -s results/$1.jsonl ]]; then echo "== have $1"; else ./run.sh "$@"; fi; }
run rules rules
run prod_q15 prod $Q15
run v4_q15 ai $Q15 prompt_v4.json
run v2raw_q15 ai $Q15
run prod_q3 prod $Q3
run v4_q3 ai $Q3 prompt_v4.json
run v4b_q15 ai $Q15 prompt_v4b.json
run v4b_q3 ai $Q3 prompt_v4b.json
# Guard changes after a run: re-judge the stored answers (no model).
for l in v4_q15 v2raw_q15 v4_q3; do
  [[ -s results/$l.jsonl ]] && ./run.sh ${l}_rejudged regard - - results/$l.jsonl
done
echo "== done $(date +%T)"
