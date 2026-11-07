#!/usr/bin/env bash
# D20 40-clip audio_ctx bench, fitted window only, low priority (D66 false-alarm check).
# usage: bench.sh AUDIO_DIR MODELS_DIR "model1.bin,model2.bin" OUT.jsonl
# Prints the per-model ASR total and full-window re-decode count on stderr.
set -euo pipefail
ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$ROOT"
FLOW_BENCH_AUDIO=$1 FLOW_MODELS_DIR=$2 FLOW_BENCH_MODELS=$3 FLOW_BENCH_OUT=$4 FLOW_BENCH_FIT=true \
  exec choom -n 1000 -- nice -n 10 cargo test --release -j 2 --offline -p flow-core \
  --features whisper,wav --test audio_ctx_bench -- --ignored --nocapture
