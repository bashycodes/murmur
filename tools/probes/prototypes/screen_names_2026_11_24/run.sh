#!/usr/bin/env bash
# On-screen names probe (2026-11-24): decodes the set prepare.py built with
# Parakeet three times per clip (no screen words / whole bubbles / name
# candidates) through murmur-core's screen_names_e2e, writing
# $SCREEN_NAMES_DATA/results/<timestamp>.jsonl (never overwritten), then scores it.
#
#   EN_ASR=<parakeet-tdt-0.6b-v2-int8 dir> run.sh
#   HE_ASR=<zipformer2-66m-streaming-he-int8 dir> SCREEN_NAMES_DATA=<he set> run.sh
#
# MURMUR_SCREEN_HOTWORDS_SCORE=<s> sets the screen names' hotword score (sweep).
# Needs the env from PROJECT_CONTEXT.md "How to verify" (BINDGEN_EXTRA_CLANG_ARGS,
# SHERPA_ONNX_LIB_DIR) and CARGO_TARGET_DIR. One model run at a time.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
data=${SCREEN_NAMES_DATA:-$HOME/.cache/murmur/screen-names-2026-11-24}
mkdir -p "$data/results"
out="$data/results/$(date +%Y%m%d-%H%M%S)${TAG:+-$TAG}.jsonl"
export MURMUR_SCREEN_MANIFEST="$data/manifest.jsonl" MURMUR_SCREEN_OUT="$out"
lang=en
if [ -n "${HE_ASR:-}" ]; then
  lang=he
  export MURMUR_SCREEN_ASR="$HE_ASR" MURMUR_SCREEN_ENGINE=sherpa_online MURMUR_SCREEN_LANG=he
else
  export MURMUR_SCREEN_ASR="${EN_ASR:?}"
fi
cd "$root"
cargo test -j 2 --offline -p murmur-core --features wav,parakeet --release \
  --test screen_names_e2e -- --nocapture --test-threads 1
"${PYTHON:-python3}" -I "$here/score.py" "$out" --manifest "$MURMUR_SCREEN_MANIFEST" --lang "$lang"
echo "$out"
