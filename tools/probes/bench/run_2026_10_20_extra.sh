#!/usr/bin/env bash
# Dated probe (2026-10-20): follow-up measurements after the first run_all pass.
#  1. llama-bench pp512/tg64 per GGUF at 4 threads (clean throughput, 3 reps) — feeds phone extrapolation.
#  2. whisper-bench encoder-only per ggml model at 4 threads.
#  3. Extended ASR set (one utterance for each of the 40 test-clean speakers) for tiny/base/small/parakeet,
#     because 308 words in the core set cannot separate base vs small.
#  4. whisper -ac (audio_ctx sized to clip) on the core set: latency win for short dictations?
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CACHE="${CACHE:-$HOME/.cache/murmur}"
TS="$(date +%Y%m%d-%H%M%S)"; L="$CACHE/logs"
{
  for m in qwen2.5-0.5b-instruct-q4_k_m.gguf qwen2.5-1.5b-instruct-q4_k_m.gguf qwen2.5-3b-instruct-q4_k_m.gguf \
           Llama-3.2-1B-Instruct-Q4_K_M.gguf Llama-3.2-3B-Instruct-Q4_K_M.gguf Qwen2.5-7B-Instruct-Q4_K_M.gguf; do
    uptime
    "$CACHE/llama.cpp/build/bin/llama-bench" -m "$CACHE/models/$m" -t 4 -p 512 -n 64 -r 3 -o md
  done
} 2>&1 | tee "$L/llama-bench-$TS.log"
{
  for m in ggml-tiny-q5_1.bin ggml-base-q5_1.bin ggml-small-q5_1.bin ggml-large-v3-turbo-q5_0.bin; do
    uptime
    "$CACHE/whisper.cpp/build/bin/whisper-bench" -m "$CACHE/models/$m" -t 4 2>&1 | grep -E 'encode time|system_info' || true
  done
} 2>&1 | tee "$L/whisper-bench-$TS.log"
if [ ! -s "$HERE/asr_manifest_extended.json" ]; then
  "$CACHE/venv/bin/python" -I "$HERE/fetch_audio.py" --pick 40 > "$L/pick40-$TS.json"
  python3 -I - "$L/pick40-$TS.json" "$HERE/asr_manifest_extended.json" <<'EOF'
import json, sys
picks = json.load(open(sys.argv[1]))
json.dump({"about": "Extended ASR set: first 5-20 s utterance of each LibriSpeech test-clean speaker (CC BY 4.0). Audio fetched by fetch_audio.py --manifest asr_manifest_extended.json.",
           "utterances": [dict(source="librispeech-test-clean", **p) for p in picks]}, open(sys.argv[2], "w"), indent=1)
EOF
fi
"$CACHE/venv/bin/python" -I "$HERE/fetch_audio.py" --manifest asr_manifest_extended.json
python3 -I "$HERE/bench_asr.py" --manifest asr_manifest_extended.json --models tiny,base,small,parakeet --threads 4 | tee "$L/asr-ext-$TS.log"
python3 -I "$HERE/bench_asr.py" --audio-ctx-auto --models tiny,base,small,turbo --threads 4 | tee "$L/asr-ac-$TS.log"
