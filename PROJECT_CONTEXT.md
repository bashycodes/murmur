# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `../windows-research/findings/by-topic/10-clone-spec.md` (§9 = core API), `../android-research` branch `worktree-android-findings` `findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md`.

## What works
- `apps/flow-windows` (Windows shell): pure logic (chord matcher, dictation machine w/ Wispr timings, resampler/level meter, paste plan, context/app kinds, prefs, controller loop) — 111 tests green on Linux; Win32 + Tauri code `cargo clippy`-clean for `x86_64-pc-windows-msvc` (incl. `--features app` via check shim). Not yet run on real Windows; uses `StubCore` until flow-core lands (see `apps/flow-windows/README.md`).

## Outstanding work — priority order
1. `flow-core` crate: §9 API, deterministic post-processing port, whisper backend, tests.
2. Windows shell: run the manual checklist in `apps/flow-windows/README.md` on a real Windows box; wire flow-core in `core_adapter.rs`; gaps listed there (clickable hands-free buttons, UIPI pre-check, hook re-install).
3. Android shell: a11y service, overlay bubble, FGS recorder, JNI to core.
4. P1: LLM cleanup, context, learn-from-edits.
