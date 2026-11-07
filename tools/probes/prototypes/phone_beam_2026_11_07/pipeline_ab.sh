#!/usr/bin/env bash
# Dated probe (2026-11-07) for docs/findings/2026-11-07-phone-beam-hotwords.md:
# the full dictation pipeline (`flow-cli transcribe`, through the wispr-parity
# runner's `asr` command) with Parakeet greedy vs beam-only + dictionary
# hotwords, the phone's two D68 paths. Not a permanent tool.
#
#   RUNNER=<tools/probes/wispr_parity/run_paths.py> CLI=<release flow-cli, feature parakeet>
#   MODEL=<parakeet model dir> CLIPS=<dir of WAVs> DICT=<dictionary JSON> SESSION=<session JSON>
#   OUT=<private output dir, one per run> SHERPA_ONNX_LIB_DIR=…  pipeline_ab.sh
#
# Writes OUT/asr_<label>.jsonl per spec; score with wispr_parity/score.py.
set -uo pipefail
: "${RUNNER:?}" "${CLI:?}" "${MODEL:?}" "${CLIPS:?}" "${DICT:?}" "${SESSION:?}" "${OUT:?}"
export LD_LIBRARY_PATH=${SHERPA_ONNX_LIB_DIR:-}:${LD_LIBRARY_PATH:-}
LOCK=${LOCK:-$HOME/.cache/flow-clone-build.lock}
P="--engine parakeet --model $MODEL --threads 4"
if [ -n "${FLOW_PARAKEET_BEAM:-}" ]; then
  # Beam width probe: only the beam spec, labelled with the width (flow-cli reads the env knob).
  SPECS=(--spec "beam${FLOW_PARAKEET_BEAM}-dict=$P --parakeet-decoding beam --dict $DICT")
else
  SPECS=(--spec "greedy=$P --parakeet-decoding greedy"
    --spec "beam-dict=$P --parakeet-decoding beam --dict $DICT"
    --spec "greedy-dict=$P --parakeet-decoding greedy --dict $DICT")
fi
flock "$LOCK" choom -n 1000 -- nice -n 10 python3 -I "$RUNNER" asr --clips "$CLIPS" --out "$OUT" --cli "$CLI" --session "$SESSION" "${SPECS[@]}"
