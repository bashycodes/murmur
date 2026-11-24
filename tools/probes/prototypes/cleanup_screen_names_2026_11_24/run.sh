#!/usr/bin/env bash
# Run one model over the cases for each arm and write OUT_DIR/TAG-ARM.jsonl.
#
#   run.sh OUT_DIR TAG [ARM...]        (default arms: new old none)
#
# env: CASES (prepare.py output), SESSION (session JSON), MODEL (GGUF),
#      LIB (libmurmur_llm.so), BIN (screen-names-clean), THREADS (default 4,
#      the home server's [model] threads).
#
# Shared-box rules: refuses when MemAvailable < model size + 2000 MB, runs
# under the build lock at low priority, one model at a time.
set -uo pipefail
OUT=$1; TAG=$2; shift 2
ARMS=${*:-new old none}
: "${CASES:?}" "${SESSION:?}" "${MODEL:?}" "${LIB:?}" "${BIN:?}"
THREADS=${THREADS:-4}
mkdir -p "$OUT"
for arm in $ARMS; do
  o=$OUT/$TAG-$arm.jsonl
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  need=$(( $(stat -c %s "$MODEL") / 1048576 + 2000 ))
  if (( avail < need )); then echo "skip $TAG-$arm: $avail MB < $need" >&2; exit 1; fi
  flock "$HOME/.cache/murmur-build.lock" timeout 3600 choom -n 1000 -- nice -n 10 \
    "$BIN" --cases "$CASES" --arm "$arm" --llm "$MODEL" --llm-lib "$LIB" \
    --session "$SESSION" --threads "$THREADS" > "$o.tmp" && mv "$o.tmp" "$o"
  echo "done $TAG-$arm" >&2
done
