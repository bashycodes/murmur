#!/usr/bin/env bash
# Explicit, user-initiated model download (D9: the app itself never fetches).
set -euo pipefail
dir="$(cd "$(dirname "$0")/.." && pwd)/models"
mkdir -p "$dir"
base=https://huggingface.co/ggerganov/whisper.cpp/resolve/main
for m in "${@:-ggml-tiny.en.bin}"; do
  [ -f "$dir/$m" ] || curl -fSL -o "$dir/$m" "$base/$m"
done
# JFK inaugural address excerpt (US-government work, public domain), whisper.cpp sample.
[ -f "$dir/jfk.wav" ] || curl -fSL -o "$dir/jfk.wav" https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav
ls -la "$dir"
