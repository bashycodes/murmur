#!/usr/bin/env bash
# Control: is Qwen2.5-14B's mixed-script Hebrew (Chinese, Cyrillic, Turkish
# tokens inside Hebrew lines) the model or prompt-lookup drafting? Five of
# the Hebrew dev lines it garbled (control_he5.jsonl), drafting off
# (MURMUR_LLM_DRAFT=0), otherwise as run.sh.
#
#   control.sh TAG MODEL.gguf      -> results/TAG-he5-nodraft.jsonl
set -uo pipefail
if [ -z "${RUN_LOCKED:-}" ]; then
  RUN_LOCKED=1 exec flock "$HOME/.cache/flow-clone-build.lock" "$0" "$@"
fi
here="$(cd "$(dirname "$0")" && pwd)"
TAG=$1; MODEL=$2
: "${BIN:?}" "${LIB:?}"
avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
need=$(( $(stat -c %s "$MODEL") / 1048576 + 2000 ))
if (( avail < need )); then echo "skip $TAG: $avail MB < $need" >&2; exit 1; fi
MURMUR_LLM_DRAFT=0 timeout 3600 choom -n 1000 -- nice -n 10 \
  "$BIN" --cases "$here/control_he5.jsonl" --mode server --llm "$MODEL" --llm-lib "$LIB" \
  --session "$here/session-casual.json" --threads "${THREADS:-4}" --language he --timeout-ms 120000 \
  > "$here/results/$TAG-he5-nodraft.jsonl"
