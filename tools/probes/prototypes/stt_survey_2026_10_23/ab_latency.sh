#!/usr/bin/env bash
# Dated probe 2026-10-23: interleaved latency A/B so shared-box load drift
# (load avg 15-27 from other agents during the survey) hits every model
# equally. ROUNDS rounds; each round runs every model once (fresh process,
# warm page cache) on the ~11 s and ~29 s concatenated clips, 3 reps each.
# Usage: ab_latency.sh [ROUNDS] [THREADS]; scores via score.py.
set -euo pipefail
CACHE=${CACHE:-$HOME/Claude/research/wispr/.cache}
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=$CACHE/stt-survey-target-2/release/stt_survey_2026_10_23
OUT=$CACHE/results/stt-survey-2026-10-23/ab-$(date +%Y%m%d-%H%M%S)
M=$CACHE/models/survey-2026-10-23
MAN=$HERE/../../bench/asr_manifest_extended.json
export LD_LIBRARY_PATH=$CACHE/sherpa-onnx/lib
mkdir -p "$OUT"
ROUNDS=${1:-3}; THR=${2:-4}
d() { echo "$M/dl-$1/$1"; }
MODELS=(
  "v2 transducer $CACHE/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"
  "unified transducer $(d sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming)"
  "v3 transducer $(d sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8)"
  "canary180m canary $(d sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8)"
  "nemotron online $(d sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25)"
)
for r in $(seq "$ROUNDS"); do
  for m in "${MODELS[@]}"; do
    set -- $m
    "$BIN" "$2" "$3" "$MAN" "$CACHE/audio" --no-clips --threads "$THR" --long 11,28 --reps 3 \
      >>"$OUT/$1-t$THR.jsonl" 2>>"$OUT/$1.err"
  done
done
for f in "$OUT"/*.jsonl; do
  python3 -I - "$f" <<'EOF'
import json, sys, statistics as st
rows = [json.loads(l) for l in open(sys.argv[1]) if l.startswith("{")]
for L in sorted({round(r["dur_s"]) for r in rows}):
    xs = [r["infer_ms"] for r in rows if round(r["dur_s"]) == L]
    cs = [r["cores"] for r in rows if round(r["dur_s"]) == L]
    print(f"{sys.argv[1].split('/')[-1]:<20} {L:>3}s n={len(xs)} median={st.median(xs):6.0f} ms min={min(xs):6.0f} ms cores={st.median(cs):.2f}")
EOF
done
