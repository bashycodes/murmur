#!/usr/bin/env bash
# Probe (stray "yeah", 2026-11-07): run_cli.sh over several WAVs in an
# evidence dir, with the PC session settings (pc_session.json).
#
# Usage: batch.sh <flow-cli dir> <parakeet model dir> <evidence dir> <wav stem>...
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$1"; model="$2"; dir="$3"; shift 3
for w in "$@"; do
  echo "== $w"
  timeout 600 "$here/run_cli.sh" "$bin" "$model" "$dir/$w.wav" --session "$here/${SESSION:-pc_session.json}" \
    | sed -E 's/"text": "[^"]*"/"text": <..>/g' | cut -c1-700
done
