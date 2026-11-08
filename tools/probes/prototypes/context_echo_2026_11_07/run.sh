#!/usr/bin/env bash
# usage: run.sh LABEL MODE [runner args...]
#   One runner pass over cases.jsonl with the PC session into
#   results/LABEL.jsonl, then the score. env: BIN (the runner binary), MODEL
#   (gguf), FLOW_LLM_LIB (libflow_llm.so). Runs under the shared build lock,
#   low priority, only with >= 4000 MB available.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
label="$1"; mode="$2"; shift 2
avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
if (( avail < 4000 )); then echo "skip $label: $avail MB available" >&2; exit 3; fi
mkdir -p "$here/results"
flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
  "$BIN" --mode "$mode" --cases "$here/cases.jsonl" --session "$here/session-pc.json" \
  --llm "$MODEL" --llm-lib "$FLOW_LLM_LIB" --threads 4 "$@" \
  > "$here/results/$label.jsonl" 2> "$here/results/$label.log"
echo "== $label"
python3 -I "$here/score.py" "$here/cases.jsonl" "$here/results/$label.jsonl" --fails
