#!/usr/bin/env bash
# Build sherpa-onnx's arm64-v8a C API library with text-to-speech OFF (D80): upstream's
# sherpa-onnx-v1.13.8-android.tar.bz2 links espeak-ng (GPL-3.0) into libsherpa-onnx-c-api.so
# for its TTS module, which we never use, and upstream ships no no-TTS Android build.
# Same source tag, same ONNX Runtime (1.28.2, upstream's Android prebuilt) and the same
# CMake options as upstream's build-android-arm64-v8a.sh, plus 16 KB page alignment.
#
#   tools/build-sherpa-android.sh OUT_DIR
#
# Writes OUT_DIR/libsherpa-onnx-c-api.so + OUT_DIR/libonnxruntime.so (the SHERPA_ANDROID_LIBS
# that tools/build-native.sh takes) and fails if espeak-ng or piper code is in them.
# Needs: network (GitHub: the sherpa-onnx source, ONNX Runtime and CMake's FetchContent deps),
# git, cmake, make, unzip, ANDROID_NDK_HOME (default NDK 27.2.12479018). Work dir:
# $SHERPA_BUILD_DIR (default ~/.cache/murmur-sherpa-android-build). Jobs: $JOBS (default 2).
set -euo pipefail
out="${1:?usage: build-sherpa-android.sh OUT_DIR}"
VERSION=1.13.8
COMMIT=11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf
ORT=1.28.2
ORT_SHA256=01518867f78241138b6aa25925802e843a4fa9085af8d303d49e35bbb52aff4d
ndk="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
work="${SHERPA_BUILD_DIR:-$HOME/.cache/murmur-sherpa-android-build}"
jobs="${JOBS:-2}"
die() { echo "build-sherpa-android.sh: $*" >&2; exit 1; }
[[ -f "$ndk/build/cmake/android.toolchain.cmake" ]] || die "no NDK at $ndk (set ANDROID_NDK_HOME)"
mkdir -p "$work" "$out"
src="$work/sherpa-onnx-$VERSION"
if [[ ! -d "$src/.git" ]]; then
  git clone -q --depth 1 --branch "v$VERSION" https://github.com/k2-fsa/sherpa-onnx "$src"
fi
[[ "$(git -C "$src" rev-parse HEAD)" == "$COMMIT" ]] || die "$src is not v$VERSION ($COMMIT)"

ort="$work/onnxruntime-android-$ORT"
if [[ ! -f "$ort/jni/arm64-v8a/libonnxruntime.so" ]]; then
  zip="$work/onnxruntime-android-$ORT.zip"
  curl -fsSL -o "$zip.part" "https://github.com/csukuangfj/onnxruntime-libs/releases/download/v$ORT/onnxruntime-android-$ORT.zip"
  [[ "$(sha256sum "$zip.part" | cut -d' ' -f1)" == "$ORT_SHA256" ]] || { rm -f "$zip.part"; die "ONNX Runtime zip: SHA-256 mismatch"; }
  rm -rf "$ort"; mkdir -p "$ort"
  unzip -q "$zip.part" -d "$ort"
  rm -f "$zip.part"
  # The zip has one top folder or none, depending on the release.
  if [[ ! -d "$ort/jni" ]]; then inner=$(find "$ort" -mindepth 1 -maxdepth 1 -type d | head -1); mv "$inner"/* "$ort"/; fi
fi
export SHERPA_ONNXRUNTIME_LIB_DIR="$ort/jni/arm64-v8a/"
export SHERPA_ONNXRUNTIME_INCLUDE_DIR="$ort/headers/"

build="$work/build-arm64-v8a-no-tts"
mkdir -p "$build"
(cd "$build" && cmake -DCMAKE_TOOLCHAIN_FILE="$ndk/build/cmake/android.toolchain.cmake" \
    -DSHERPA_ONNX_ENABLE_TTS=OFF \
    -DSHERPA_ONNX_ENABLE_SPEAKER_DIARIZATION=ON \
    -DSHERPA_ONNX_ENABLE_BINARY=OFF \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=ON \
    -DSHERPA_ONNX_ENABLE_PYTHON=OFF \
    -DSHERPA_ONNX_ENABLE_TESTS=OFF \
    -DSHERPA_ONNX_ENABLE_CHECK=OFF \
    -DSHERPA_ONNX_ENABLE_PORTAUDIO=OFF \
    -DSHERPA_ONNX_ENABLE_JNI=OFF \
    -DSHERPA_ONNX_LINK_LIBSTDCPP_STATICALLY=OFF \
    -DSHERPA_ONNX_ENABLE_C_API=ON \
    -DSHERPA_ONNX_ENABLE_RKNN=OFF \
    -DSHERPA_ONNX_ENABLE_QNN=OFF \
    -DCMAKE_INSTALL_PREFIX=./install \
    -DANDROID_ABI=arm64-v8a \
    -DANDROID_PLATFORM=android-21 \
    -DANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES=ON \
    -DCMAKE_SHARED_LINKER_FLAGS="-Wl,-z,max-page-size=16384" \
    "$src" > cmake.log && make -j "$jobs" > make.log && make install/strip > install.log) ||
  die "build failed (logs in $build/*.log)"
cp -f "$build/install/lib/libsherpa-onnx-c-api.so" "$out/"
cp -f "$SHERPA_ONNXRUNTIME_LIB_DIR/libonnxruntime.so" "$out/"
"$(dirname "$0")/../../../tools/checks/no-tts-libs.sh" "$out/libsherpa-onnx-c-api.so" "$out/libonnxruntime.so"
ls -la "$out"
