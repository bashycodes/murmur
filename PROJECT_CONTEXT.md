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
| JNI exports in a real JVM (`com.flowclone.core.NativeFlowCore`) | `flow-ffi/tests/jni_smoke.rs` (`--features jni`) |
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

## Outstanding work — priority order
1. `flow-core` crate — DONE on branch `core` (see What works). Remaining core gaps:
   * llama.cpp `Cleaner` backend (feature `llm` is an empty stub; `llm_path` is ignored);
   * Android link via cargo-ndk (needs the NDK; type-checks for aarch64-linux-android);
   * the JNI class name must match `apps/flow-android/JNI_CONTRACT.md`;
   * VAD and endpointing;
   * a per-session cache for the DictMatcher (it is rebuilt each commit).
2. Windows shell: flow-core wired (branch `integration`). Remaining:
   * a real-Windows build and run of the README manual checklist (needs the user's Windows box);
   * a history UI and "copy last transcript";
   * learn-from-edits (`edited` column);
   * the mouse/Caps Lock shortcuts.
3. Android shell: a11y service, overlay bubble, FGS recorder, JNI to core.
4. P1: LLM cleanup, context, learn-from-edits.
