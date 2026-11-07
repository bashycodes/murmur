#!/usr/bin/env bash
# usage: run.sh LABEL CASES.jsonl LANG [runner args...]
#   One runner pass (local mode = phone pipeline, Qwen2.5-1.5B, phone session)
#   into results/LABEL.jsonl. env: BIN (the runner binary), MODEL (the 1.5B
#   gguf), FLOW_LLM_LIB (libflow_llm.so). Runs under the shared build lock,
#   low priority and earlyoom-first, only with >= 4000 MB available.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
label="$1"; cases="$2"; lang="$3"; shift 3
avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
if (( avail < 4000 )); then echo "skip $label: $avail MB available" >&2; exit 3; fi
echo "== $label $(date +%T) load $(cut -d' ' -f1 /proc/loadavg) avail $avail MB" >&2
flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
  "$BIN" --mode local --cases "$cases" --session "$here/session-phone.json" \
  --llm "$MODEL" --llm-lib "$FLOW_LLM_LIB" --threads 4 --language "$lang" "$@" \
  > "$here/results/$label.jsonl"
