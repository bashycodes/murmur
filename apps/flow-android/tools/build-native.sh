#!/usr/bin/env bash
# Build crates/flow-ffi for arm64-v8a into app/src/main/jniLibs/ (git-ignored).
# Requires: NDK 27.2.12479018, cargo-ndk, rustup target aarch64-linux-android,
# cmake + make (whisper only).
# Usage: tools/build-native.sh [--whisper]
# Env:   FLOW_ARM_ARCH  ggml -march for aarch64 (default armv8.2-a+dotprod+fp16, any
#                       2018+ big core; use armv8-a for Cortex-A53-only phones).
set -euo pipefail
app="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
features="jni"
if [[ "${1:-}" == "--whisper" ]]; then
  features="jni,whisper"
  # whisper-rs-sys runs CMake; without the NDK toolchain file CMake 3.18 can't find
  # the NDK ("Neither the NDK or a standalone toolchain was found").
  export CMAKE_TOOLCHAIN_FILE="$app/tools/android-arm64.toolchain.cmake"
  export GGML_CPU_ARM_ARCH="${FLOW_ARM_ARCH:-armv8.2-a+dotprod+fp16}"
fi
# Android 15 requires 16 KB-aligned LOAD segments; NDK r27 still links 4 KB by default.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cd "$repo"
# --link-libcxx-shared: whisper.cpp is C++ and links libc++_shared.so, which must ship too.
cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared -o "$app/app/src/main/jniLibs" \
  build -p flow-ffi --release --features "$features"
ls -la "$app/app/src/main/jniLibs/arm64-v8a/"
