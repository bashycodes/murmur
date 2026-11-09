#!/usr/bin/env bash
# Stage host-built murmur-server binaries for the container image, so
#   docker build -f deploy/docker/Dockerfile --build-arg SOURCE=prebuilt .
# needs no network and no compiler (docs/container.md "Air-gapped build").
#
#   tools/container/stage-prebuilt.sh [--build] [--sherpa-lib DIR] [--cuda-llm FILE]
#
#   --build           first build murmur-server (features whisper,parakeet,tls,llm)
#                     and the murmur-llm plugin, release, --offline
#   --sherpa-lib DIR  dir with libsherpa-onnx-c-api.so + libonnxruntime.so, the no-TTS
#                     build (sherpa-onnx-v1.13.8-linux-x64-shared-no-tts-lib/lib; D80)
#                     (default $SHERPA_ONNX_LIB_DIR, else target/release)
#   --cuda-llm FILE   a CUDA build of libmurmur_llm.so (`cargo build --release
#                     -p murmur-llm --features cuda`), staged as
#                     libmurmur_llm_cuda.so for `--target cuda`
#
# The image runs Ubuntu 24.04 (glibc 2.39): build on a host with that glibc
# or older. Writes deploy/docker/prebuilt/{bin,lib} (git-ignored).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}/release"
OUT="$ROOT/deploy/docker/prebuilt"
SHERPA_LIB="${SHERPA_ONNX_LIB_DIR:-}"
BUILD=0
CUDA_LLM=""

die() { echo "stage-prebuilt.sh: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
    case "$1" in
        --build) BUILD=1 ;;
        --sherpa-lib) SHERPA_LIB="${2:?--sherpa-lib DIR}"; shift ;;
        --cuda-llm) CUDA_LLM="${2:?--cuda-llm FILE}"; shift ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done

if [ "$BUILD" = 1 ]; then
    [ -n "$SHERPA_LIB" ] || die "--build needs --sherpa-lib DIR or SHERPA_ONNX_LIB_DIR (offline build)"
    GCC_INC="$(gcc -print-file-name=include)"
    (cd "$ROOT" &&
        SHERPA_ONNX_LIB_DIR="$SHERPA_LIB" cargo build --release --offline -p murmur-server \
            --no-default-features --features whisper,parakeet,tls,llm &&
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

rm -rf "${OUT:?}/bin" "${OUT:?}/lib"
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
    install -m 0644 "$CUDA_LLM" "$OUT/lib/libmurmur_llm_cuda.so"
fi
echo "staged in ${OUT#"$ROOT"/}:"
(cd "$OUT" && find bin lib -type f -printf '  %p  %s bytes\n' | sort)
