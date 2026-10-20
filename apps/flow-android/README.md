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
tools/build-native.sh [--whisper]              # build libflow_ffi.so (needs crates/flow-ffi with the `jni` feature)
```

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
| `bubble/` | `BubbleGesture` (tap / hold / drag), `BubblePositioner` (anchors + persistence), `Waveform`; `BubbleView`/`BubbleOverlay` (Android) | logic yes |
| `data/` | `Retention`, `DictionaryMerge`, `Settings`, `FieldContextBuilder`; `Stores` (SharedPreferences, JSON dictionary file, SQLite history) | logic yes |
| `service/` | `FlowAccessibilityService`, `ShellInputMethod`/node adapters, `FlowService` (FGS) | device only |
| `audio/` | `MicRecorder` (MIC 16 k mono PCM16, 100 ms frames → `push_audio`) | device only |
| `ui/` | Main/onboarding, model import (SAF), settings, dictionary (shared JSON import/export), history | device only |

`policy/PolicyLists.kt` is generated: `python3 -I tools/gen_policy_lists.py` from
`policy-data/*.json`, which are verbatim copies of android-research `findings/data/`.

## Deviations from Wispr (deliberate)
- V3 inserter only, with every A/B feature on. No Legacy path.
- Single process, so no AIDL bridge, watchdog, job or boot receivers in the MVP.
- No accelerometer shake or snooze target. Sounds use `ToneGenerator` beeps instead of
  bundled samples.
- The bubble position is persisted. No minimise/shrink yet.
- The cap is configurable (1–20 min, default 5) and warns 61 s / stops 1 s before the cap.
- A blank transcript is treated as empty. Browser+GO never un-hides a password field.
  `isAccessibilityDataSensitive` hides the bubble. A user app denylist sits next to the
  137 banking packages.
- History stores text only, never audio.

## Needs a device
See `PROJECT_CONTEXT.md` (repo root) §Android for the on-device checklist.
