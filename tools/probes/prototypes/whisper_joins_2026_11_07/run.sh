#!/usr/bin/env bash
# One probe run (shared box: low priority). All data paths are arguments.
# usage: run.sh core|pieces WHISPER_MODEL SESSION.json CLIPS_DIR OUT.jsonl [fit_ctx true|false] [clip ...]
set -euo pipefail
MODE=$1 MODEL=$2 SESSION=$3 CLIPS=$4 OUT=$5 FIT=${6:-true}
shift 6 || shift $#
BIN=${CARGO_TARGET_DIR:-target}/release/joins
CFG='{"asr_engine":"whisper","asr_path":"'"$MODEL"'","threads":4,"whisper_fit_ctx":'"$FIT"'}'
exec choom -n 1000 -- nice -n 10 "$BIN" "$MODE" "$CFG" "$SESSION" "$CLIPS" "$OUT" "$@"
