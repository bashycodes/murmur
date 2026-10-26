#!/usr/bin/env bash
# usage: run.sh LABEL MODE [MODEL.gguf|- [PROMPT.json|- [FROM.jsonl]]]
#   MODE prod|ai|rules (see runner/src/main.rs). env: MODELS (gguf dir),
#   FLOW_LLM_LIB (libflow_llm.so), BENCH_BIN (default: the release build under
#   the repo's target/ai-bench).
# Runs the 102 cases (88 formatting-eval + 14 Wispr) with 4 LLM threads, as a
# low-priority, earlyoom-first process under the shared build lock, and only
# when MemAvailable >= model size + 2000 MB (the home flow-server must not be
# the one earlyoom picks). Peak RSS goes to results/LABEL.time.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="${BENCH_BIN:-$here/../../../../target/ai-bench/release/ai-cleanup-bench}"
label="$1"; mode="$2"; model="${3:-}"; prompt="${4:-}"
cat "$here/../formatting_eval_2026_10_24/cases.jsonl" "$here/wispr_cases.jsonl" > "$here/results/all_cases.jsonl"
args=(--mode "$mode" --cases "$here/results/all_cases.jsonl" --session "$here/sessions/prod.json" --threads 4 --timeout-ms 8000)
if [[ -n "$model" && "$model" != - ]]; then
  need=$(( $(stat -c %s "$MODELS/$model") / 1048576 + 2000 ))
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  if (( avail < need )); then echo "skip $label: $avail MB available < $need MB" >&2; exit 3; fi
  args+=(--llm "$MODELS/$model" --llm-lib "$FLOW_LLM_LIB")
fi
[[ -n "$prompt" && "$prompt" != - ]] && args+=(--prompt "$here/$prompt")
[[ -n "${5:-}" ]] && args+=(--from "$here/$5")
echo "== $label $(date +%T) load $(cut -d' ' -f1 /proc/loadavg) avail $(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo) MB" >&2
flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
  /usr/bin/time -v -o "$here/results/$label.time" "$bin" "${args[@]}" > "$here/results/$label.jsonl"
grep -E 'Maximum resident|Elapsed' "$here/results/$label.time" >&2
