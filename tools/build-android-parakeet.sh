#!/usr/bin/env bash
# Cross-build libmurmur_ffi.so (jni + whisper + parakeet) for arm64-v8a into
# target/android-parakeet/jniLibs/arm64-v8a, next to the sherpa-onnx runtime
# libs it needs. Does not touch apps/: copy the output dir into
# apps/murmur-android/app/src/main/jniLibs/ to package it (docs/core-architecture.md
# "Android: Parakeet").
# Requires: NDK 27.2.12479018, cargo-ndk, rustup target aarch64-linux-android,
# cmake + make (whisper), and sherpa-onnx's Android libs:
#   SHERPA_ANDROID_LIBS=<dir with libsherpa-onnx-c-api.so + libonnxruntime.so for arm64-v8a>
#   (apps/murmur-android/tools/build-sherpa-android.sh: no TTS, D80). If unset,
#   sherpa-onnx-sys downloads upstream's Android archive at build time, which
#   links espeak-ng (GPL-3.0): a probe build only, never one to ship.
# Usage: tools/build-android-parakeet.sh [--no-whisper]
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
app="$repo/apps/murmur-android"
out="$repo/target/android-parakeet/jniLibs"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
features="jni,parakeet"
if [[ "${1:-}" != "--no-whisper" ]]; then
  features="$features,whisper"
  export CMAKE_TOOLCHAIN_FILE="$app/tools/android-arm64.toolchain.cmake"
  export GGML_CPU_ARM_ARCH="${MURMUR_ARM_ARCH:-armv8.2-a+dotprod+fp16}"
fi
if [[ -n "${SHERPA_ANDROID_LIBS:-}" ]]; then export SHERPA_ONNX_LIB_DIR="$SHERPA_ANDROID_LIBS"; fi
# Android 15 16 KB pages. The sherpa-onnx/onnxruntime prebuilts are already
# 16 KB aligned (checked by tools/probes/check_parakeet_so.sh).
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
cd "$repo"
cargo ndk -t arm64-v8a --platform 33 --link-libcxx-shared -o "$out" \
  build -p murmur-ffi --release --features "$features"
# The runtime libs murmur-ffi is NEEDED-linked against (not libsherpa-onnx-jni /
# -cxx-api: murmur-ffi uses the C API).
src="${SHERPA_ONNX_LIB_DIR:-$repo/target/aarch64-linux-android/sherpa-onnx-prebuilt/jniLibs/arm64-v8a}"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
  if [[ ! -f "$src/$l" ]]; then
    src="$(dirname "$(find "$repo/target/aarch64-linux-android/sherpa-onnx-prebuilt" -name "$l" -path '*arm64-v8a*' | head -1)")"
  fi
  cp -f "$src/$l" "$out/arm64-v8a/$l"
done
ls -la "$out/arm64-v8a/"
