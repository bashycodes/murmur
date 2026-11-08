#!/usr/bin/env bash
# Wrong-capital counts for every asr_*.jsonl of a parity eval run dir.
# usage: score_runs.sh REFS.json RUN_DIR [FIELD] [--show]
set -euo pipefail
here=$(dirname "$0")
refs=$1 dir=$2
shift 2
for f in "$dir"/asr_*.jsonl; do
  python3 -I "$here/capscore.py" "$refs" "$f" "$@"
done
