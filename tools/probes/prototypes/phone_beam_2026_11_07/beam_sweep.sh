#!/usr/bin/env bash
# Dated probe (2026-11-07) for docs/findings/2026-11-07-phone-beam-hotwords.md:
# Parakeet greedy vs modified beam search (widths 4..1) with dictionary
# hotwords, one warm engine per run (`flow-asr-sherpa` example spot_check).
# Not a permanent tool; the consumer is the findings doc's tables.
#
#   SPOT=<release spot_check binary> MODEL=<parakeet model dir> MANIFEST=<spot_check manifest>
#   AUDIO=<dir of ID.wav> HOTWORDS=<file: comma-separated phrases> OUT=<private output dir>
#   SHERPA_ONNX_LIB_DIR=… [RUNS="1 2 3"] [BEAMS="4 3 2 1"] beam_sweep.sh
#
# Writes OUT/sweep_<tag>_<run>.jsonl (one line per clip: id, hyp, infer_ms) and
# its stderr summary; an existing output is kept (rerun = resume).
set -uo pipefail
: "${SPOT:?}" "${MODEL:?}" "${MANIFEST:?}" "${AUDIO:?}" "${HOTWORDS:?}" "${OUT:?}"
mkdir -p "$OUT"
export LD_LIBRARY_PATH=${SHERPA_ONNX_LIB_DIR:-}:${LD_LIBRARY_PATH:-}
HW=$(cat "$HOTWORDS")
LOCK=${LOCK:-$HOME/.cache/flow-clone-build.lock}
run() {
  local tag=$1
  shift
  [ -s "$OUT/sweep_$tag.jsonl" ] && return
  flock "$LOCK" choom -n 1000 -- nice -n 10 "$SPOT" "$MODEL" "$MANIFEST" "$AUDIO" --threads 4 "$@" \
    >"$OUT/sweep_$tag.jsonl.tmp" 2>"$OUT/sweep_$tag.log" &&
    mv "$OUT/sweep_$tag.jsonl.tmp" "$OUT/sweep_$tag.jsonl"
  echo "$tag $(tail -1 "$OUT/sweep_$tag.log")"
}
for r in ${RUNS:-1}; do
  run "greedy_$r" --greedy
  for b in ${BEAMS:-4 3 2 1}; do run "beam${b}_$r" --beam "$b" --hotwords "$HW"; done
done
