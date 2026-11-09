#!/usr/bin/env bash
# Dated probe 2026-10-23: does a model drop whole utterances in one long
# (43 s / 60 s) decode? (sherpa-onnx PR #4084 reports this for Parakeet int8.)
set -euo pipefail
CACHE=${CACHE:-$HOME/.cache/murmur}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=$CACHE/stt-survey-target-2/release/stt_survey_2026_10_23
OUT=$CACHE/results/stt-survey-2026-10-23/longdel-$(date +%Y%m%d-%H%M%S)
M=$CACHE/models/survey-2026-10-23
export LD_LIBRARY_PATH=$CACHE/sherpa-onnx/lib
mkdir -p "$OUT"
d() { echo "$M/dl-$1/$1"; }
while read -r name kind dir extra; do
  for man in long43 long60; do
    L=$([ $man = long43 ] && echo 44 || echo 61)
    "$BIN" "$kind" "$dir" "$HERE/${man}_manifest.json" "$CACHE/audio" --no-clips --threads 4 --long $L --reps 1 $extra \
      >>"$OUT/$name.jsonl" 2>>"$OUT/$name.err" || echo "FAILED $name $man"
  done
  python3 -I "$HERE/score.py" "$OUT/$name.jsonl"
done <<LIST
v2 transducer $CACHE/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8
v2beam4 transducer $CACHE/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 --beam 4
v3 transducer $(d sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8)
unified transducer $(d sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming)
nemotron online $(d sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25)
cohere cohere $(d sherpa-onnx-cohere-transcribe-14-lang-int8-2026-04-01)
LIST
