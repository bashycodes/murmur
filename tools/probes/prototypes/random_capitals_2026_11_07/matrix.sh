#!/usr/bin/env bash
# One measurement round: a murmur-cli build over the parity sets and the PC
# recordings, Parakeet unified (greedy), bridge joins (product), plain joins
# and whole clips. All data paths are arguments; outputs stay private.
# MODES (env, default "bridge plain whole") picks the join modes.
# usage: matrix.sh MURMUR_CLI PARAKEET_DIR SESSION_SEG SESSION_NOSEG OUT_DIR NAME=WAV_DIR...
set -euo pipefail
here=$(dirname "$0")
cli=$1 model=$2 seg=$3 noseg=$4 out=$5
shift 5
mkdir -p "$out"
run() { choom -n 1000 -- nice -n 10 python3 -I "$here/run_set.py" "$cli" "$@"; }
for spec in "$@"; do
  name=${spec%%=*} wavs=${spec#*=}
  for mode in ${MODES:-bridge plain whole}; do
    session=$seg join=bridge
    [ "$mode" = whole ] && session=$noseg
    [ "$mode" = plain ] && join=plain
    MURMUR_SEGMENT_JOIN=$join run "$wavs" "$out/asr_${name}-${mode}.jsonl" -- --engine parakeet \
      --model "$model" --threads 4 --parakeet-decoding greedy --session "$session"
  done
done
