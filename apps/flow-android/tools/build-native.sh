#!/usr/bin/env bash
# Build crates/flow-ffi for arm64-v8a into app/src/main/jniLibs/ (git-ignored).
# Requires: NDK 27.2.12479018, cargo-ndk, rustup target aarch64-linux-android.
# Usage: tools/build-native.sh [--whisper]   (whisper needs cmake for whisper.cpp)
set -euo pipefail
app="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
features="jni"
[[ "${1:-}" == "--whisper" ]] && features="jni,whisper"
cd "$repo"
cargo ndk -t arm64-v8a --platform 33 -o "$app/app/src/main/jniLibs" build -p flow-ffi --release --features "$features"
ls -la "$app/app/src/main/jniLibs/arm64-v8a/"
