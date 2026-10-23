#!/usr/bin/env bash
# Re-run of the 2026-10-23 cliptest (whole vs segment_pause_ms 800, previews
# on, real-time pacing unless CLIPTEST_FAST=1) against this worktree's
# flow-core. Writes a new per-run dir; grade with analyze.py.
# usage: run_cliptest.sh CLIPS_DIR OUT_ROOT PARAKEET_DIR WHISPER_MODEL
set -u
CLIPS=$1; OUT=$2/$(date +%Y%m%d-%H%M%S); P=$3; W=$4
B=${CARGO_TARGET_DIR:-target}/release/cliptest
mkdir -p "$OUT"
NAMES="jfk ls_1221_F ls_7021_M ls_6829_F ls_3729_F_french ls_1188_M ls_4992_F long_010s long_028s long_040s ls_4992_F_pink10dB ls_7021_M_fast1.25"
uptime > "$OUT/load_start"
"$B" '{"asr_engine":"parakeet","asr_path":"'"$P"'","threads":4}' '["en"]' "$CLIPS" $NAMES > "$OUT/parakeet.jsonl" 2> "$OUT/parakeet.err"
"$B" '{"asr_engine":"whisper","asr_path":"'"$W"'","threads":4}' '["en"]' "$CLIPS" $NAMES > "$OUT/whisper.jsonl" 2> "$OUT/whisper.err"
uptime > "$OUT/load_end"; echo "$OUT"
