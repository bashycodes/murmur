# PROJECT_CONTEXT — flow-clone (local-only Wispr Flow clone)

Research inputs: `../windows-research/findings/by-topic/10-clone-spec.md` (§9 = core API), `../android-research` branch `worktree-android-findings` `findings/by-topic/10-clone-spec-android.md`. Decisions: `docs/DECISIONS.md`.

## What works
- Android shell builds: `cd apps/flow-android && ./gradlew testDebugUnitTest assembleDebug` is green (166 JVM tests; APK without the .so). The merged manifest has no network permission, which a Gradle gate and a unit test enforce.
- Android V3 insertion state machine (07 §3), keep-alive, bubble/visibility policy, dictation controller (cap, timeout, debounce), gesture/position/waveform logic: unit-tested on the JVM with virtual time.
- The Kotlin↔JNI contract (`apps/flow-android/JNI_CONTRACT.md`) is link-tested against a host C stub (`tools/probes/jni_stub/run.sh`).

## Outstanding work — priority order
1. `flow-core` crate: §9 API, deterministic post-processing port, whisper backend, tests.
2. Windows shell (Tauri): hooks, capture, paste, overlay, tray.
3. Android shell. Code is done (`apps/flow-android/`). Remaining:
   a. `crates/flow-ffi` must implement the `jni` feature per `JNI_CONTRACT.md`, then `tools/build-native.sh --whisper` and repackage.
   b. **Device verification (needs the user's phone).** In order:
      - grant mic, overlay, a11y and notifications;
      - mock engine: tap and hold dictation into Notes, WhatsApp, Chrome URL bar, a notification reply and a password field (must be hidden);
      - check that the banking app is hidden;
      - insertion fallbacks: commitText vs ACTION_SET_TEXT vs clipboard (logcat tag `FlowA11y` prints the step trail);
      - FGS microphone promotion on Android 14+;
      - the 5-min cap;
      - SAF import of `ggml-base-q5_1.bin`, then real ASR latency;
      - MIC vs VOICE_RECOGNITION.
   c. P1 (07 / 10 §6): shake/snooze, bubble minimise, watchdog/boot recovery, battery-optimisation onboarding, VAD trim, partials.
4. P1: LLM cleanup, context, learn-from-edits.
