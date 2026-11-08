#!/usr/bin/env bash
# Base vs new flow-server, live, with and without the dictionary, R
# interleaved rounds (live.sh each), so box load hits both alike.
#
#   live_ab.sh OUT_DIR ROUNDS SPEED
#
# env: live.sh's env (minus SERVER), plus BASE and NEW (two flow-server
# binaries) and DICT (run_paths.py --dict file). ENVF, when set, is sourced
# first. Output: OUT_DIR/server_{base,new}-{dict,nodict}-rN.jsonl.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1090
[ -n "${ENVF:-}" ] && . "$ENVF"
OUT=$1; R=$2; SPEED=$3
: "${BASE:?}" "${NEW:?}" "${DICT:?}"
for r in $(seq 1 "$R"); do
  for v in base new; do
    bin=$BASE; [ "$v" = new ] && bin=$NEW
    [ -s "$OUT/server_$v-dict-r$r.jsonl" ] ||
      SERVER=$bin bash "$here/live.sh" "$OUT" "$v-dict-r$r" "$SPEED" --dict "$DICT"
    [ -s "$OUT/server_$v-nodict-r$r.jsonl" ] ||
      SERVER=$bin bash "$here/live.sh" "$OUT" "$v-nodict-r$r" "$SPEED"
  done
done
