#!/usr/bin/env bash
# Every LLM configuration of the 2026-10-24 eval, one model loaded at a time.
# env: MODELS, FLOW_LLM_LIB (see run_llm.sh). Skips 3B below 4 GB available.
set -uo pipefail
cd "$(dirname "$0")"
Q15=qwen2.5-1.5b-instruct-q4_k_m.gguf; Q05=qwen2.5-0.5b-instruct-q4_k_m.gguf; Q3=qwen2.5-3b-instruct-q4_k_m.gguf
SH=sessions/shipped.json; ST=sessions/proposed_styles.json
run() { echo "== $1 $(date +%T) load $(cut -d' ' -f1 /proc/loadavg) avail $(free -m | awk '/Mem:/{print $7}')"; ./run_llm.sh "$@" 2>&1 | grep -v '^$'; }
run llm15_current_shipped $Q15 $SH
run llm15_current_styles $Q15 $ST
run llm15_v2_styles $Q15 $ST prompt_v2.json
run llm15_current_tuned $Q15 $ST - cases.jsonl --tuned-guard
run llm15_v2_tuned $Q15 $ST prompt_v2.json cases.jsonl --tuned-guard
run llm15_v2_tuned_rules $Q15 $ST prompt_v2.json cases_rules.jsonl --tuned-guard
run llm15_v3_tuned_rules $Q15 $ST prompt_v3.json cases_rules.jsonl --tuned-guard
run llm05_v2_tuned_rules $Q05 $ST prompt_v2.json cases_rules.jsonl --tuned-guard
if [ "$(free -m | awk '/Mem:/{print $7}')" -ge 4000 ]; then
  run llm3b_v2_tuned_rules $Q3 $ST prompt_v2.json cases_rules.jsonl --tuned-guard
else echo "== skip 3B: $(free -m | awk '/Mem:/{print $7}') MB available"; fi
run llm15_v2_tuned_rules_30s $Q15 sessions/proposed_styles_30s.json prompt_v2.json cases_rules.jsonl --tuned-guard
echo "== done $(date +%T)"
