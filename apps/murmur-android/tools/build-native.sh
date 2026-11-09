#!/usr/bin/env bash
# Build the arm64-v8a native libs for one APK flavour into app/src/<flavour>/jniLibs/ (git-ignored).
# D17 §1.4:
#   --flavor local   libflow_ffi.so (jni,whisper,llm,parakeet) + libflow_llm.so + libc++_shared.so
#                    + libsherpa-onnx-c-api.so + libonnxruntime.so          (no network code)
#   --flavor hybrid  as local, flow-ffi additionally with `remote,cloud,admin,library`
#   --flavor remote  libflow_ffi.so (jni,remote,cloud,admin,library) only: no ASR/LLM, no C++ runtime
#   `library` (D41) = model downloads from the set server's library (flow_remote::library);
#   never in `local`, which stays import-only with no network code (D9).
#   `cloud` (D18, docs/cloud-stt.md §9) = opt-in third-party STT over HTTPS (reqwest + rustls,
#   webpki roots; no OpenSSL). Never in `local` (C2; tools/check-build-matrix.sh + the APK gate).
#   libflow_llm.so   crates/flow-llm (llama.cpp cleanup plugin). It must stay a separate .so:
#                    llama.cpp and whisper.cpp bundle different ggml builds (core-architecture.md).
# Options: --no-llm, --no-parakeet (smaller/faster local or hybrid builds).
# Legacy: --whisper [--llm] without --flavor builds the old whisper(+llm) set into the local flavour.
# Requires: NDK 27.2.12479018, cargo-ndk, rustup target aarch64-linux-android, cmake + make
# (whisper / llm), and for parakeet sherpa-onnx's Android libs without text to speech (never
# downloaded here; D80: upstream's Android release links espeak-ng, GPL-3.0):
#   SHERPA_ANDROID_LIBS=<dir with libsherpa-onnx-c-api.so + libonnxruntime.so for arm64-v8a>
#   (the output dir of tools/build-sherpa-android.sh).
# Env: FLOW_ARM_ARCH  ggml -march for aarch64 (default armv8.2-a+dotprod+fp16, any 2018+ big
#                     core; use armv8-a for Cortex-A53-only phones).
set -euo pipefail
app="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
flavor=""; whisper=0; llm=0; parakeet=0; remote=0; cloud=0; library=0; no_llm=0; no_parakeet=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --flavor) flavor="$2"; shift ;;
    --flavor=*) flavor="${1#*=}" ;;
    --whisper) whisper=1 ;;
    --llm) llm=1 ;;
    --no-llm) no_llm=1 ;;
    --no-parakeet) no_parakeet=1 ;;
    *) echo "unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done
case "$flavor" in
  local) whisper=1; llm=1; parakeet=1 ;;
  hybrid) whisper=1; llm=1; parakeet=1; remote=1; cloud=1; library=1 ;;
  remote) remote=1; cloud=1; library=1 ;;
  "") flavor=local ;;  # legacy flags
  *) echo "--flavor must be local, hybrid or remote" >&2; exit 2 ;;
esac
[[ $no_llm == 1 ]] && llm=0
[[ $no_parakeet == 1 ]] && parakeet=0
features="jni"
[[ $whisper == 1 ]] && features="$features,whisper"
[[ $llm == 1 ]] && features="$features,llm"
[[ $parakeet == 1 ]] && features="$features,parakeet"
[[ $remote == 1 ]] && features="$features,remote"
[[ $cloud == 1 ]] && features="$features,cloud"
# D39 "Users & keys": the admin API client rides on `remote`.
[[ $remote == 1 ]] && features="$features,admin"
[[ $library == 1 ]] && features="$features,library"
out="$app/app/src/$flavor/jniLibs"
rm -rf "$out/arm64-v8a"; mkdir -p "$out/arm64-v8a"
echo "flavor=$flavor features=$features -> $out/arm64-v8a"

export GGML_CPU_ARM_ARCH="${FLOW_ARM_ARCH:-armv8.2-a+dotprod+fp16}"
# Android 15 requires 16 KB-aligned LOAD segments; NDK r27 still links 4 KB by default.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cxx=()
[[ $whisper == 1 || $parakeet == 1 || $llm == 1 ]] && cxx=(--link-libcxx-shared)
cd "$repo"
if [[ $llm == 1 ]]; then
  # llama-cpp-sys-2 drives the NDK CMake toolchain itself (ANDROID_ABI/PLATFORM from
  # cargo-ndk) and turns OpenMP off on Android; it must NOT see our whisper wrapper.
  (unset CMAKE_TOOLCHAIN_FILE
   cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared -o "$out" \
     build -p flow-llm --release --lib)
fi
if [[ $whisper == 1 ]]; then
  # whisper-rs-sys runs CMake; without the NDK toolchain file CMake 3.18 can't find
  # the NDK ("Neither the NDK or a standalone toolchain was found").
  export CMAKE_TOOLCHAIN_FILE="$app/tools/android-arm64.toolchain.cmake"
fi
if [[ $parakeet == 1 ]]; then
  if [[ -z "${SHERPA_ANDROID_LIBS:-}" || ! -f "$SHERPA_ANDROID_LIBS/libsherpa-onnx-c-api.so" ]]; then
    echo "parakeet needs SHERPA_ANDROID_LIBS=<tools/build-sherpa-android.sh output> (or pass --no-parakeet)" >&2
    exit 2
  fi
  # D80: refuse a sherpa-onnx build with TTS (espeak-ng) before spending a build on it.
  "$repo/tools/checks/no-tts-libs.sh" "$SHERPA_ANDROID_LIBS/libsherpa-onnx-c-api.so" "$SHERPA_ANDROID_LIBS/libonnxruntime.so" || exit 2
  # Point sherpa-onnx-sys at the prebuilts so it never downloads.
  export SHERPA_ONNX_LIB_DIR="$SHERPA_ANDROID_LIBS"
fi
# --link-libcxx-shared: whisper.cpp is C++ and links libc++_shared.so, which must ship too.
cargo ndk -t arm64-v8a --platform 33 "${cxx[@]}" -o "$out" \
  build -p flow-ffi --release --features "$features"
if [[ $parakeet == 1 ]]; then
  # The runtime libs flow-ffi is NEEDED-linked against (not libsherpa-onnx-jni / -cxx-api).
  for l in libsherpa-onnx-c-api.so libonnxruntime.so; do cp -f "$SHERPA_ANDROID_LIBS/$l" "$out/arm64-v8a/$l"; done
fi
ls -la "$out/arm64-v8a/"
