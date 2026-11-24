#!/usr/bin/env bash
# Language-detection probe (2026-11-24): runs murmur-core's lang_detect_e2e
# against the manifest prepare.py built, writing per-session results under
# $LANG_DETECT_DATA/results/<timestamp>-<mode>.jsonl (never overwritten).
#
#   LID_MODEL=…/ggml-tiny-q5_1.bin run.sh scores      # raw per-window LID scores
#   LID_MODEL=… run.sh route                           # full core path, mock decoders
#   LID_MODEL=… EN_ASR=<parakeet dir> HE_ASR=<zipformer dir> run.sh route   # real text, WER
#
# Needs the env from PROJECT_CONTEXT.md "How to verify" (BINDGEN_EXTRA_CLANG_ARGS,
# SHERPA_ONNX_LIB_DIR) and CARGO_TARGET_DIR. One model run at a time.
set -euo pipefail
mode=${1:?scores|route}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
data=${LANG_DETECT_DATA:-$HOME/.cache/murmur/lang-detect-2026-11-24}
mkdir -p "$data/results"
out="$data/results/$(date +%Y%m%d-%H%M%S)-$mode${TAG:+-$TAG}.jsonl"
export MURMUR_LANG_MANIFEST="$data/manifest.jsonl" MURMUR_LID_MODEL="${LID_MODEL:?}" MURMUR_LANG_OUT="$out"
[ "$mode" = scores ] && export MURMUR_LANG_SCORES=1
[ -n "${EN_ASR:-}" ] && export MURMUR_LANG_EN_ASR="$EN_ASR" MURMUR_LANG_HE_ASR="${HE_ASR:?}"
cd "$root"
cargo test -j 2 --offline -p murmur-core --features whisper,wav,parakeet --release \
  --test lang_detect_e2e -- --nocapture --test-threads 1
py=${PYTHON:-python3}
if [ "$mode" = scores ]; then
  "$py" -I "$here/score.py" rules "$out"
else
  "$py" -I "$here/score.py" route "$out" --manifest "$MURMUR_LANG_MANIFEST"
fi
echo "$out"
