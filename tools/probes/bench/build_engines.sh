#!/usr/bin/env bash
# Build whisper.cpp + llama.cpp (CPU, static) into the shared cache, outside the repo.
# Usage: tools/probes/bench/build_engines.sh   (env CACHE overrides the cache dir)
set -euo pipefail
CACHE="${CACHE:-$HOME/Claude/research/wispr/.cache}"
JOBS="${JOBS:-6}"
mkdir -p "$CACHE"
[ -d "$CACHE/whisper.cpp" ] || git clone --depth 1 https://github.com/ggml-org/whisper.cpp "$CACHE/whisper.cpp"
[ -d "$CACHE/llama.cpp" ]   || git clone --depth 1 https://github.com/ggml-org/llama.cpp   "$CACHE/llama.cpp"

cmake -S "$CACHE/whisper.cpp" -B "$CACHE/whisper.cpp/build" -DCMAKE_BUILD_TYPE=Release \
  -DWHISPER_SDL2=OFF -DBUILD_SHARED_LIBS=OFF
cmake --build "$CACHE/whisper.cpp/build" -j "$JOBS" --target whisper-cli whisper-bench

cmake -S "$CACHE/llama.cpp" -B "$CACHE/llama.cpp/build" -DCMAKE_BUILD_TYPE=Release \
  -DLLAMA_CURL=OFF -DBUILD_SHARED_LIBS=OFF
cmake --build "$CACHE/llama.cpp/build" -j "$JOBS" --target llama-cli llama-server llama-bench

echo "whisper rev: $(git -C "$CACHE/whisper.cpp" rev-parse --short HEAD)"
echo "llama rev:   $(git -C "$CACHE/llama.cpp" rev-parse --short HEAD)"
