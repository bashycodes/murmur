#!/usr/bin/env bash
# Stage host-built murmur-server binaries for the container image, so
#   docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt .
# needs no network and no compiler (docs/container.md "Air-gapped build").
#
#   tools/container/stage-prebuilt.sh [--build] [--sherpa-lib DIR]
#                                     [--cuda-llm FILE] [--cuda-whisper FILE --cuda-lib DIR]
#
#   --build             first build murmur-server (features
#                       whisper,whisper-plugin,parakeet,tls,llm) and the murmur-llm
#                       plugin, release, --offline
#   --sherpa-lib DIR    dir with libsherpa-onnx-c-api.so + libonnxruntime.so, the no-TTS
#                       build (sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib/lib; D80)
#                       (default $SHERPA_ONNX_LIB_DIR, else target/release)
#   --cuda-llm FILE     a CUDA build of libmurmur_llm.so (`cargo build --release
#                       -p murmur-llm --features cuda`), staged as
#                       libmurmur_llm_cuda.so for `--target cuda`
#   --cuda-whisper FILE a CUDA build of libmurmur_whisper.so (`cargo build --release
#                       -p murmur-whisper --features cuda`, D84), staged as
#                       libmurmur_whisper_cuda.so for `--target cuda`
#   --cuda-lib DIR      the CUDA toolkit's lib64 (e.g. /usr/local/cuda/lib64): the
#                       libcudart, libcublas, libcublasLt .so.12 the whisper plugin
#                       links; needed with --cuda-whisper
#
# The CUDA files go to deploy/docker/prebuilt/cuda/, which only the CUDA image
# copies (D84 moved libmurmur_llm_cuda.so there from prebuilt/lib/: restage
# an older staging before building `--target cuda`). The CUDA image points
# MURMUR_WHISPER_LIB at libmurmur_whisper_cuda.so; staged without
# --cuda-whisper, whisper runs on the CPU with a warning in the log.
#
# The image runs Ubuntu 24.04 (glibc 2.39): build on a host with that glibc
# or older. Writes deploy/docker/prebuilt/{bin,lib,cuda} (git-ignored).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}/release"
OUT="$ROOT/deploy/docker/prebuilt"
SHERPA_LIB="${SHERPA_ONNX_LIB_DIR:-}"
BUILD=0
CUDA_LLM=""
CUDA_WHISPER=""
CUDA_LIB=""

die() { echo "stage-prebuilt.sh: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
    case "$1" in
        --build) BUILD=1 ;;
        --sherpa-lib) SHERPA_LIB="${2:?--sherpa-lib DIR}"; shift ;;
        --cuda-llm) CUDA_LLM="${2:?--cuda-llm FILE}"; shift ;;
        --cuda-whisper) CUDA_WHISPER="${2:?--cuda-whisper FILE}"; shift ;;
        --cuda-lib) CUDA_LIB="${2:?--cuda-lib DIR}"; shift ;;
        -h|--help) sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done

if [ "$BUILD" = 1 ]; then
    [ -n "$SHERPA_LIB" ] || die "--build needs --sherpa-lib DIR or SHERPA_ONNX_LIB_DIR (offline build)"
    GCC_INC="$(gcc -print-file-name=include)"
    (cd "$ROOT" &&
        SHERPA_ONNX_LIB_DIR="$SHERPA_LIB" cargo build --release --offline -p murmur-server \
            --no-default-features --features whisper,whisper-plugin,parakeet,tls,llm &&
        BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I$GCC_INC}" \
            cargo build --release --offline -p murmur-llm)
fi

SHERPA_LIB="${SHERPA_LIB:-$TARGET}"
[ -x "$TARGET/murmur-server" ] || die "$TARGET/murmur-server not built (pass --build)"
[ -f "$TARGET/libmurmur_llm.so" ] || die "$TARGET/libmurmur_llm.so not built (cargo build --release -p murmur-llm)"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    [ -e "$SHERPA_LIB/$l" ] || die "$SHERPA_LIB/$l not found (--sherpa-lib DIR)"
done
GOMP="$(gcc -print-file-name=libgomp.so.1 2>/dev/null || true)"
[ -e "$GOMP" ] || GOMP="$(ldconfig -p | awk '/libgomp\.so\.1 .*x86-64/ {print $NF; exit}')"
[ -e "$GOMP" ] || die "libgomp.so.1 not found (the llama.cpp plugin needs it)"

[ -z "$CUDA_WHISPER" ] || [ -n "$CUDA_LIB" ] || die "--cuda-whisper needs --cuda-lib DIR (its cuBLAS/cudart)"
if [ -n "$CUDA_LLM" ] && [ -z "$CUDA_WHISPER" ]; then
    echo "stage-prebuilt.sh: warning: --cuda-llm without --cuda-whisper: the CUDA image's" \
        "whisper speech models will run on the CPU (MURMUR_WHISPER_LIB names a missing file)" >&2
fi
rm -rf "${OUT:?}/bin" "${OUT:?}/lib" "${OUT:?}/cuda"
mkdir -p "$OUT/bin" "$OUT/lib"
install -m 0755 "$TARGET/murmur-server" "$OUT/bin/murmur-server"
install -m 0644 "$TARGET/libmurmur_llm.so" "$OUT/lib/libmurmur_llm.so"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    install -m 0644 "$(readlink -f "$SHERPA_LIB/$l")" "$OUT/lib/$l"
done
install -m 0644 "$(readlink -f "$GOMP")" "$OUT/lib/libgomp.so.1"
# D80: the no-TTS sherpa-onnx build only (no espeak-ng, GPL-3.0).
"$ROOT/tools/checks/no-tts-libs.sh" "$OUT"/lib/libsherpa-onnx-c-api.so "$OUT"/lib/libonnxruntime.so ||
    die "--sherpa-lib: use sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib (docs/air-gapped.md)"
if [ -n "$CUDA_LLM" ]; then
    [ -f "$CUDA_LLM" ] || die "--cuda-llm $CUDA_LLM: no such file"
    mkdir -p "$OUT/cuda"
    install -m 0644 "$CUDA_LLM" "$OUT/cuda/libmurmur_llm_cuda.so"
fi
if [ -n "$CUDA_WHISPER" ]; then
    [ -f "$CUDA_WHISPER" ] || die "--cuda-whisper $CUDA_WHISPER: no such file"
    mkdir -p "$OUT/cuda"
    install -m 0644 "$CUDA_WHISPER" "$OUT/cuda/libmurmur_whisper_cuda.so"
    for l in libcudart.so.12 libcublas.so.12 libcublasLt.so.12; do
        [ -e "$CUDA_LIB/$l" ] || die "--cuda-lib $CUDA_LIB: no $l"
        install -m 0644 "$(readlink -f "$CUDA_LIB/$l")" "$OUT/cuda/$l"
    done
fi
echo "staged in ${OUT#"$ROOT"/}:"
dirs="bin lib"
[ -d "$OUT/cuda" ] && dirs="$dirs cuda"
# shellcheck disable=SC2086 # $dirs: the staged directories, word-split on purpose
(cd "$OUT" && find $dirs -type f -printf '  %p  %s bytes\n' | sort)
