# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `../windows-research/findings/by-topic/10-clone-spec.md` (§9 = core API), `../android-research` branch `worktree-android-findings` `findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md`.

## What works
(nothing yet)

## Outstanding work — priority order
1. `flow-core` crate: §9 API, deterministic post-processing port, whisper backend, tests.
2. Windows shell (Tauri): hooks, capture, paste, overlay, tray.
3. Android shell: a11y service, overlay bubble, FGS recorder, JNI to core.
4. P1: LLM cleanup, context, learn-from-edits.
