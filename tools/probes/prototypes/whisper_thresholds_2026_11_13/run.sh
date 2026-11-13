#!/usr/bin/env bash
# Whisper decoder thresholds experiment (2026-11-13): run the 40-clip WER bench
# (fitted window) and the tails bench for one build, low priority.
# usage: run.sh AUDIO_DIR MODELS_DIR "model1.bin,model2.bin" OUT_PREFIX
# Writes OUT_PREFIX-wer.jsonl and OUT_PREFIX-tails.jsonl. Build "before" and
# "after" by switching the thresholds in crates/murmur-core/src/whisper.rs,
# then score with score.py. Finding: docs/findings/2026-11-13-whisper-decoder-thresholds.md (D88).
set -euo pipefail
ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$ROOT"
run() {
  MURMUR_BENCH_AUDIO=$1 MURMUR_MODELS_DIR=$2 MURMUR_BENCH_MODELS=$3 MURMUR_BENCH_OUT=$4 MURMUR_BENCH_FIT=true \
    choom -n 1000 -- nice -n 10 cargo test --release -j 2 --offline -p murmur-core \
    --features whisper,wav --test "$5" -- --ignored --nocapture
}
run "$1" "$2" "$3" "$4-wer.jsonl" audio_ctx_bench
run "$1" "$2" "$3" "$4-tails.jsonl" whisper_tails_bench
