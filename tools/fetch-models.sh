#!/usr/bin/env bash
# Explicit, user-initiated model download (D9: the app itself never fetches).
#   tools/fetch-models.sh [ggml-whisper.bin ...]   whisper models (default ggml-tiny.en.bin)
#   tools/fetch-models.sh --llm [0.5b|1.5b ...]    Qwen2.5-Instruct Q4_K_M GGUF for LLM cleanup (default 0.5b)
#   tools/fetch-models.sh --parakeet               Parakeet-TDT-0.6B-v2 int8 (sherpa-onnx, 640 MB unpacked) + jfk.wav
#   tools/fetch-models.sh --hebrew [--hebrew-parakeet]
#       Hebrew ASR (D33): ivrit-ai zipformer2-66m-streaming-he int8 (69 MiB, the
#       default Hebrew model, asr_backend "sherpa_online") and/or, with
#       --hebrew-parakeet, parakeet-tdt-0.6b-v3-he int8 (642 MiB, "parakeet").
#       Pinned revisions + SHA-256 from docs/findings/2026-10-30-hebrew-model-pins.md;
#       files already present with the right hash are skipped, a mismatch is
#       downloaded again, a download that does not verify is an error.
# Destination: FLOW_MODELS_DIR, else <repo>/models (the server's models dir:
#   FLOW_MODELS_DIR=/var/lib/flow-server/models tools/fetch-models.sh --hebrew).
set -euo pipefail
dir="${FLOW_MODELS_DIR:-$(cd "$(dirname "$0")/.." && pwd)/models}"
mkdir -p "$dir"

# fetch_pinned REPO REVISION DEST_SUBDIR "REMOTE_PATH SIZE SHA256" ...
# Each file lands as DEST_SUBDIR/<basename REMOTE_PATH>.
fetch_pinned() {
  local repo="$1" rev="$2" dest="$dir/$3" spec remote size sha f
  shift 3
  mkdir -p "$dest"
  for spec in "$@"; do
    read -r remote size sha <<<"$spec"
    f="$dest/$(basename "$remote")"
    if [ -f "$f" ] && [ "$(stat -c %s "$f")" = "$size" ] \
       && [ "$(sha256sum "$f" | cut -d' ' -f1)" = "$sha" ]; then
      echo "ok (verified): $f"
      continue
    fi
    [ -f "$f" ] && echo "hash mismatch, downloading again: $f"
    curl -fSL -o "$f.part" "https://huggingface.co/$repo/resolve/$rev/$remote"
    if [ "$(sha256sum "$f.part" | cut -d' ' -f1)" != "$sha" ]; then
      rm -f "$f.part"
      echo "fetch-models.sh: SHA-256 mismatch for $repo@$rev $remote" >&2
      return 1
    fi
    mv -f "$f.part" "$f"
    echo "ok (downloaded): $f"
  done
}

hebrew_zipformer() {
  fetch_pinned ivrit-ai/zipformer2-66m-streaming-he ee769a2a13b887f91572eec349458bfb0fe79e7a \
    zipformer2-66m-streaming-he-int8 \
    "sherpa-onnx/encoder.int8.onnx 70478964 6c6febcb74a09435315a1d872a6539f6e7fc2c333533f14c74edc0e85c49ffcd" \
    "sherpa-onnx/decoder.int8.onnx 1049105 feb03e7c8aa761593a9284a6fa1e91259be3702de75f630062256c1a59c5a906" \
    "sherpa-onnx/joiner.int8.onnx 771805 1d2c6d8e6dabc5eb52de52e95718e9bdfbffbfe7ead3808fb5ffd74e83bd9cae" \
    "sherpa-onnx/tokens.txt 17086 e4ebb5e72a6aeef47437a55fa2308ebbedb7c51c557de7a89bf965c23ea9af4a"
}

hebrew_parakeet() {
  fetch_pinned krut42/voice-parakeet-he-int8 39de3b473bed626c7796c156d71adc66161ebfe1 \
    parakeet-tdt-0.6b-v3-he-int8 \
    "encoder.int8.onnx 653215173 7eaf80bc88fccd579a7d3950c25e50e4c7935e5b037ec2aa1eee29bca5ca3981" \
    "decoder.int8.onnx 12494234 3a1d0bbf3e393909095f826b7719fdaef42cd2e0b8b95c54518034ecc4b7d246" \
    "joiner.int8.onnx 7008293 056c5464464b38a9cbacdccb6a9ff2114919780badb5c48e0f0260af7f91ccf2" \
    "tokens.txt 107590 55a91c862d0a63e8b9587a3264ced4fc76943e66098468b4638e6a6c5642e364"
}

case "${1:-}" in
  --hebrew|--hebrew-parakeet)
    for a in "$@"; do
      case "$a" in
        --hebrew|--hebrew-parakeet) ;;
        *) echo "fetch-models.sh: unknown option with --hebrew: $a" >&2; exit 2 ;;
      esac
    done
    for a in "$@"; do
      case "$a" in
        --hebrew) hebrew_zipformer ;;
        --hebrew-parakeet) hebrew_parakeet ;;
      esac
    done
    exit 0
    ;;
esac
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
