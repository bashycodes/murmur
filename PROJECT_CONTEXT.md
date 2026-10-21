# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `research/windows/findings/by-topic/10-clone-spec.md` (§9 = core API), `research/android/findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md` (D1-D16). Core architecture + "Changelog for shells": `docs/core-architecture.md`. Core requests from the shells: `docs/core-change-requests.md`. Model choices: `docs/model-benchmarks.md`.

Branch `integration` holds everything: `core`, `core-p1`, `parakeet`, `win-p1`, `android-int` and `android-p1` are merged into it.

## Layout

```
crates/
  flow-core/        lib: session engine, ASR/LLM seams, VAD, hallucination guard, spoken cleanup, deterministic post-processing, paste segments
  flow-ffi/         C ABI (cdylib/staticlib, include/flow_core.h) + JNI (feature `jni`)
  flow-cli/         dev harness: `flow-cli transcribe` (WAV → full pipeline), `flow-cli clean` (LLM cleanup on text)
  flow-llm/         llama.cpp cleanup engine as a cdylib plugin (libflow_llm.so / flow_llm.dll), loaded at runtime (D10)
  flow-asr-sherpa/  Parakeet-TDT ASR via the official sherpa-onnx crate (flow-core feature `parakeet`, D12)
apps/
  flow-windows/     Tauri 2 shell (own cargo workspace; MSVC cross-check shims in tools/check-shims)
  flow-android/     Kotlin shell (Gradle; JNI contract JNI_CONTRACT.md, install guide INSTALL.md)
docs/               DECISIONS, core-architecture, core-change-requests, model-benchmarks, bench/
tools/              fetch-models.sh, build-android-parakeet.sh, probes/
```

## How to verify

* Core: `cargo test --workspace`. Full: `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include" SHERPA_ONNX_LIB_DIR=<sherpa-onnx lib dir> cargo test --workspace --all-features` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` (llama-cpp-sys-2 needs the BINDGEN flag even for the default build on this box). Real-model tests skip without `models/` (`tools/fetch-models.sh [--llm] [--parakeet]`).
* Windows shell: `cd apps/flow-windows && cargo test` (184: 171 lib + 13 integration), plus the MSVC clippy cross-check in its README.
* Android shell: `cd apps/flow-android && ANDROID_HOME=~/Android/Sdk ./gradlew testDebugUnitTest assembleDebug` (Gradle daemon on JDK 21): 238 JVM tests, 9 gated skips.

## What works

### Core (flow-core and friends)
Every row is backed by a passing test (`cargo test --all-features`).

| Capability | Proof (test) |
|---|---|
| §9 v2 session lifecycle: start/push/set_context (late)/partial/commit/cancel; isolation; max_samples; non-blocking push | `crates/flow-core/tests/session.rs` |
| Real local ASR end to end: whisper.cpp tiny.en on jfk.wav → correct text + dictionary applied | `tests/whisper_e2e.rs`, `flow-cli/tests/cli.rs::real_model_on_jfk_if_present` (skip if `models/` missing; `tools/fetch-models.sh`) |
| Parakeet-TDT-0.6B-v2 int8 ASR engine (D12; sherpa-onnx, crate `flow-asr-sherpa`, `asr_engine:"parakeet"`) in the same binary as whisper: jfk.wav exact, dictionary → sherpa hotwords ("Meadocroft" → "Meadowcroft"), silence → EMPTY, missing model dir → `ModelNotFound`; `flow-cli transcribe --engine parakeet --model <dir>`. Spot check: WER 1.2% on the 40-clip set (= bench), RTF 0.10-0.16; preload 3 s warm / 10-11 s cold | `flow-core/tests/parakeet_e2e.rs`, `flow-cli/tests/cli.rs::parakeet_engine_on_jfk_if_present`, `flow-asr-sherpa/tests/model.rs` (skip without `tools/fetch-models.sh --parakeet`) |
| Deterministic post-processing ported from Wispr: all 108 Windows + 58 Android ground-truth vectors, plus an Android→Windows cross-check | `tests/vectors.rs` |
| Deterministic spoken cleanup (D15: punctuation words, fillers, stutters, backtracking, money/time/date ITN), default on (`spoken_cleanup`): bench 20 cases 0.710 → 0.920 mean similarity with no LLM | `postprocess/spoken.rs` units, `tests/spoken_bench.rs` |
| Energy+ZCR VAD (D16): room noise → EMPTY without ASR (saves 0.7 s tiny / 3.9 s base per empty press), edges trimmed; optional whisper.cpp Silero (`vad_model_path`) | `vad.rs` units, `session.rs::vad_*`, `tests/vad_bench.rs` (ignored, numbers in core-architecture) |
| Whisper silence-hallucination guard (D16: "Thank you.", "you", credits) gated on VAD speech / no_speech_prob | `tests/hallucination_vectors.rs`, `session.rs::stock_phrase_*` |
| LLM seam: timeout → RAW_TRANSCRIPT + backend cancel flag, preamble scrubbing, snippet/terminal/raw skip, static KV-reuse prefix. The prompt is the short few-shot one (D14; it replaced the 03 §7 rule prompt with 6 few-shot turns); the edit-distance divergence check is off by default (`divergence_threshold`) | `cleanup.rs` unit tests + session tests (mock cleaner) |
| LLM guardrail = support + coverage (`guard::check`, D13): 79/79 good kept, 14/14 harmful rejected on 120 judged bench outputs (old ratio: 75/79, 4/14) | `tests/guard_bench.rs` |
| Real local LLM cleanup: llama.cpp `LlamaCleaner` (flow-llm plugin, loaded once in `load_models`, warm KV prefix, greedy, 2×input+32, per-token deadline/cancel, unload frees) on 8 messy dictations through `FlowCore`: safety contract at the 1.5 s budget. Long prompt: quality 4/8 (Qwen2.5-0.5B) / 8/8 (1.5B) at a generous budget. Short few-shot prompt, 1.5B: 6/8 (misses: dropped "thanks" → guard fell back to the correct deterministic text; "first … second … third" not made a list); 5/8 FORMATTED at the real 1.5 s budget (p50 1.0 s); bench 20 cases 0.919 mean similarity, no fallbacks | `flow-core/tests/llm_e2e.rs` (skip if model/plugin missing; `tools/fetch-models.sh --llm`; bench opt-in `FLOW_LLM_BENCH=1`), `flow-llm` unit tests, `llama.rs` loader tests |
| LLM prefill-ahead while speaking (start, set_context, partials; coalesced, preempted by commit): commit prefill 81 → 10-29 tokens (≈ 0.7-1.4 s saved on this box) | `session.rs::llm_prefill_*`, measurement in `docs/bench/llm-prefill-2026-10-20/` |
| `flow-cli clean --llm m.gguf TEXT…`: FORMATTED output, warm-prefix prefill counters; `transcribe --model whisper --llm gguf`: real ASR → real LLM in one process (whisper ggml linked, llama ggml in the plugin) | `flow-cli/tests/cli.rs::clean_with_real_llm_if_present`, `whisper_then_llm_in_one_process_if_present` (skip if models missing) |
| Distinct load errors `ModelNotFound`/`LlmLibNotFound` + `kind()` + C `flow_last_error_kind` (CR-1); paste `segments` `[{text}\|{key}]` + `plain_text` (CR-2); `empty_reason` (`no_audio`/`no_speech`/`blank_asr`/`hallucination`/`only_filler`) | `tests/api_additions.rs`, `flow-ffi/tests/c_abi.rs` |
| DictMatcher compiled once per dictionary | `session.rs::dict_matcher_is_built_once_per_dictionary` |
| learn_edit candidates (word alignment) | `learn.rs`, `align.rs` unit tests |
| C ABI + header (C program links the staticlib and runs a session) | `flow-ffi/tests/c_abi.rs`, `c_header.rs` |
| JNI exports match `apps/flow-android/JNI_CONTRACT.md` (`dev.flowclone.core.NativeFlowCore.native*`, CoreJson-shaped payloads) in a real JVM; exported set == contract table | `flow-ffi/tests/jni_smoke.rs` (`--features jni`) |
| `flow-cli transcribe` dev harness (mock / whisper / parakeet) | `flow-cli/tests/cli.rs` |

E2E timing on this box (i7-6700, 8 logical cores, no GPU, load ≈ 8-14 from other jobs, 4 threads, release build), jfk.wav 11.0 s:
* tiny.en: ASR 1.3-1.6 s, model load 0.1 s warm;
* base.en: ASR 3.0-3.2 s;
* tiny (multilingual, auto-detects `en`): 2.5-3.4 s;
* Parakeet int8 (`flow-cli --engine parakeet`): ASR 0.9-1.6 s.

Post-processing takes under 1 ms. With live partials on, commit takes about 1.8 s versus 1.5 s without them, because the in-flight partial decode is aborted.

### Windows shell (`apps/flow-windows`)
Wired to the real core (from branch `win-p1`). Proof = `cd apps/flow-windows && cargo test` (184 tests: 171 lib + 13 integration; the real-model ones skip without `models/` / the plugin).

| Capability | Proof (test) |
|---|---|
| Shell drives real flow-core: PTT → capture → push_audio → commit → paste plan, with the dictionary from prefs applied by the core | `apps/flow-windows/tests/core_integration.rs::mock_asr_audio_to_commit_to_paste_and_history` |
| Real whisper.cpp through the shell: tiny.en on jfk.wav → "ask not what your country…" pasted, `lang=en` in history | `core_integration.rs::real_tiny_model_on_jfk_pastes_the_speech` (skips without `models/`) |
| §9 status handling: EMPTY pastes nothing; press_enter → Enter after paste; missing model → error pill naming the path, never downloads | `core_integration.rs` (silence / press_enter / missing_model), `core_adapter.rs` tests |
| Live partials → overlay preview, cleared when listening ends | `core_integration.rs::live_partials_reach_the_overlay_preview` |
| Local SQLite history (10-clone-spec §2 columns, no audio column), keep/24h/never retention; paste-last (Shift+Alt+Z) from it | `history.rs` tests, `core_integration.rs` (paste-last, retention never) |
| Optional LLM cleanup through the shell: real `flow_llm` plugin + Qwen2.5-0.5B → FORMATTED pasted, raw ASR kept in history; whisper tiny.en + llama in one shell process; missing plugin/GGUF = `LlmStatus` notice while dictation still pastes RAW; two-phase load (ASR ready first, LLM joins without reloading whisper), supersede on toggle | `core_integration.rs::real_llm_plugin_formats_through_the_shell`, `real_whisper_then_real_llm_in_the_shell_process` (skip without plugin/GGUF), `missing_llm_plugin_still_dictates_raw`; `core_adapter.rs` two-phase tests (fake cleaner) |
| Learn-from-edits: after a paste the field is re-read; the user's correction (`whisper flow` → `Wispr Flow`) becomes a pending suggestion (never auto-added) and fills history `edited`; Wispr end reasons, anchor-bounded region, trailing-typing trim | `core_integration.rs::user_correction_becomes_a_suggestion_not_an_entry`, `learning_off_or_untouched_paste_suggests_nothing`, `learn.rs`, `suggestions.rs` tests. UIA reader only clippy-checked |
| History tab backend (search/get/delete/clear/set_edited, undo-AI-edit flag), copy-last (Shift+Alt+X), dictionary import/export of the shared JSON | `history.rs`, `dictionary.rs`, `chord.rs` tests, `core_integration.rs::copy_last_puts_the_transcript_on_the_clipboard` |
| Settings UI tabs (General / AI cleanup / History / Dictionary) render and run against a mocked `__TAURI__` in headless Chromium | `tools/probes/settings_mock/` (manual screenshot check, not a test) |
| Hook watchdog (LowLevelHooksTimeout removal → probe → re-install); UIPI integrity pre-check; clickable hands-free ✕/✓ hit-test; {{BREAK}}/{{TAB}} paste segments | `hotkeys/watchdog.rs`, `paste/mod.rs` (`uipi_blocks`, `segments`), `overlay.rs` tests. Win32 sides are only `cargo clippy`-clean for `x86_64-pc-windows-msvc --no-default-features --features app` (whisper-rs-sys can't cross-build; see README) |

Not yet run on real Windows. The manual checklist is in `apps/flow-windows/README.md`.

### Android shell (`apps/flow-android`)
From branches `android-int` + `android-p1`. `./gradlew testDebugUnitTest assembleDebug` is green (238 JVM tests, 9 gated skips) and the no-network gate passes: the merged manifest has no network permission, which a Gradle gate and a unit test enforce. Debug APK with all three .so (flow_ffi, flow_llm, libc++_shared): 14.2 MB; with flow_ffi only: 9.3 MB (.so 3.9 MB stripped + libc++ 1.3 MB). Install guide: `apps/flow-android/INSTALL.md`.

| Capability | Proof (test) |
|---|---|
| V3 insertion state machine (07 §3), keep-alive, bubble/visibility policy, dictation controller (cap, timeout, debounce), gesture/position/waveform logic | JVM unit tests with virtual time |
| Kotlin↔JNI contract (`JNI_CONTRACT.md`) link-tested against a host C stub | `tools/probes/jni_stub/run.sh` |
| Real flow-ffi under the shipped Kotlin binding (no device): x86_64 host `libflow_ffi.so` (jni,whisper) loaded by `NativeFlowCore` + `CoreJson`: start → push (jfk.wav in 100 ms frames) → commit returns the correct transcript with 0 dropped samples; mock round trip and missing-model path pass. Host timings (i7-6700, load 12-20 from other jobs, 4 threads, 11 s audio): tiny.en ASR 5.5-9.5 s, load 0.2-1.1 s; base.en ASR 12.6-17.6 s; worst push_audio 4-19 ms under that load (not phone numbers) | `apps/flow-android/tools/probes/host_native/run.sh [model]` |
| arm64-v8a `libflow_ffi.so` with whisper cross-built (`tools/build-native.sh --whisper`; NDK toolchain wrapper, ggml armv8.2-a+dotprod+fp16), packaged with `libc++_shared.so`: all 11 contract symbols exported, every LOAD segment 16 KB aligned, NEEDED satisfied, APK `.so` entries stored and 16 KB zip-aligned | `tools/probes/check_native_so.sh app-debug.apk` |
| arm64 `libflow_llm.so` cross-built (`tools/build-native.sh --whisper --llm`; llama-cpp-sys-2 drives the NDK toolchain, OpenMP off, ggml armv8.2-a+dotprod+fp16) and packaged; exports exactly the 6 `flow_llm_*` symbols, NEEDED = libc++_shared/libc/libm/libdl, 16 KB aligned; flow-ffi built with the `llm` loader | `tools/probes/check_native_so.sh app-debug.apk` |
| Shake detector (2 impulses ≤ 1.2 s, 100 ms cooldown), bubble presence (minimise after 5 s, snooze 10 min from Ready only / not in Repair, shake un-snooze / 10 s reveal, lock-screen ignore), snooze-target geometry | `bubble/BubblePresenceTest.kt` (virtual time) |
| Ghost a11y detection (enabled but unbound after 15 s grace, or heartbeat stale > 60 s with the screen on), enabled-services parsing, FlowService watchdog spacing ≥ 5 s, OEM autostart guidance | `health/HealthTest.kt` |
| Live partials polled from the core while recording (only when `partial_interval_ms` is set), cleared on submit/cancel; "can't hear you" after 3 s leading silence (Wispr HeadSilenceTracker threshold); EMPTY → transient "no speech detected" | `session/DictationControllerTest.kt`, `bubble/PreviewLogicTest.kt` |
| LLM settings (default off, LIGHT when on, 3 s budget), EnginePlan with ASR-only fallback when the GGUF/plugin fails, ggml-vs-GGUF sniffing, dictionary edit/validate/filter, history search/raw-vs-formatted | `data/EditorLogicTest.kt` |
| Real LLM cleanup through the shipped Kotlin binding on the host JVM (flow-ffi jni,whisper,llm + host flow-llm plugin, Qwen2.5-0.5B Q4_K_M, settings path = shell's own `Settings.toSessionConfig`): FORMATTED ("so um i think … uh bring the slides" → "So, I think we should meet tomorrow at noon and bring the slides."), llm 2.1 s, load 15.9 s at load ~17; LLM off → RAW with llm_ms 0; bad plugin path → EnginePlan falls back to ASR-only; whisper tiny.en + LLM in one JVM: FORMATTED (asr 9.4 s, llm 3.3 s) | `NativeFlowCoreHostTest` via `tools/probes/host_native/run.sh` (7/7) |

Wired on Android, unverified on a device: snooze target + notification undo, shake listener (screen-on only), 36dp minimised dot, Repair bubble + ghost notification text, a11y 15 s heartbeat + FlowService respawn, `BootReceiver` (BOOT_COMPLETED / MY_PACKAGE_REPLACED / USER_UNLOCKED), USER_PRESENT re-attach, battery-exemption + OEM steps in onboarding, preview window, GGUF import, dictionary editor, history screen, history DB v2 (fallback reason + per-stage timings).

## Outstanding work — priority order
**Active work:** hybrid local/server ASR (D17), spec in `docs/hybrid-architecture.md`.
**Next:** opt-in cloud STT — docs/cloud-stt.md (starts after hybrid-* branches merge)
1. **Core gaps** (the core itself is done; see What works):
   * Shells adopt the core-p1 API per core-architecture "Changelog for shells": paste `segments` instead of acting on `press_enter`, `plain_text` in history, `empty_reason` ("no speech heard"), typed load errors (Windows can drop its `is_file()` pre-check), optional `spoken_cleanup` pref. Both shells build and pass against the merged core (Windows needed only a `..Default` and a test fix); adoption itself is not done.
   * LLM latency on CPU: prefill-ahead is in; generation (≈ 25 tokens at 5-16 tok/s here) now dominates, so 1.5B misses the 1.5 s budget about half the time. Levers left: a LoRA on (raw → formatted) pairs for 0.5B (03 §7.5); `audio_ctx` for whisper with a repetition guard (benchmark: small 1.8× faster); Parakeet (D12) as the faster English engine. Shells must ship `libflow_llm` next to the exe (Windows) / in jniLibs (Android: packaged, host-JVM-proven, not yet loaded on bionic).
   * Spoken cleanup gaps: no list formatting, no comma splices / grammar, no email layout (LLM only); c04-style "actually I mean" restructures keep the correction's wording.
   * Endpointing (auto-stop on trailing silence in hands-free mode) is not done: the VAD runs at commit and on partials only.
   * Engine choice: no shell selects Parakeet yet (D4: English-only user + model present → `asr_engine:"parakeet"`).
2. **Windows shell.** Remaining:
   * a real-Windows build and run of the README manual checklist (needs the user's Windows box);
   * the mouse/Caps Lock shortcuts;
   * the P1 items (LLM cleanup + flow_llm.dll packaging, history tab, learn-from-edits via UIA, dictionary editor) are only cross-checked for MSVC: the README checklist covers them on real Windows;
   * learn-from-edits: UIA TextChanged events instead of 1 s polling; optional local-LLM vetting of candidates (03 §6).
3. **Android shell.** Code is done. Remaining:
   a. JNI + arm64 packaging is done, but the .so has never been loaded on bionic. The first device run must confirm `Native engine: flow-ffi 0.1.0`.
   b. Parakeet on Android: arm64-v8a `libflow_ffi.so` (jni,whisper,parakeet) + sherpa/onnxruntime prebuilts cross-built and 16 KB aligned (`tools/build-android-parakeet.sh`, `tools/probes/check_parakeet_so.sh`); packaging steps in `docs/core-architecture.md` "Android: Parakeet". Not yet in `apps/` and never loaded on a device.
   c. **Device verification (needs the user's phone).** Full checklist: `apps/flow-android/INSTALL.md` §"Device test checklist". In order:
      - grant mic, overlay, a11y and notifications;
      - mock engine: tap and hold dictation into Notes, WhatsApp, Chrome URL bar, a notification reply and a password field (must be hidden);
      - check that the banking app is hidden;
      - insertion fallbacks: commitText vs ACTION_SET_TEXT vs clipboard (logcat tag `FlowA11y` prints the step trail);
      - FGS microphone promotion on Android 14+;
      - the 5-min cap;
      - SAF import of `ggml-base.en-q5_1.bin` (quantised models are untested; tiny.en/base.en f16 are proven on host), then real ASR latency; no SIGILL (dotprod/fp16 build);
      - 16 KB-page device (Android 15): installs without the compat dialog;
      - MIC vs VOICE_RECOGNITION;
      - P1 (07 / 10 §6, done in code on `android-p1`): INSTALL.md sections AI cleanup, Robustness, Live preview, Dictionary & history — shake/snooze, minimise, watchdog/boot/unlock recovery, ghost check, battery + OEM onboarding, live partials + silence hints, LLM toggle + GGUF import, dictionary editor, history screen. Open questions only a device answers: does USER_UNLOCKED reach the manifest receiver; partial-decode CPU cost on a phone (preview default on); LLM latency/RAM with whisper + 0.5B both resident; which OEM deep links resolve.
   d. Not done: `shrinkBubbleInSearch`, snooze-target gravity pull, learn-from-edits. VAD head/tail trim now lives in the core (D16); the shell only reports silence.
4. **P1 cross-platform:** context awareness (both shells); learn-from-edits on Android (Windows is wired).
