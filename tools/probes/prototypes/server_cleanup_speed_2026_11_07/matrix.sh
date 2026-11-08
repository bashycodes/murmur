#!/usr/bin/env bash
# Run several run.sh variants back to back, R rounds interleaved, so a load
# change on the shared box hits every variant alike.
#
#   matrix.sh OUT_DIR ROUNDS 'TAG|ENV=...|runner args' ...
#
# Each spec: tag, then optional space-separated env assignments, then
# optional runner args. Output: OUT_DIR/TAG.rN.jsonl. Needs run.sh's env.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1090
[ -n "${ENVF:-}" ] && . "$ENVF"
OUT=$1; R=$2; shift 2
for r in $(seq 1 "$R"); do
  for spec in "$@"; do
    IFS='|' read -r tag envs args <<< "$spec"
    [ -s "$OUT/$tag.r$r.jsonl" ] && continue
    # shellcheck disable=SC2086
    env $envs bash "$here/run.sh" "$OUT" "$tag.r$r" $args
  done
done
