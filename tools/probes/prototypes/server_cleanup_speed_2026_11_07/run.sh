#!/usr/bin/env bash
# Replay one case file through the runner in `server` mode (flow-server's
# cleanup: prompt v2, never skipped) and write OUT_DIR/TAG.jsonl.
#
#   run.sh OUT_DIR TAG [extra runner args...]
#
# env (all required unless defaulted):
#   CASES    case file ({id, input, app_category} per line)
#   SESSION  session JSON (llm_timeout_ms, per-char budget, dictionary, ...)
#   MODEL    GGUF;  LIB  libflow_llm.so;  BIN  server-speed-clean
#   THREADS  default 4 (the home server's [model] threads)
#
# Shared-box rules: refuses when MemAvailable < model size + 2000 MB, runs
# under the build lock at low priority. Real-speech cases and sessions stay
# outside the repo.
set -uo pipefail
OUT=$1; TAG=$2; shift 2
: "${CASES:?}" "${SESSION:?}" "${MODEL:?}" "${LIB:?}" "${BIN:?}"
THREADS=${THREADS:-4}
mkdir -p "$OUT"
o=$OUT/$TAG.jsonl
avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
need=$(( $(stat -c %s "$MODEL") / 1048576 + 2000 ))
if (( avail < need )); then echo "skip $TAG: $avail MB < $need" >&2; exit 1; fi
flock "$HOME/.cache/flow-clone-build.lock" timeout 1500 choom -n 1000 -- nice -n 10 \
  "$BIN" --cases "$CASES" --mode server --llm "$MODEL" --llm-lib "$LIB" \
  --session "$SESSION" --threads "$THREADS" "$@" > "$o.tmp" && mv "$o.tmp" "$o"
echo "done $TAG" >&2
