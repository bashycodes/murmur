# phone_lid_2026_11_27: on-phone language ID cost (dated probe)

Measures what D34's `murmur_core::lid::WhisperLid` (whisper tiny q5_1, shrunk
4 s encoder window) costs on an Android phone: load time, per-call time on
3 s of speech, and RSS alone or next to a resident ASR model (`--ballast-mb`).
Deliverable: `docs/findings/2026-11-27-phone-language-id.md` (go/no-go for
Automatic language ID on Android).

Build (same ggml flags as `apps/murmur-android/tools/build-native.sh --flavor hybrid`):

    export ANDROID_NDK_HOME=<ndk 27.2.12479018> ANDROID_NDK_ROOT=$ANDROID_NDK_HOME
    export GGML_CPU_ARM_ARCH=armv8.2-a+dotprod+fp16
    export CMAKE_TOOLCHAIN_FILE=<repo>/apps/murmur-android/tools/android-arm64.toolchain.cmake
    export CARGO_TARGET_DIR=<repo>/target-phone-lid
    cargo ndk -t arm64-v8a --platform 33 build --release -j 2 --offline

Run in `/data/local/tmp` with `libc++_shared.so` (NDK) next to the binary:

    LD_LIBRARY_PATH=. ./phone-lid-probe ggml-tiny-q5_1.bin --threads 4 --iters 10
    LD_LIBRARY_PATH=. ./phone-lid-probe ggml-tiny-q5_1.bin --threads 4 --ballast-mb 900

Synthetic audio by default (LID time does not depend on content); `--wav`
takes a public 16 kHz mono clip. Never use personal recordings.

Host sanity run (i7-6700, loaded machine, 4 threads): load 134 ms,
LID 62 ms mean / 68 ms p90 on 3 s, +71 MB RSS for the loaded model.
