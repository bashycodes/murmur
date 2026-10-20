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
| `flow-cli clean --llm m.gguf TEXT…` / `transcribe --llm` | manual (`docs/core-architecture.md` Build/test); loader path covered by `llm_e2e` |
| learn_edit candidates (word alignment) | `learn.rs`, `align.rs` unit tests |
| C ABI + header (C program links the staticlib and runs a session) | `flow-ffi/tests/c_abi.rs`, `c_header.rs` |
| JNI exports in a real JVM (`com.flowclone.core.NativeFlowCore`) | `flow-ffi/tests/jni_smoke.rs` (`--features jni`) |
| `flow-cli transcribe` dev harness (mock / whisper) | `flow-cli/tests/cli.rs` |

E2E timing on this box (i7-6700, 8 logical cores, no GPU, load ≈ 8-14 from other jobs, 4 threads, release build), jfk.wav 11.0 s:
* tiny.en: ASR 1.3-1.6 s, model load 0.1 s warm;
* base.en: ASR 3.0-3.2 s;
* tiny (multilingual, auto-detects `en`): 2.5-3.4 s.

Post-processing takes under 1 ms. With live partials on, commit takes about 1.8 s versus 1.5 s without them, because the in-flight partial decode is aborted.

- `apps/flow-windows` (Windows shell): pure logic (chord matcher, dictation machine w/ Wispr timings, resampler/level meter, paste plan, context/app kinds, prefs, controller loop) — 111 tests green on Linux; Win32 + Tauri code `cargo clippy`-clean for `x86_64-pc-windows-msvc` (incl. `--features app` via check shim). Not yet run on real Windows; uses `StubCore` until flow-core lands (see `apps/flow-windows/README.md`).

## Outstanding work — priority order
1. `flow-core` crate — DONE on branch `core` (see What works). Remaining core gaps:
   * LLM latency on CPU: 1.5B is 8/8 but ~1.1-5 s per call on this shared box (0.5B: 4/8, 0.75-2.7 s). Next levers: prefill the context + partial transcript into the KV cache while the user speaks; a LoRA on (raw → formatted) pairs for 0.5B (03 §7.5); pick the shipped model from `docs/model-benchmarks.md`. Shells must ship `libflow_llm` next to the exe (Windows) / in jniLibs (Android, untested);
   * Android link via cargo-ndk (needs the NDK; type-checks for aarch64-linux-android);
   * the JNI class name must match `apps/flow-android/JNI_CONTRACT.md`;
   * VAD and endpointing;
   * a per-session cache for the DictMatcher (it is rebuilt each commit).
2. Windows shell: wire flow-core in `apps/flow-windows/src/core_adapter.rs`; run manual checklist in `apps/flow-windows/README.md` on real Windows; gaps there (clickable hands-free buttons, UIPI pre-check, hook re-install).
3. Android shell: a11y service, overlay bubble, FGS recorder, JNI to core.
4. P1: LLM cleanup, context, learn-from-edits.
