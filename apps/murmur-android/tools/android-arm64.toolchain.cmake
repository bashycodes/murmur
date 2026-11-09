# Wrapper around the NDK's toolchain file for whisper-rs-sys's CMake build of
# whisper.cpp. whisper-rs-sys forwards only CMAKE_*/GGML_*/WHISPER_* env vars as
# -D defines, so ANDROID_ABI/ANDROID_PLATFORM can't be passed directly; they are
# pinned here instead. Used by tools/build-native.sh via CMAKE_TOOLCHAIN_FILE.
set(ANDROID_ABI arm64-v8a)
set(ANDROID_PLATFORM android-33)
set(ANDROID_STL c++_shared)
# Android 15+: ELF segments must be 16 KB aligned (only matters for shared objects,
# kept for consistency with the final Rust link flags).
set(ANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES ON)
include("$ENV{ANDROID_NDK_HOME}/build/cmake/android.toolchain.cmake")
