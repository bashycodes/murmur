#!/usr/bin/env bash
# The 2026-11-12 control that never ran: Qwen2.5-14B on this box's CPU
# through the real murmur-llm plugin (not llama-server), the five Hebrew dev
# lines it garbled (control_he5.jsonl), drafting off and on. Same runner,
# plugin and arguments as gpu_cleanup_model_2026_11_12/control.sh, output
# here as results/cpu-qwen2.5-14b-he5-{nodraft,draft4}.jsonl.
#
# env: BIN (gpu-model-clean), LIB (libmurmur_llm.so), MODEL (14B GGUF).
set -uo pipefail
if [ -z "${RUN_LOCKED:-}" ]; then
  RUN_LOCKED=1 exec flock "$HOME/.cache/murmur-build.lock" "$0" "$@"
fi
here="$(cd "$(dirname "$0")" && pwd)"
O="$here/../gpu_cleanup_model_2026_11_12"
: "${BIN:?}" "${LIB:?}" "${MODEL:?}"
for d in 0 4; do
  name=$([ "$d" = 0 ] && echo nodraft || echo draft4)
  o="$here/results/cpu-qwen2.5-14b-he5-$name.jsonl"
  [ -s "$o" ] && continue
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  need=$(( $(stat -c %s "$MODEL") / 1048576 + 2000 ))
  if (( avail < need )); then echo "skip $name: $avail MB < $need" >&2; exit 1; fi
  MURMUR_LLM_DRAFT=$d timeout 3600 choom -n 1000 -- nice -n 10 \
    "$BIN" --cases "$O/control_he5.jsonl" --mode server --llm "$MODEL" --llm-lib "$LIB" \
    --session "$O/session-casual.json" --threads "${THREADS:-4}" --language he --timeout-ms 120000 \
    > "$o.tmp" && mv "$o.tmp" "$o"
done
