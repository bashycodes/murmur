#!/usr/bin/env bash
# One model through the four synthetic suites in `server` mode (murmur-server's
# cleanup: the shipped prompt v2, never skipped), quality only: a 120 s
# budget, so on this CPU no answer is cut by time.
#
#   run.sh TAG MODEL.gguf          -> results/TAG-{bench,numbers,he,ho}.jsonl
#
# env: BIN (gpu-model-clean), LIB (libmurmur_llm.so), THREADS (default 4).
# Shared-box rules: refuses to start when MemAvailable < model size +
# 2000 MB; the four suites run under one hold of the build lock (other
# jobs' builds between suites cost hours of queueing), at low priority.
set -uo pipefail
if [ -z "${RUN_LOCKED:-}" ]; then
  RUN_LOCKED=1 exec flock "$HOME/.cache/flow-clone-build.lock" "$0" "$@"
fi
here="$(cd "$(dirname "$0")" && pwd)"
TAG=$1; MODEL=$2
: "${BIN:?}" "${LIB:?}"
THREADS=${THREADS:-4}
mkdir -p "$here/results"
suite() { # name cases session language
  local o="$here/results/$TAG-$1.jsonl"
  if [ -s "$o" ]; then echo "have $TAG-$1" >&2; return; fi
  local avail need
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  need=$(( $(stat -c %s "$MODEL") / 1048576 + 2000 ))
  if (( avail < need )); then echo "skip $TAG-$1: $avail MB < $need" >&2; return 1; fi
  echo "== $TAG-$1 $(date +%T) load $(cut -d' ' -f1 /proc/loadavg) avail $avail MB" >&2
  timeout 7200 choom -n 1000 -- nice -n 10 \
    "$BIN" --cases "$2" --mode server --llm "$MODEL" --llm-lib "$LIB" \
    --session "$3" --threads "$THREADS" --language "$4" --timeout-ms 120000 \
    > "$o.tmp" && mv "$o.tmp" "$o"
}
P="$here/.."
suite bench "$here/bench_cases.jsonl" "$here/session-formal.json" en
suite numbers "$P/numbers_2026_11_06/cases.jsonl" "$here/session-casual.json" en
suite he "$P/hebrew_stays_hebrew_2026_11_07/he_cases.jsonl" "$here/session-casual.json" he
suite ho "$P/hebrew_stays_hebrew_2026_11_07/he_holdout.jsonl" "$here/session-casual.json" he
echo "done $TAG" >&2
