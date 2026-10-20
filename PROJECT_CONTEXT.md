# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `../windows-research/findings/by-topic/10-clone-spec.md` (§9 = core API), `../android-research` branch `worktree-android-findings` `findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md`.

## What works
Branch `core` (flow-core). Every row below is backed by a passing test (`cargo test --all-features`). Architecture: `docs/core-architecture.md`.

| Capability | Proof (test) |
|---|---|
| §9 v2 session lifecycle: start/push/set_context (late)/partial/commit/cancel; isolation; max_samples; non-blocking push | `crates/flow-core/tests/session.rs` (29) |
| Real local ASR end to end: whisper.cpp tiny.en on jfk.wav → correct text + dictionary applied | `tests/whisper_e2e.rs`, `flow-cli/tests/cli.rs::real_model_on_jfk_if_present` (skip if `models/` missing; `tools/fetch-models.sh`) |
| Deterministic post-processing ported from Wispr: all 108 Windows + 58 Android ground-truth vectors, plus an Android→Windows cross-check | `tests/vectors.rs` |
| LLM seam: 03 §7 prompt, timeout → RAW_TRANSCRIPT, divergence guardrail, snippet/terminal/raw skip | `cleanup.rs` unit tests + session tests (mock cleaner) |
| learn_edit candidates (word alignment) | `learn.rs`, `align.rs` unit tests |
| C ABI + header (C program links the staticlib and runs a session) | `flow-ffi/tests/c_abi.rs`, `c_header.rs` |
| JNI exports match `apps/flow-android/JNI_CONTRACT.md` (`dev.flowclone.core.NativeFlowCore.native*`, CoreJson-shaped payloads) in a real JVM; exported set == contract table | `flow-ffi/tests/jni_smoke.rs` (`--features jni`) |
| `flow-cli transcribe` dev harness (mock / whisper) | `flow-cli/tests/cli.rs` |

E2E timing on this box (i7-6700, 8 logical cores, no GPU, load ≈ 8-14 from other jobs, 4 threads, release build), jfk.wav 11.0 s:
* tiny.en: ASR 1.3-1.6 s, model load 0.1 s warm;
* base.en: ASR 3.0-3.2 s;
* tiny (multilingual, auto-detects `en`): 2.5-3.4 s.

Post-processing takes under 1 ms. With live partials on, commit takes about 1.8 s versus 1.5 s without them, because the in-flight partial decode is aborted.

- `apps/flow-windows` (Windows shell): pure logic (chord matcher, dictation machine w/ Wispr timings, resampler/level meter, paste plan, context/app kinds, prefs, controller loop) — 111 tests green on Linux; Win32 + Tauri code `cargo clippy`-clean for `x86_64-pc-windows-msvc` (incl. `--features app` via check shim). Not yet run on real Windows; uses `StubCore` until flow-core lands (see `apps/flow-windows/README.md`).
- Android shell builds: `cd apps/flow-android && ./gradlew testDebugUnitTest assembleDebug` is green (166 JVM tests; APK without the .so). The merged manifest has no network permission, which a Gradle gate and a unit test enforce.
- Android V3 insertion state machine (07 §3), keep-alive, bubble/visibility policy, dictation controller (cap, timeout, debounce), gesture/position/waveform logic: unit-tested on the JVM with virtual time.
- The Kotlin↔JNI contract (`apps/flow-android/JNI_CONTRACT.md`) is link-tested against a host C stub (`tools/probes/jni_stub/run.sh`).
- Real flow-ffi under the shipped Kotlin binding (no device): an x86_64 host `libflow_ffi.so` (jni,whisper) loaded by `NativeFlowCore` + `CoreJson` handles start → push (jfk.wav in 100 ms frames) → commit and returns the correct transcript, with 0 dropped samples. Mock-engine round trip and the missing-model error path also pass. Run with `apps/flow-android/tools/probes/host_native/run.sh [model]`. Host timings (i7-6700, load 12-20 from other jobs, 4 threads, 11 s audio): tiny.en ASR 5.5-9.5 s, model load 0.2-1.1 s; base.en ASR 12.6-17.6 s; worst push_audio call 4-19 ms under that load. These are not phone numbers.
- arm64-v8a `libflow_ffi.so` with whisper is cross-built (`tools/build-native.sh --whisper`; NDK toolchain wrapper, ggml armv8.2-a+dotprod+fp16) and packaged with `libc++_shared.so`. `tools/probes/check_native_so.sh app-debug.apk` confirms: all 11 contract symbols exported, every LOAD segment 16 KB aligned, NEEDED libs satisfied, and the APK `.so` entries stored and 16 KB zip-aligned. `./gradlew testDebugUnitTest assembleDebug` is green (169 tests, 5 gated skips) and the no-network gate passes. The debug APK with the .so is 9.3 MB (.so 3.9 MB stripped + libc++ 1.3 MB). Install guide: `apps/flow-android/INSTALL.md`.

## Outstanding work — priority order
1. `flow-core` crate — DONE on branch `core` (see What works). Remaining core gaps:
   * llama.cpp `Cleaner` backend (feature `llm` is an empty stub; `llm_path` is ignored);
   * VAD and endpointing;
   * a per-session cache for the DictMatcher (it is rebuilt each commit).
2. Windows shell: wire flow-core in `apps/flow-windows/src/core_adapter.rs`; run manual checklist in `apps/flow-windows/README.md` on real Windows; gaps there (clickable hands-free buttons, UIPI pre-check, hook re-install).
3. Android shell. Code is done (`apps/flow-android/`). Remaining:
   a. JNI + arm64 packaging: DONE (see What works). The .so has never been loaded on bionic. The first device run must confirm `Native engine: flow-ffi 0.1.0`.
   b. **Device verification (needs the user's phone).** Full checklist: `apps/flow-android/INSTALL.md` §"Device test checklist". In order:
      - grant mic, overlay, a11y and notifications;
      - mock engine: tap and hold dictation into Notes, WhatsApp, Chrome URL bar, a notification reply and a password field (must be hidden);
      - check that the banking app is hidden;
      - insertion fallbacks: commitText vs ACTION_SET_TEXT vs clipboard (logcat tag `FlowA11y` prints the step trail);
      - FGS microphone promotion on Android 14+;
      - the 5-min cap;
      - SAF import of `ggml-base.en-q5_1.bin` (quantised models are untested; tiny.en/base.en f16 are proven on host), then real ASR latency; no SIGILL (dotprod/fp16 build);
      - 16 KB-page device (Android 15): installs without the compat dialog;
      - MIC vs VOICE_RECOGNITION.
   c. P1 (07 / 10 §6): shake/snooze, bubble minimise, watchdog/boot recovery, battery-optimisation onboarding, VAD trim, partials.
4. P1: LLM cleanup, context, learn-from-edits.
