#!/usr/bin/env bash
# Dated probe 2026-10-23 (docs/proposals/stt-model-survey.md): run every
# candidate sequentially (no self-contention) on the 40-clip extended set plus
# concatenated long clips. Results: $CACHE/results/stt-survey-2026-10-23/ (one
# file per run, never overwritten: a timestamp suffix is added if one exists).
# Models were fetched into $CACHE/models/survey-2026-10-23/dl-<name>/<name>/.
set -euo pipefail
CACHE=${CACHE:-$HOME/.cache/murmur}
HERE=$(cd "$(dirname "$0")" && pwd)
PROBES=$(cd "$HERE/../.." && pwd)
BIN=$CACHE/stt-survey-target-2/release/stt_survey_2026_10_23
OUT=$CACHE/results/stt-survey-2026-10-23
M=$CACHE/models/survey-2026-10-23
MAN=$PROBES/bench/asr_manifest_extended.json
export LD_LIBRARY_PATH=$CACHE/sherpa-onnx/lib
mkdir -p "$OUT"
LONG=${LONG:-5,10,15,20,25,28,32}

dir() { echo "$M/dl-$1/$1"; }
run() { # name kind modeldir args...
  local name=$1 kind=$2 d=$3; shift 3
  local f=$OUT/$name.jsonl
  [ -e "$f" ] && f=$OUT/$name-$(date +%H%M%S).jsonl
  echo "== $name (load1 $(cut -d' ' -f1 /proc/loadavg))" >&2
  "$BIN" "$kind" "$d" "$MAN" "$CACHE/audio" --long "$LONG" "$@" >"$f" 2>"${f%.jsonl}.err" || echo "FAILED $name" >&2
  python3 -I "$HERE/score.py" "$f"
}

V2=$CACHE/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8
UNI=$(dir sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming)
V3=$(dir sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8)
for what in ${WHAT:-v2t4 v2b4 uni4 v3 canary moon qwen3 cohere nemotron v2t1 v2t8 unit1 unit8 moont1}; do
  case $what in
    v2b4)   run v2-t4-beam4 transducer "$V2" --threads 4 --beam 4 ;;
    v2t4)   run v2-t4-greedy transducer "$V2" --threads 4 ;;
    v2t1)   run v2-t1-greedy transducer "$V2" --threads 1 ;;
    v2t8)   run v2-t8-greedy transducer "$V2" --threads 8 ;;
    uni4)   run unified-t4-greedy transducer "$UNI" --threads 4 ;;
    unit1)  run unified-t1-greedy transducer "$UNI" --threads 1 ;;
    unit8)  run unified-t8-greedy transducer "$UNI" --threads 8 ;;
    v3)     run v3-t4-greedy transducer "$V3" --threads 4 ;;
    canary) run canary180m-t4 canary "$(dir sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8)" --threads 4 ;;
    moon)   run moonshine-base-v2-t4 moonshine2 "$(dir sherpa-onnx-moonshine-base-en-quantized-2026-02-27)" --threads 4 ;;
    moont1) run moonshine-base-v2-t1 moonshine2 "$(dir sherpa-onnx-moonshine-base-en-quantized-2026-02-27)" --threads 1 ;;
    qwen3)  run qwen3-0.6b-t4 qwen3 "$(dir sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25)" --threads 4 ;;
    cohere) run cohere-t4 cohere "$(dir sherpa-onnx-cohere-transcribe-14-lang-int8-2026-04-01)" --threads 4 ;;
    nemotron) run nemotron-en-560ms-t4 online "$(dir sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25)" --threads 4 ;;
  esac
done
