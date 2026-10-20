# flow-android: Android shell of the local Wispr Flow clone

This is a Kotlin shell around the shared Rust core (`crates/flow-core` via `crates/flow-ffi`).
It's single-process with no AndroidX and no `INTERNET` permission. minSdk 33 (it needs
`AccessibilityService.InputMethod`), targetSdk 35, arm64-v8a.

Specs: android-research `findings/by-topic/07-android-shell-impl-spec.md` (primary), 05, 06,
10. Core API: windows-research `10-clone-spec.md` §9. The JNI surface is in
[`JNI_CONTRACT.md`](JNI_CONTRACT.md).

## Build

```sh
cd apps/flow-android
./gradlew testDebugUnitTest assembleDebug     # APK: app/build/outputs/apk/debug/app-debug.apk
tools/unit.sh                                  # same tests, compact summary
tools/probes/jni_stub/run.sh                   # link-test NativeFlowCore against a host C stub of the JNI contract
tools/build-native.sh --whisper --llm          # arm64 libflow_ffi.so (+ LLM loader) + libflow_llm.so + libc++_shared.so into app/src/main/jniLibs/
tools/probes/check_native_so.sh [apk]          # contract symbols, 16 KB alignment, NEEDED libs, plugin exports (+ APK zipalign -P 16)
tools/probes/host_native/run.sh [model.bin]    # real host flow-ffi under the real NativeFlowCore: whisper on jfk.wav, + LLM cleanup if a GGUF is in models/
```

Install on a phone (sideload, permissions, model import, device checklist): [`INSTALL.md`](INSTALL.md).

- The Gradle daemon is pinned to JDK 21 (`gradle/gradle-daemon-jvm.properties`). This host's
  default `java` is a JDK-25 JRE without `javac`. Gradle picks up the JDK 21 already
  provisioned at `~/.gradle/jdks/`; on other machines the foojay resolver provisions it.
- `local.properties` (git-ignored) holds `sdk.dir=~/Android/Sdk`.
- Without `app/src/main/jniLibs/arm64-v8a/libflow_ffi.so` the APK still builds and runs. The
  main screen then says the native engine is unavailable, and dictation shows an
  "engine unavailable" bubble error instead of crashing. **Settings → Mock engine** swaps in
  `FakeFlowCore`, which inserts a fixed sentence, so the shell can be tested on a device
  without the .so or a model.

### Toolchain installed for this (2026-10-20)
- `sdkmanager "platforms;android-35" "build-tools;35.0.0" "build-tools;36.0.0" "platform-tools" "ndk;27.2.12479018"`
- Gradle 9.3.1 wrapper (generated from the cached distribution); AGP 9.1.1 (built-in Kotlin).
- `cargo-ndk` 4.1.2 was already installed; the `aarch64-linux-android` rustup target was
  already present.

## Privacy gate
- The manifest strips `INTERNET`, `ACCESS_NETWORK_STATE` and `ACCESS_WIFI_STATE` with
  `tools:node="remove"`.
- `verify<Variant>NoNetworkPermissions` reads the **merged** manifest and fails the
  assemble, test or check run if any network permission survives. It was verified by
  temporarily re-adding INTERNET: the build failed.
- `ManifestPrivacyTest` checks the same thing from the unit suite.
- Backups and device transfer are disabled (`allowBackup=false` plus data-extraction rules).

## Layout (`app/src/main/java/dev/flowclone/`)
| Package | What | Tested on JVM |
|---|---|---|
| `core/` | `FlowCore` (§9 mirror), `CoreJson` (wire format), `NativeFlowCore` (JNI), `FakeFlowCore` | yes, plus the stub link test |
| `insert/` | `V3Inserter` (07 §3), `InsertionCoordinator` (mutex, 3500 ms budget, sensitive clipboard), `ImeKeepAlive` (07 §3.5), `TextMatching` (06 §4), `Ports` (injectable editor abstraction) | yes |
| `policy/` | `EditorInputType`, `BubbleVisibilityPolicy` (sensitive + banking + user denylist + clone deviations), `AppCategorizer`, generated `PolicyLists` | yes |
| `session/` | `DictationController` (state machine, debounce, cap 239/299 s, 20 s timeout), `LevelMeter`, `AudioBatcher`, `SessionCap` | yes |
| `bubble/` | `BubbleGesture` (tap / hold / drag), `BubblePositioner` (anchors + persistence), `Waveform`, `BubblePresence` (minimise 5 s, snooze 10 min, shake reveal), `ShakeDetector`, `SnoozeTarget`, `PreviewContent`/`PreviewPlacement` (live preview line); `BubbleView`/`BubbleOverlay` (Android: bubble + preview + snooze-target windows) | logic yes |
| `data/` | `Retention`, `DictionaryMerge`, `Settings`, `FieldContextBuilder`, `EnginePlan` (ASR + optional LLM, ASR-only fallback), `ModelFiles` (ggml/GGUF sniffing, LLM recommendations), `DictionaryEdit`, `HistoryView`; `Stores` (SharedPreferences, JSON dictionary file, SQLite history v2) | logic yes |
| `health/` | `A11ySetting` (enabled-services parse), `AccessibilityHealth` (ghost binding), `ServiceWatchdog` (≥ 5 s respawn), `OemGuidance` (autostart steps + deep links) | yes |
| `service/` | `FlowAccessibilityService` (+ 15 s heartbeat / FlowService watchdog), `ShellInputMethod`/node adapters, `FlowService` (FGS, shake sensor, health tick, unlock re-check) | device only |
| `receiver/` | `BootReceiver` (BOOT_COMPLETED, MY_PACKAGE_REPLACED, USER_UNLOCKED → start FGS or "re-enable" notification) | device only |
| `audio/` | `MicRecorder` (MIC 16 k mono PCM16, 100 ms frames → `push_audio`) | device only |
| `ui/` | Main/onboarding (mic → overlay → a11y + ghost fix → battery → notifications → OEM), model import (SAF; speech ggml and LLM GGUF), settings, dictionary editor (search, tabs, edit, shared JSON import/export), history (search, copy, delete, raw vs formatted) | device only |

`policy/PolicyLists.kt` is generated: `python3 -I tools/gen_policy_lists.py` from
`policy-data/*.json`, which are verbatim copies of android-research `findings/data/`.

## Deviations from Wispr (deliberate)
- V3 inserter only, with every A/B feature on. No Legacy path.
- Single process, so no AIDL bridge and no JobScheduler recovery job. The watchdog lives in
  the a11y service's 15 s health tick (respawns FlowService ≥ 5 s apart); `BootReceiver`
  covers boot/update/unlock; FlowService re-checks on `USER_PRESENT`.
- Shake, snooze target and minimise follow 07 §5.3/§5.5 (no gravity pull on the target).
  Minimise and shake are settings (default on). The accelerometer is only registered while
  the screen is on. Sounds use `ToneGenerator` beeps instead of bundled samples.
- The bubble position is persisted; a snooze puts it back on its saved anchor.
- Live preview: a click-through window above the bubble shows the core's partial
  (`partial_interval_ms` 1500, polled every 250 ms) plus "can't hear you" after 3 s of
  leading silence and "no speech detected" on EMPTY. Wispr has neither.
- LLM cleanup is opt-in (default off), with a 3 s budget on Android (desktop 1.5 s). If the
  plugin or GGUF fails to load, dictation keeps working ASR-only and the main screen says why.
- Snippet expansions may be up to 2000 characters (Wispr: 60).
- The cap is configurable (1–20 min, default 5) and warns 61 s / stops 1 s before the cap.
- A blank transcript is treated as empty. Browser+GO never un-hides a password field.
  `isAccessibilityDataSensitive` hides the bubble. A user app denylist sits next to the
  137 banking packages.
- History stores text only, never audio.

## Needs a device
See the checklist in [`INSTALL.md`](INSTALL.md) and `PROJECT_CONTEXT.md` §Android.
