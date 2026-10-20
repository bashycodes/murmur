# Installing Flow Clone on an Android phone

Flow Clone has **no INTERNET permission**. It can't download anything, including the
speech model. You fetch the model on a PC and copy it to the phone yourself.

Requirements: Android 13 or newer (minSdk 33), an arm64 phone, and about 200 MB free
(APK about 9 MB, model 30–150 MB). Android 15 phones with 16 KB pages are supported: every
`.so` is 16 KB aligned (checked by `tools/probes/check_native_so.sh`).

## 1. Build the APK (on the PC)

```sh
cd apps/flow-android
tools/build-native.sh --whisper          # libflow_ffi.so + libc++_shared.so -> app/src/main/jniLibs/arm64-v8a/
./gradlew testDebugUnitTest assembleDebug
tools/probes/check_native_so.sh app/build/outputs/apk/debug/app-debug.apk
```

The output is `app/build/outputs/apk/debug/app-debug.apk`, debug-signed. If you skip
`build-native.sh`, the APK still installs, but the main screen says "Native engine
unavailable". Only the mock engine works in that build.

The whisper code is compiled for `armv8.2-a+dotprod+fp16`, which covers any big core from
2018 on (Cortex-A75 and later, all recent Snapdragon, Tensor, Exynos and Dimensity chips).
If the app crashes with `SIGILL` the first time it transcribes, your phone has only older
cores, such as a Cortex-A53-only budget SoC. Rebuild for it:
`FLOW_ARM_ARCH=armv8-a tools/build-native.sh --whisper`.

## 2. Get a speech model (on the PC)

Models are whisper.cpp **ggml** files (`ggml-*.bin`). They are not `.gguf`, `.pt` or
`.onnx` files. Download one in a browser from the whisper.cpp model repository on Hugging
Face, `https://huggingface.co/ggerganov/whisper.cpp/tree/main`:

| File | Size | Use |
|---|---|---|
| `ggml-base.en-q5_1.bin` | ~57 MB | **Recommended.** English only, good accuracy |
| `ggml-tiny.en-q5_1.bin` | ~31 MB | English only, fastest. Try this first on slow phones |
| `ggml-base-q5_1.bin` | ~57 MB | Multilingual. Set the language in Settings, or "auto" |
| `ggml-tiny.en.bin` / `ggml-base.en.bin` | 75 / 142 MB | Unquantised. The host tests use these, so they are the proven builds |

The `.en` models are English only. The multilingual ones (no `.en`) also auto-detect the
language. A direct link looks like
`https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin`.

Copy the file to the phone's `Download/` folder, over USB file transfer or any way you like.

## 3. Install the APK

Do one of these:

- **USB with platform-tools.** Turn on Developer options → USB debugging, then run the
  platform-tools `install` command on `app-debug.apk` (add `-r` to update in place).
- **No PC tools.** Copy `app-debug.apk` to the phone and open it in Files. Allow
  "Install unknown apps" for Files when Android asks.

Updating keeps your data. Uninstalling deletes the imported model, the dictionary and the
history.

## 4. Grant permissions (in order)

Open **Flow Clone**. The main screen lists each permission and its button:

1. **Microphone**: tap *Grant microphone*, then *While using the app*.
2. **Display over other apps**: tap *Allow display over other apps* and switch it on. The
   bubble needs it.
3. **Accessibility service**: tap *Open Accessibility settings* → *Flow Clone* → on.
   - If you installed from Files or a browser (step 3, second option), Android 13+ greys
     the switch out as a **restricted setting**. Fix it in *Settings → Apps → Flow Clone →
     ⋮ (top right) → Allow restricted settings*, then try again. USB installs aren't
     restricted.
4. **Notifications** (optional) show the status and the session-limit warning.

## 5. Import the model (SAF)

Main screen → **Speech model…** → **Import model file…**. The system file picker opens.
Choose `Download/ggml-base.en-q5_1.bin`, or the model you copied.

The app copies the file into its private storage (`files/models/`) and loads it. The
original in `Download/` can be deleted afterwards. The main screen should then show
`Native engine: flow-ffi 0.1.0` and `Loaded: ggml-base.en-q5_1.bin`. To switch models,
use *Use* on another imported file. *Delete* frees the space.

## 6. Use it

Main screen → **Start / show bubble**. Focus any text field and the bubble appears by the
keyboard:

- **tap** starts hands-free dictation; tap again to stop and insert;
- **hold** is push-to-talk; release to insert;
- drag the bubble to move it.

## Device test checklist

None of this can be checked on the build machine. Record the results in
`PROJECT_CONTEXT.md`.

Logcat tags: `FlowA11y` (insertion step trail), `FlowService`, `FlowMic`, `FlowEngine`.

**Native engine / packaging**
- [ ] The main screen shows `Native engine: flow-ffi 0.1.0`, not "unavailable". This is
      the first time `System.loadLibrary` runs on bionic: `libc++_shared.so` is resolved,
      and there is no `dlopen` error under `FlowEngine`.
- [ ] Android 15 / 16 KB-page device (e.g. Pixel 8+ with the 16 KB developer option): the
      app installs and no "page size compatibility" dialog appears.
- [ ] Model import: progress, then *Active: …*. Note the model load time ("Loading
      model…" → "Loaded").
- [ ] Real ASR latency. Dictate about 10 s, then measure the time from stop to text
      appearing. Note the model, the phone and the SoC. On the host (x86, loaded) tiny.en
      takes about 5.5 s and base.en about 12–17 s for 11 s of audio; a modern phone should
      beat that.
- [ ] No `SIGILL`, i.e. the CPU supports dotprod/fp16 (see §1).
- [ ] A 60 s dictation transcribes completely, with no words missing in the middle, and
      the bubble waveform stays smooth. The shell doesn't surface the core's
      `dropped_samples` yet.

**Shell** (mock engine: *Settings → Mock engine*, then the real engine)
- [ ] Tap and hold dictation into Notes, WhatsApp, the Chrome URL bar and a notification
      reply.
- [ ] In a password field the bubble is hidden and nothing is inserted.
- [ ] The bubble is hidden in a banking app.
- [ ] Insertion fallbacks: commitText vs ACTION_SET_TEXT vs clipboard. `FlowA11y` prints
      which step was taken.
- [ ] The FGS microphone promotion works on Android 14+, and dictation keeps going with the
      screen locked or the app in the background.
- [ ] Session cap: warning about 61 s before the cap (default 5 min), then auto-stop and
      insert.
- [ ] Check MIC-source audio quality. `MicRecorder` uses MIC; VOICE_RECOGNITION is untried
      and needs a code change.
- [ ] Airplane mode on and off makes no difference. The app has no network permission, and
      nothing in Settings mentions accounts or sync.
