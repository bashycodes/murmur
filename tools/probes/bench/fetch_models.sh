#!/usr/bin/env bash
# Fetch benchmark models (not in git) into $CACHE/models. Idempotent.
# Usage: tools/probes/bench/fetch_models.sh [asr|llm|parakeet|all]
set -euo pipefail
CACHE="${CACHE:-$HOME/Claude/research/wispr/.cache}"
M="$CACHE/models"; mkdir -p "$M"
what="${1:-all}"

get() { # url dest
  local url="$1" dest="$M/$2"
  if [ -s "$dest" ]; then echo "have $2"; return; fi
  echo "fetch $2"; curl -fL --retry 3 -o "$dest.part" "$url" && mv "$dest.part" "$dest"
}

HFW=https://huggingface.co/ggerganov/whisper.cpp/resolve/main
if [[ $what == asr || $what == all ]]; then
  for f in ggml-tiny-q5_1.bin ggml-base-q5_1.bin ggml-small-q5_1.bin ggml-large-v3-turbo-q5_0.bin; do
    get "$HFW/$f" "$f"
  done
fi

if [[ $what == llm || $what == all ]]; then
  get https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf qwen2.5-0.5b-instruct-q4_k_m.gguf
  get https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf qwen2.5-1.5b-instruct-q4_k_m.gguf
  get https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf qwen2.5-3b-instruct-q4_k_m.gguf
  get https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf Llama-3.2-1B-Instruct-Q4_K_M.gguf
  get https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf Llama-3.2-3B-Instruct-Q4_K_M.gguf
fi

if [[ $what == llm7b || $what == all ]]; then  # reference point: is quality reachable at all on CPU?
  get https://huggingface.co/bartowski/Qwen2.5-7B-Instruct-GGUF/resolve/main/Qwen2.5-7B-Instruct-Q4_K_M.gguf Qwen2.5-7B-Instruct-Q4_K_M.gguf
fi

if [[ $what == parakeet || $what == all ]]; then
  SO=v1.13.8
  if [ ! -x "$CACHE/sherpa-onnx/bin/sherpa-onnx-offline" ]; then
    get "https://github.com/k2-fsa/sherpa-onnx/releases/download/$SO/sherpa-onnx-$SO-linux-x64-shared-no-tts.tar.bz2" sherpa-onnx.tar.bz2
    mkdir -p "$CACHE/sherpa-onnx" && tar -xjf "$M/sherpa-onnx.tar.bz2" -C "$CACHE/sherpa-onnx" --strip-components=1
  fi
  if [ ! -d "$M/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8" ]; then
    get https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2 parakeet.tar.bz2
    tar -xjf "$M/parakeet.tar.bz2" -C "$M"
  fi
fi
ls -la "$M"
