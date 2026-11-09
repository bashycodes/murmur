# murmur (Murmur)

A privacy-first, local-only dictation app: push-to-talk dictation that
transcribes speech (whisper.cpp or Parakeet-TDT), cleans it up (deterministic
post-processing plus an optional local LLM), and pastes the result into the
focused field. There is a Windows shell (Tauri 2) and an Android shell (Kotlin),
both on one shared Rust core.

Inspired by Wispr Flow.

Current state, priorities and the evidence for each working capability:
[`PROJECT_CONTEXT.md`](PROJECT_CONTEXT.md). Design decisions:
[`docs/DECISIONS.md`](docs/DECISIONS.md).

## Layout

```
crates/      Rust workspace: murmur-core (session engine, ASR/LLM seams, VAD,
             post-processing), murmur-ffi (C ABI + JNI), murmur-cli (dev harness),
             murmur-llm (llama.cpp cleanup plugin), murmur-asr-sherpa (Parakeet)
apps/        murmur-windows (Tauri 2 shell), murmur-android (Kotlin shell)
docs/        decisions, core and hybrid architecture, model benchmarks
tools/       fetch-models.sh, build-android-parakeet.sh, probes/
```

## Build and test

* Core: `cargo test --workspace`.
  Full (all engines):
  `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include" SHERPA_ONNX_LIB_DIR=<sherpa-onnx lib dir> cargo test --workspace --all-features`
  and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
* Windows shell: `cd apps/murmur-windows && cargo test`, plus the MSVC clippy
  cross-check described in its README.
* Android shell:
  `cd apps/murmur-android && ANDROID_HOME=~/Android/Sdk ./gradlew testDebugUnitTest assembleDebug`
  (JDK 21). Install guide: `apps/murmur-android/INSTALL.md`.

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

## Your own server

`murmur-server` does the dictation for your PCs and phones over your LAN or
VPN. With Docker, from this repo and a models folder (one folder per model
with its `model.toml`, e.g. a bundle's unzipped `library/`):

```sh
docker build -t murmur-server -f deploy/docker/Dockerfile .
docker volume create murmur_data
docker run -d --name murmur-server --restart unless-stopped -p 0.0.0.0:7313:7313 \
  -v /path/to/models:/models:ro -v murmur_data:/data murmur-server
docker exec murmur-server cat /data/admin.token     # the admin key
```

Then in each app: Settings → Your server: this machine's address
(`hostname -I`), port 7313, the key. Guide: [`docs/server-setup.md`](docs/server-setup.md)
(without Docker: [`docs/server-advanced.md`](docs/server-advanced.md)).

## No internet

Murmur runs on a network with no internet at all: one `murmur-server`
does the dictation, serves the models and hands out one key per device.
Owner guide: [`docs/air-gapped.md`](docs/air-gapped.md).

## Privacy

* No third-party servers: no cloud ASR, no cloud LLM, no telemetry, no
  accounts. Audio and text stay on the user's devices.
* The Android app currently ships without the network permission, and a build gate
  enforces that.
* The optional hybrid mode (`docs/hybrid-architecture.md`) streams to a
  `murmur-server` that the user hosts on their own machine. The server stores no
  audio or text.
* The Windows *Insights* tab (words dictated, speaking speed, streaks,
  estimated time saved, top apps) is computed on the PC from a local
  per-day count file (`%APPDATA%\Murmur\insights.json`, no text). It is
  never sent anywhere and survives deleting History.
