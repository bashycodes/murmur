#!/usr/bin/env bash
# Build the native libs for arm64-v8a into app/src/main/jniLibs/ (git-ignored).
#   libflow_ffi.so   crates/flow-ffi (JNI; + whisper.cpp with --whisper; + the LLM loader with --llm)
#   libflow_llm.so   crates/flow-llm (llama.cpp cleanup plugin, --llm only). It must stay a separate
#                    .so: llama.cpp and whisper.cpp bundle different ggml builds (core-architecture.md).
#   libc++_shared.so NDK C++ runtime (whisper.cpp and llama.cpp are C++)
# Requires: NDK 27.2.12479018, cargo-ndk, rustup target aarch64-linux-android,
# cmake + make (whisper / llm only).
# Usage: tools/build-native.sh [--whisper] [--llm]
# Env:   FLOW_ARM_ARCH  ggml -march for aarch64 (default armv8.2-a+dotprod+fp16, any
#                       2018+ big core; use armv8-a for Cortex-A53-only phones).
set -euo pipefail
app="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
features="jni"; llm=0
for a in "$@"; do
  case "$a" in
    --whisper) features="$features,whisper" ;;
    --llm) features="$features,llm"; llm=1 ;;
    *) echo "unknown arg $a" >&2; exit 2 ;;
  esac
done
export GGML_CPU_ARM_ARCH="${FLOW_ARM_ARCH:-armv8.2-a+dotprod+fp16}"
# Android 15 requires 16 KB-aligned LOAD segments; NDK r27 still links 4 KB by default.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cd "$repo"
if [[ $llm == 1 ]]; then
  # llama-cpp-sys-2 drives the NDK CMake toolchain itself (ANDROID_ABI/PLATFORM from
  # cargo-ndk) and turns OpenMP off on Android; it must NOT see our whisper wrapper.
  (unset CMAKE_TOOLCHAIN_FILE
   cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared -o "$app/app/src/main/jniLibs" \
     build -p flow-llm --release --lib)
fi
if [[ ",$features," == *,whisper,* ]]; then
  # whisper-rs-sys runs CMake; without the NDK toolchain file CMake 3.18 can't find
  # the NDK ("Neither the NDK or a standalone toolchain was found").
  export CMAKE_TOOLCHAIN_FILE="$app/tools/android-arm64.toolchain.cmake"
fi
# --link-libcxx-shared: whisper.cpp is C++ and links libc++_shared.so, which must ship too.
cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared -o "$app/app/src/main/jniLibs" \
  build -p flow-ffi --release --features "$features"
ls -la "$app/app/src/main/jniLibs/arm64-v8a/"
