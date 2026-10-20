#!/usr/bin/env bash
# Explicit, user-initiated model download (D9: the app itself never fetches).
#   tools/fetch-models.sh [ggml-whisper.bin ...]   whisper models (default ggml-tiny.en.bin)
#   tools/fetch-models.sh --llm [0.5b|1.5b ...]    Qwen2.5-Instruct Q4_K_M GGUF for LLM cleanup (default 0.5b)
#   tools/fetch-models.sh --parakeet               Parakeet-TDT-0.6B-v2 int8 (sherpa-onnx, 640 MB unpacked) + jfk.wav
set -euo pipefail
dir="$(cd "$(dirname "$0")/.." && pwd)/models"
mkdir -p "$dir"
if [ "${1:-}" = "--parakeet" ]; then
  p=sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8
  if [ ! -f "$dir/$p/tokens.txt" ]; then
    curl -fSL -o "$dir/$p.tar.bz2.part" "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/$p.tar.bz2"
    tar -xjf "$dir/$p.tar.bz2.part" -C "$dir" && rm -f "$dir/$p.tar.bz2.part"
  fi
  [ -f "$dir/jfk.wav" ] || curl -fSL -o "$dir/jfk.wav" https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav
  ls -la "$dir/$p"
  exit 0
fi
if [ "${1:-}" = "--llm" ]; then
  shift
  for s in "${@:-0.5b}"; do
    f="qwen2.5-$s-instruct-q4_k_m.gguf"
    [ -f "$dir/$f" ] && continue
    curl -fSL -o "$dir/$f.part" "https://huggingface.co/Qwen/Qwen2.5-${s^^}-Instruct-GGUF/resolve/main/$f"
    mv -f "$dir/$f.part" "$dir/$f"
  done
  ls -la "$dir"
  exit 0
fi
base=https://huggingface.co/ggerganov/whisper.cpp/resolve/main
for m in "${@:-ggml-tiny.en.bin}"; do
  [ -f "$dir/$m" ] || curl -fSL -o "$dir/$m" "$base/$m"
done
# JFK inaugural address excerpt (US-government work, public domain), whisper.cpp sample.
[ -f "$dir/jfk.wav" ] || curl -fSL -o "$dir/jfk.wav" https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav
ls -la "$dir"
