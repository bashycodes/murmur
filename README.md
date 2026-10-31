# murmur (Murmur)

A privacy-first, local-only clone of Wispr Flow: push-to-talk dictation that
transcribes speech (whisper.cpp or Parakeet-TDT), cleans it up (deterministic
post-processing plus an optional local LLM), and pastes the result into the
focused field. There is a Windows shell (Tauri 2) and an Android shell (Kotlin),
both on one shared Rust core.

Current state, priorities and the evidence for each working capability:
[`PROJECT_CONTEXT.md`](PROJECT_CONTEXT.md). Design decisions:
[`docs/DECISIONS.md`](docs/DECISIONS.md).

## Layout

```
crates/      Rust workspace: flow-core (session engine, ASR/LLM seams, VAD,
             post-processing), flow-ffi (C ABI + JNI), flow-cli (dev harness),
             flow-llm (llama.cpp cleanup plugin), flow-asr-sherpa (Parakeet)
apps/        flow-windows (Tauri 2 shell), flow-android (Kotlin shell)
research/    windows/, android/: reverse-engineering findings on the original
             Wispr Flow apps (imported with history via git subtree)
docs/        decisions, core and hybrid architecture, model benchmarks,
             handoffs/ (session handoff notes)
tools/       fetch-models.sh, build-android-parakeet.sh, probes/
```

## Build and test

* Core: `cargo test --workspace`.
  Full (all engines):
  `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include" SHERPA_ONNX_LIB_DIR=<sherpa-onnx lib dir> cargo test --workspace --all-features`
  and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
* Windows shell: `cd apps/flow-windows && cargo test`, plus the MSVC clippy
  cross-check described in its README.
* Android shell:
  `cd apps/flow-android && ANDROID_HOME=~/Android/Sdk ./gradlew testDebugUnitTest assembleDebug`
  (JDK 21). Install guide: `apps/flow-android/INSTALL.md`.

Tests that need real models skip when `models/` is missing.

## Models

Models are not in git (`models/`, `*.bin` and `*.gguf` are ignored). Download
them explicitly:

```
tools/fetch-models.sh                 # whisper ggml-tiny.en.bin (default)
tools/fetch-models.sh --llm 0.5b      # Qwen2.5-Instruct Q4_K_M GGUF for LLM cleanup
tools/fetch-models.sh --parakeet      # Parakeet-TDT-0.6B-v2 int8 (sherpa-onnx)
```

The apps download a model only when you press Download, and with a server
set only from that server's model library.

## No internet

Flow Clone runs on a network with no internet at all: one `flow-server`
does the dictation, serves the models and hands out one key per device.
Owner guide: [`docs/air-gapped.md`](docs/air-gapped.md).

## Privacy

* No third-party servers: no cloud ASR, no cloud LLM, no telemetry, no
  accounts. Audio and text stay on the user's devices.
* The Android app currently ships without the network permission, and a build gate
  enforces that.
* The optional hybrid mode (`docs/hybrid-architecture.md`) streams to a
  `flow-server` that the user hosts on their own machine. The server stores no
  audio or text.
* The Windows *Insights* tab (words dictated, speaking speed, streaks,
  estimated time saved, top apps) is computed on the PC from a local
  per-day count file (`%APPDATA%\FlowClone\insights.json`, no text). It is
  never sent anywhere and survives deleting History.
* The research notes describe the original app's network use for comparison.
  Vendor credentials found during that research are redacted and never
  recorded.
