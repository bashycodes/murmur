# flow-android: Android shell of Murmur (a local Wispr Flow clone)

This is a Kotlin shell around the shared Rust core (`crates/flow-core` via `crates/flow-ffi`).
It's single-process with no AndroidX. minSdk 33 (it needs `AccessibilityService.InputMethod`),
targetSdk 35, arm64-v8a. Three product flavours (D17): `local` (no `INTERNET`, on-device
only), `hybrid` (own flow-server + on-device fallback), `remote` (server only, no ASR libs).
Every session goes through the native `flow_router::Router` (`nativeOpenRouter`).

Core API: [`docs/core-architecture.md`](../../docs/core-architecture.md). The JNI surface is in
[`JNI_CONTRACT.md`](JNI_CONTRACT.md).

## Build

```sh
cd apps/flow-android
./gradlew testDebugUnitTest assembleDebug     # all flavours; APKs: app/build/outputs/apk/<flavour>/debug/
tools/unit.sh                                  # same tests, compact summary
tools/probes/jni_stub/run.sh                   # link-test NativeFlowCore against a host C stub of the JNI contract
tools/build-native.sh --flavor local|hybrid|remote   # arm64 native libs into app/src/<flavour>/jniLibs/ (see INSTALL.md)
tools/probes/check_native_so.sh --flavor F [apk]  # contract symbols, 16 KB alignment, NEEDED libs, plugin exports, flavour rules (+ APK zipalign -P 16)
tools/probes/check_native_so_test.sh [runs]    # that gate on a stub toolchain: no SIGPIPE flake, failures name their check (CI repo-checks)
tools/dist.sh [--debug-only]                   # clean debug (+ release) APKs of all flavours into dist/ + SHA256SUMS, native gates
tools/probes/host_native/run.sh [model.bin]    # real host flow-ffi under the real NativeFlowCore: whisper on jfk.wav, + LLM cleanup if a GGUF is in models/
tools/probes/host_cloud/run.sh                 # D18: real host flow-ffi (jni,cloud) under the shipped Kotlin cloud path vs a JVM mock provider
```

Install on a phone (sideload, permissions, model import, device checklist): [`INSTALL.md`](INSTALL.md).

- The Gradle daemon is pinned to JDK 21 (`gradle/gradle-daemon-jvm.properties`). This host's
  default `java` is a JDK-25 JRE without `javac`. Gradle picks up the JDK 21 already
  provisioned at `~/.gradle/jdks/`; on other machines the foojay resolver provisions it.
- `local.properties` (git-ignored) holds `sdk.dir=~/Android/Sdk`.
- Without `app/src/<flavour>/jniLibs/arm64-v8a/libflow_ffi.so` the APK still builds and runs. The
  Permissions page then says the native engine is unavailable, and dictation shows an
  "engine unavailable" bubble error instead of crashing. **Settings → Mock engine** swaps in
  `FakeFlowCore`, which inserts a fixed sentence, so the shell can be tested on a device
  without the .so or a model.

### Toolchain installed for this (2026-10-20)
- `sdkmanager "platforms;android-35" "build-tools;35.0.0" "build-tools;36.0.0" "platform-tools" "ndk;27.2.12479018"`
- Gradle 9.3.1 wrapper (generated from the cached distribution); AGP 9.1.1 (built-in Kotlin).
- `cargo-ndk` 4.1.2 was already installed; the `aarch64-linux-android` rustup target was
  already present.

## Privacy gates (per flavour)
- D18 cloud STT (`docs/cloud-stt.md`): only the `hybrid` / `remote` libraries link the provider
  client (flow-ffi feature `cloud`; `BuildConfig.CAP_CLOUD`); `verifyLocal<BT>NoNetworkCode` and
  `check_native_so.sh` reject it in `local`. Off by default; per-provider consent naming the host;
  API keys only in the Keystore (`SecretVault("flow_cloud_<id>")`) and native memory.
- `src/local/AndroidManifest.xml` strips every network permission with `tools:node="remove"`;
  `src/hybrid` / `src/remote` add `INTERNET` and strip the rest. The main manifest decides nothing.
- `verifyLocal<BT>NoNetworkPermissions` / `verify{Hybrid,Remote}<BT>OnlyInternetPermission`
  read the **merged** manifest; `verifyRemote<BT>NoAsrLibs` and `verifyLocal<BT>NoNetworkCode`
  inspect the packaged APK. They fail assemble / test / check.
- `ManifestPrivacyTest` checks each flavour's merged manifest from the unit suite.
- Backups and device transfer are disabled (`allowBackup=false` plus data-extraction rules).

## Layout (`app/src/main/java/dev/flowclone/`)
| Package | What | Tested on JVM |
|---|---|---|
| `core/` | `FlowCore` (§9 mirror), `CoreJson` (wire format), `RouterJson` + `DevText` (router JSON, D17), `NativeFlowCore` (JNI, core + router handles), `FakeFlowCore` | yes, plus the stub link test and `RouterJsonTest` on `crates/flow-router/fixtures` |
| `insert/` | `V3Inserter` (07 §3), `InsertionCoordinator` (mutex, 3500 ms budget, sensitive clipboard), `ImeKeepAlive` (07 §3.5), `TextMatching` (06 §4), `Ports` (injectable editor abstraction) | yes |
| `policy/` | `EditorInputType`, `BubbleVisibilityPolicy` (sensitive + restricted (banking) apps + user denylist + clone deviations), `AppCategorizer`, `OwnAppGate` (our own screens: no bubble except the Scratchpad), `PolicyLists` (app groups and naming rules) | yes |
| `session/` | `DictationController` (state machine, debounce, cap 239/299 s, 20 s timeout), `LevelMeter`, `AudioBatcher`, `SessionCap` | yes |
| `bubble/` | `BubbleGesture` (tap / hold / drag), `BubblePositioner` (anchors + persistence), `Waveform`, `BubblePresence` (minimise 5 s, snooze 10 min, shake reveal), `ShakeDetector`, `SnoozeTarget`, `PreviewContent`/`PreviewPlacement` (live preview line); `BubbleView`/`BubbleOverlay` (Android: bubble + preview + snooze-target windows) | logic yes |
| `data/` | `Retention`, `DictionaryMerge`, `Settings`, `FieldContextBuilder`, `EnginePlan` (ASR + optional LLM, ASR-only fallback), `ModelFiles` (ggml/GGUF sniffing, LLM recommendations), `DictionaryEdit`, `HistoryView`; `Stores` (SharedPreferences, JSON dictionary file, SQLite history v2); D17: `DevSettings` (+ `HostRules`, `DevSettingsForm`, `DevUnlock`), `DevStore` + `SecretVault` (Keystore AES-GCM: server token, cloud keys), `EngineRoute` (local engine → router), `ParakeetFiles`; D18: `CloudSettings` (`CloudProvider`, presets, consent, `CloudRules`, `CloudText`, `CloudUsage`), `CloudKeyStore` | logic yes |
| `notes/` | Scratchpad: `DocumentStore` (load / save by id; `FileDocumentStore` = `files/notes/<id>.txt`, atomic replace), `Autosaver` (debounced, flush on pause), `TextStats` (word count) | yes |
| `learn/` | Learn from my edits: `EditWatch` (anchor region, end reasons, stabilise, `boundEdit`), `Suggestions` (pending / dismissed, add / accept / dismiss, JSON), `EditLearning` | yes |
| `health/` | `A11ySetting` (enabled-services parse), `AccessibilityHealth` (ghost binding), `ServiceWatchdog` (≥ 5 s respawn), `OemGuidance` (autostart steps + deep links) | yes |
| `service/` | `FlowAccessibilityService` (+ 15 s heartbeat / FlowService watchdog), `ShellInputMethod`/node adapters, `FlowService` (FGS, shake sensor, health tick, unlock re-check) | device only |
| `receiver/` | `BootReceiver` (BOOT_COMPLETED, MY_PACKAGE_REPLACED, USER_UNLOCKED → start FGS or "re-enable" notification) | device only |
| `audio/` | `MicRecorder` (MIC 16 k mono PCM16, 100 ms frames → `push_audio`) | device only |
| `ui/` | Main/onboarding (mic → overlay → a11y + ghost fix → battery → notifications → OEM), model import (SAF; speech ggml, Parakeet folder, LLM GGUF), settings, Developer settings (D17 §5; D18 cloud section + `CloudProviderActivity`), dictionary editor (search, tabs, edit, shared JSON import/export), history (search, copy, delete, raw vs formatted), Scratchpad (`NoteEditorActivity`: one document to dictate into, Copy all / Share / Clear) | device only |

`policy/PolicyLists.kt` reads its app groups from `crates/flow-core/data/app-groups.json`
(packed as a Java resource), the file flow-core and the Windows app use too (D80; the JSON and
the PolicyLists header document the method and sources).

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
  plugin or GGUF fails to load, dictation keeps working ASR-only and the Permissions page says why.
- Snippet expansions may be up to 2000 characters (Wispr: 60).
- The cap is configurable (1–20 min, default 5) and warns 61 s / stops 1 s before the cap.
- A blank transcript is treated as empty. Browser+GO never un-hides a password field.
  `isAccessibilityDataSensitive` hides the bubble. A user app denylist sits next to the
  137 banking packages.
- History stores text only, never audio.
- Command Mode is a second bubble button (✎, shown while the focused field has a selection;
  *Settings → Bubble → Command mode*), not a gesture on the mic: tap / hold / drag keep their
  meaning. The instruction is transcribed raw and rewritten by the shared Rust Command Mode
  (`nativeCommand` → `flow_core::command`, same prompt, sanitiser and privacy gate as Windows).
  Only the captured selection is replaced (`command/SelectionSplice`: `ACTION_SET_TEXT` with the
  text around it kept, read back; else `commitText` while the same text is still selected).

## Look and fonts
Screens follow the Murmur design (`docs/design/app-design`, Android artboards): tokens in
`ui/Tokens.kt` (light from `.lw-android`, dark from `.lw-dark`), helpers in `ui/DevUi.kt`.
Text uses Figtree and stat numbers EB Garamond, both variable-weight fonts bundled in
`app/src/main/assets/fonts/` under the SIL Open Font License 1.1; the licence texts ship in the
APK at `assets/licenses/figtree-OFL.txt` and `assets/licenses/ebgaramond-OFL.txt`
(EB Garamond is a Latin subset). If a font fails to load, the system sans / serif is used.

## Needs a device
See the checklist in [`INSTALL.md`](INSTALL.md) and `PROJECT_CONTEXT.md` §Android.
