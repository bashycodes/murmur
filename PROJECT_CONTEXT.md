# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `../windows-research/findings/by-topic/10-clone-spec.md` (§9 = core API), `../android-research` branch `worktree-android-findings` `findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md`.

## What works
Branch `core` (flow-core). Every row below is backed by a passing test (`cargo test --all-features`). Architecture: `docs/core-architecture.md`.

| Capability | Proof (test) |
|---|---|
| §9 v2 session lifecycle: start/push/set_context (late)/partial/commit/cancel; isolation; max_samples; non-blocking push | `crates/flow-core/tests/session.rs` (29) |
| Real local ASR end to end: whisper.cpp tiny.en on jfk.wav → correct text + dictionary applied | `tests/whisper_e2e.rs`, `flow-cli/tests/cli.rs::real_model_on_jfk_if_present` (skip if `models/` missing; `tools/fetch-models.sh`) |
| Deterministic post-processing ported from Wispr: all 108 Windows + 58 Android ground-truth vectors, plus an Android→Windows cross-check | `tests/vectors.rs` |
| LLM seam: 03 §7 prompt (KV-reuse layout + 6 few-shot turns), timeout → RAW_TRANSCRIPT + backend cancel flag, preamble scrubbing, divergence guardrail, snippet/terminal/raw skip | `cleanup.rs` unit tests + session tests (mock cleaner) |
| Real local LLM cleanup: llama.cpp `LlamaCleaner` (flow-llm plugin, loaded once in `load_models`, warm KV prefix, greedy, 2×input+32, per-token deadline/cancel, unload frees) on 8 messy dictations through `FlowCore`: safety contract at the 1.5 s budget; quality 4/8 (Qwen2.5-0.5B) / 8/8 (1.5B) at a generous budget | `flow-core/tests/llm_e2e.rs` (skip if model/plugin missing; `tools/fetch-models.sh --llm`), `flow-llm` unit tests, `llama.rs` loader tests |
| `flow-cli clean --llm m.gguf TEXT…`: FORMATTED output, warm-prefix prefill counters; `transcribe --model whisper --llm gguf`: real ASR → real LLM in one process (whisper ggml linked, llama ggml in the plugin) | `flow-cli/tests/cli.rs::clean_with_real_llm_if_present`, `whisper_then_llm_in_one_process_if_present` (skip if models missing) |
| learn_edit candidates (word alignment) | `learn.rs`, `align.rs` unit tests |
| C ABI + header (C program links the staticlib and runs a session) | `flow-ffi/tests/c_abi.rs`, `c_header.rs` |
| JNI exports match `apps/flow-android/JNI_CONTRACT.md` (`dev.flowclone.core.NativeFlowCore.native*`, CoreJson-shaped payloads) in a real JVM; exported set == contract table | `flow-ffi/tests/jni_smoke.rs` (`--features jni`) |
| `flow-cli transcribe` dev harness (mock / whisper) | `flow-cli/tests/cli.rs` |

E2E timing on this box (i7-6700, 8 logical cores, no GPU, load ≈ 8-14 from other jobs, 4 threads, release build), jfk.wav 11.0 s:
* tiny.en: ASR 1.3-1.6 s, model load 0.1 s warm;
* base.en: ASR 3.0-3.2 s;
* tiny (multilingual, auto-detects `en`): 2.5-3.4 s.

Post-processing takes under 1 ms. With live partials on, commit takes about 1.8 s versus 1.5 s without them, because the in-flight partial decode is aborted.

Branch `integration`: Windows shell (`apps/flow-windows`) wired to the real core. Proof = `cd apps/flow-windows && cargo test` (146 tests: 139 lib + 7 integration).

| Capability | Proof (test) |
|---|---|
| Shell drives real flow-core: PTT → capture → push_audio → commit → paste plan, with the dictionary from prefs applied by the core | `apps/flow-windows/tests/core_integration.rs::mock_asr_audio_to_commit_to_paste_and_history` |
| Real whisper.cpp through the shell: tiny.en on jfk.wav → "ask not what your country…" pasted, `lang=en` in history | `core_integration.rs::real_tiny_model_on_jfk_pastes_the_speech` (skips without `models/`) |
| §9 status handling: EMPTY pastes nothing; press_enter → Enter after paste; missing model → error pill naming the path, never downloads | `core_integration.rs` (silence / press_enter / missing_model), `core_adapter.rs` tests |
| Live partials → overlay preview, cleared when listening ends | `core_integration.rs::live_partials_reach_the_overlay_preview` |
| Local SQLite history (10-clone-spec §2 columns, no audio column), keep/24h/never retention; paste-last (Shift+Alt+Z) from it | `history.rs` tests, `core_integration.rs` (paste-last, retention never) |
| Hook watchdog (LowLevelHooksTimeout removal → probe → re-install); UIPI integrity pre-check; clickable hands-free ✕/✓ hit-test; {{BREAK}}/{{TAB}} paste segments | `hotkeys/watchdog.rs`, `paste/mod.rs` (`uipi_blocks`, `segments`), `overlay.rs` tests. Win32 sides are only `cargo clippy`-clean for `x86_64-pc-windows-msvc --no-default-features --features app` (whisper-rs-sys can't cross-build; see README) |

Not yet run on real Windows. The manual checklist is in `apps/flow-windows/README.md`. Core requests: `docs/core-change-requests.md`.

Branch `android-int` (merged): Android shell + real core.

- Android shell builds: `cd apps/flow-android && ./gradlew testDebugUnitTest assembleDebug` is green (166 JVM tests; APK without the .so). The merged manifest has no network permission, which a Gradle gate and a unit test enforce.
- Android V3 insertion state machine (07 §3), keep-alive, bubble/visibility policy, dictation controller (cap, timeout, debounce), gesture/position/waveform logic: unit-tested on the JVM with virtual time.
- The Kotlin↔JNI contract (`apps/flow-android/JNI_CONTRACT.md`) is link-tested against a host C stub (`tools/probes/jni_stub/run.sh`).
- Real flow-ffi under the shipped Kotlin binding (no device): an x86_64 host `libflow_ffi.so` (jni,whisper) loaded by `NativeFlowCore` + `CoreJson` handles start → push (jfk.wav in 100 ms frames) → commit and returns the correct transcript, with 0 dropped samples. Mock-engine round trip and the missing-model error path also pass. Run with `apps/flow-android/tools/probes/host_native/run.sh [model]`. Host timings (i7-6700, load 12-20 from other jobs, 4 threads, 11 s audio): tiny.en ASR 5.5-9.5 s, model load 0.2-1.1 s; base.en ASR 12.6-17.6 s; worst push_audio call 4-19 ms under that load. These are not phone numbers.
- arm64-v8a `libflow_ffi.so` with whisper is cross-built (`tools/build-native.sh --whisper`; NDK toolchain wrapper, ggml armv8.2-a+dotprod+fp16) and packaged with `libc++_shared.so`. `tools/probes/check_native_so.sh app-debug.apk` confirms: all 11 contract symbols exported, every LOAD segment 16 KB aligned, NEEDED libs satisfied, and the APK `.so` entries stored and 16 KB zip-aligned. `./gradlew testDebugUnitTest assembleDebug` is green (169 tests, 5 gated skips) and the no-network gate passes. The debug APK with the .so is 9.3 MB (.so 3.9 MB stripped + libc++ 1.3 MB). Install guide: `apps/flow-android/INSTALL.md`.

Branch `android-p1`: Android P1 (robustness, preview, LLM, editors). `./gradlew testDebugUnitTest assembleDebug` green: 238 JVM tests (9 gated skips), no-network gate passes. Debug APK with all three .so: 14.2 MB.

| Capability | Proof (test) |
|---|---|
| Shake detector (2 impulses ≤ 1.2 s, 100 ms cooldown), bubble presence (minimise after 5 s, snooze 10 min from Ready only / not in Repair, shake un-snooze / 10 s reveal, lock-screen ignore), snooze-target geometry | `bubble/BubblePresenceTest.kt` (virtual time) |
| Ghost a11y detection (enabled but unbound after 15 s grace, or heartbeat stale > 60 s with the screen on), enabled-services parsing, FlowService watchdog spacing ≥ 5 s, OEM autostart guidance | `health/HealthTest.kt` |
| Live partials polled from the core while recording (only when `partial_interval_ms` is set), cleared on submit/cancel; "can't hear you" after 3 s leading silence (Wispr HeadSilenceTracker threshold); EMPTY → transient "no speech detected" | `session/DictationControllerTest.kt` (+6), `bubble/PreviewLogicTest.kt` |
| LLM settings (default off, LIGHT when on, 3 s budget), EnginePlan with ASR-only fallback when the GGUF/plugin fails, ggml-vs-GGUF sniffing, dictionary edit/validate/filter, history search/raw-vs-formatted | `data/EditorLogicTest.kt` |
| arm64 `libflow_llm.so` cross-built (`tools/build-native.sh --whisper --llm`; llama-cpp-sys-2 drives the NDK toolchain, OpenMP off, ggml armv8.2-a+dotprod+fp16) and packaged; exports exactly the 6 `flow_llm_*` symbols, NEEDED = libc++_shared/libc/libm/libdl, 16 KB aligned; flow-ffi built with the `llm` loader | `tools/probes/check_native_so.sh app-debug.apk` |
| Real LLM cleanup through the shipped Kotlin binding on the host JVM (flow-ffi jni,whisper,llm + host flow-llm plugin, Qwen2.5-0.5B Q4_K_M, settings path = shell's own `Settings.toSessionConfig`): FORMATTED ("so um i think … uh bring the slides" → "So, I think we should meet tomorrow at noon and bring the slides."), llm 2.1 s, load 15.9 s at load ~17; LLM off → RAW with llm_ms 0; bad plugin path → EnginePlan falls back to ASR-only; whisper tiny.en + LLM in one JVM: FORMATTED (asr 9.4 s, llm 3.3 s) | `NativeFlowCoreHostTest` via `tools/probes/host_native/run.sh` (7/7) |

Wired on Android, unverified on a device: snooze target + notification undo, shake listener (screen-on only), 36dp minimised dot, Repair bubble + ghost notification text, a11y 15 s heartbeat + FlowService respawn, `BootReceiver` (BOOT_COMPLETED / MY_PACKAGE_REPLACED / USER_UNLOCKED), USER_PRESENT re-attach, battery-exemption + OEM steps in onboarding, preview window, GGUF import, dictionary editor, history screen, history DB v2 (fallback reason + per-stage timings).

## Outstanding work — priority order
1. `flow-core` crate — DONE on branch `core` (see What works). Remaining core gaps:
   * LLM latency on CPU: 1.5B is 8/8 but ~1.1-5 s per call on this shared box (0.5B: 4/8, 0.75-2.7 s). Next levers: prefill the context + partial transcript into the KV cache while the user speaks; a LoRA on (raw → formatted) pairs for 0.5B (03 §7.5); pick the shipped model from `docs/model-benchmarks.md`. Shells must ship `libflow_llm` next to the exe (Windows) / in jniLibs (Android: packaged on `android-p1`, host-JVM-proven, not yet loaded on bionic);
   * VAD and endpointing;
   * a per-session cache for the DictMatcher (it is rebuilt each commit).
2. Windows shell: flow-core wired (branch `integration`). Remaining:
   * a real-Windows build and run of the README manual checklist (needs the user's Windows box);
   * a history UI and "copy last transcript";
   * learn-from-edits (`edited` column);
   * the mouse/Caps Lock shortcuts.
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
   c. P1 (07 / 10 §6): DONE on branch `android-p1` except on-device proof — shake/snooze, minimise, watchdog/boot/unlock recovery, ghost check, battery + OEM onboarding, live partials + silence hints, LLM cleanup packaging + toggle + GGUF import, dictionary editor, history screen. Device checks: INSTALL.md §"Device test checklist" (AI cleanup, Robustness, Live preview, Dictionary & history). Open questions only a device answers: does USER_UNLOCKED reach the manifest receiver; partial-decode CPU cost on a phone (preview default on); LLM latency/RAM with whisper + 0.5B both resident; which OEM deep links resolve.
   d. Not done: VAD head/tail trim (core side; the shell only reports silence), `shrinkBubbleInSearch`, snooze-target gravity pull, learn-from-edits.
4. P1: context awareness, learn-from-edits.
