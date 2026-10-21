# Installing Flow Clone on an Android phone

Flow Clone comes in three **flavours** (D17, `docs/hybrid-architecture.md`). They share one
application id (`dev.flowclone`), so installing another flavour over the current one is an
in-place update that keeps settings, dictionary and history.

| Flavour | APK | Dictates with | Network |
|---|---|---|---|
| `local` | `app-local-debug.apk` | the speech model on the phone | **none**: no INTERNET permission, no network code in the native library |
| `hybrid` | `app-hybrid-debug.apk` | your own `flow-server` (LAN / Tailscale), falling back to the phone's model | INTERNET only, used only for the server you enter |
| `remote` | `app-remote-debug.apk` | your own `flow-server` only; no models, no ASR libraries (~10x smaller) | INTERNET only |

The `local` flavour can't download anything, including the speech model: you fetch the model
on a PC and copy it to the phone yourself. The `hybrid`/`remote` flavours only talk to the
server address you type in Developer settings; voice never reaches a third party.

Requirements: Android 13 or newer (minSdk 33), an arm64 phone, and about 200 MB free
(APK about 14 MB, model 30–150 MB). Optional AI cleanup needs another 0.5–1.1 GB of storage
and roughly as much free RAM. Android 15 phones with 16 KB pages are supported: every
`.so` is 16 KB aligned (checked by `tools/probes/check_native_so.sh`).

## 1. Build the APK (on the PC)

```sh
cd apps/flow-android
export SHERPA_ANDROID_LIBS=<sherpa-onnx-v1.13.8-android>/jniLibs/arm64-v8a   # Parakeet runtime (local/hybrid)
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"  # this box's libclang
tools/build-native.sh --flavor local     # -> app/src/local/jniLibs/arm64-v8a/
tools/build-native.sh --flavor hybrid    # -> app/src/hybrid/jniLibs/arm64-v8a/
tools/build-native.sh --flavor remote    # -> app/src/remote/jniLibs/arm64-v8a/ (libflow_ffi.so only)
./gradlew testDebugUnitTest assembleDebug          # all three flavours + their gates
tools/probes/check_native_so.sh --flavor local app/build/outputs/apk/local/debug/app-local-debug.apk
```

`--no-parakeet` / `--no-llm` build smaller local/hybrid libraries. The outputs are
`app/build/outputs/apk/<flavour>/debug/app-<flavour>-debug.apk`, debug-signed. The build
fails if a flavour breaks its promise: `verifyLocalDebugNoNetworkPermissions` (+
`…NoNetworkCode`), `verifyHybridDebugOnlyInternetPermission`,
`verifyRemoteDebugOnlyInternetPermission` and `verifyRemoteDebugNoAsrLibs` (no
whisper/sherpa/onnxruntime/flow_llm/libc++ in the APK, no `whisper_`/`llama_` exports).

If you skip `build-native.sh`, the APK still installs, but the main screen says "Native engine
unavailable". Only the mock engine works in that build. Without the LLM plugin
(`--no-llm`) dictation works, and turning on AI cleanup shows "AI cleanup failed to load …
plugin missing" on the main screen. If the main screen says **"Native library flavour
mismatch"**, the APK was packaged with another flavour's `libflow_ffi.so`: rebuild with the
matching `--flavor`.

The whisper code is compiled for `armv8.2-a+dotprod+fp16`, which covers any big core from
2018 on (Cortex-A75 and later, all recent Snapdragon, Tensor, Exynos and Dimensity chips).
If the app crashes with `SIGILL` the first time it transcribes, your phone has only older
cores, such as a Cortex-A53-only budget SoC. Rebuild for it:
`FLOW_ARM_ARCH=armv8-a tools/build-native.sh --flavor local`.

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
  platform-tools `install` command on `app-<flavour>-debug.apk` (add `-r` to update in place).
- **No PC tools.** Copy `app-<flavour>-debug.apk` to the phone and open it in Files. Allow
  "Install unknown apps" for Files when Android asks.

Switching flavour: install the other APK over the current one (same signature). Settings,
dictionary, history and the developer settings survive; toggles the new flavour can't honour
read as off (e.g. *Allow server* in `local`).

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
4. **Battery optimisation**: tap *Allow running in the background* → *Allow*. Without it
   Android may stop the service and the bubble disappears.
5. **Notifications** (optional) show the status, the session-limit warning and the
   "Accessibility needs fixing" alert.
6. **Keep it running** (Xiaomi, Samsung, Huawei/Honor, OPPO/realme/OnePlus, vivo): the main
   screen shows the phone-specific steps (Autostart, Never sleeping apps, App launch…) and
   an *Open phone settings* button.

If the main screen later says the accessibility service is **"switched on but not
responding"**, the phone killed it without turning the switch off. Turn Flow Clone off and
on again in Accessibility settings, then redo steps 4 and 6. The bubble shows a red **!**
in that state; tapping it opens the main screen.

## 5. Import the model (SAF)

Main screen → **Speech model…** → **Import model file…**. The system file picker opens.
Choose `Download/ggml-base.en-q5_1.bin`, or the model you copied.

The app copies the file into its private storage (`files/models/`) and loads it. The
original in `Download/` can be deleted afterwards. The main screen should then show
`Native engine: flow-ffi 0.1.0` and `Loaded: ggml-base.en-q5_1.bin`. To switch models,
use *Use* on another imported file. *Delete* frees the space.

### Parakeet (optional, local / hybrid)

English-only, more accurate engine (D12). On the PC, get the folder
`sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` (`tools/fetch-models.sh --parakeet`; ~661 MB:
`encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`, `tokens.txt`) and copy the whole
folder to the phone. *Settings → Parakeet model folder… → Import model folder…* and pick the
folder; the app copies the four files into `files/parakeet/<name>/` and switches the engine.
*Settings → Use Parakeet* toggles back to whisper. If Parakeet fails to load (RAM, files), the
app loads the whisper model instead and the main screen says "Parakeet failed: …".

## 5b. Server mode (hybrid / remote)

1. On the server box: run `flow-server` (see `docs/protocol.md` / `crates/flow-server`), bound
   to its Tailscale `100.x` address or the LAN; `flow-server gen-token` prints the token.
2. Phone: install Tailscale (or be on the same LAN). No extra permission is needed.
3. Main screen → **Developer settings** (visible by default in hybrid/remote): host = the
   server's `100.x.y.z` or `name.tailnet.ts.net`, port `7313`, paste the token, *Save*,
   then *Test connection* → "OK · median N ms (…) · flow-server … · parakeet".
4. Dictate: *Active engine (last dictation)* shows `Server 100.x.y.z:7313`.

In the `local` flavour Developer settings are hidden: tap the version row at the bottom of
Settings 7 times to show them (only the client toggle and timeouts apply there).

## 6. AI cleanup (optional)

A small local language model can rewrite each transcript: punctuation, filler words,
"scratch that" corrections, lists. It is **off by default** and never uses the network.

1. On the PC, download one GGUF file in a browser:

   | File | Size | Use |
   |---|---|---|
   | `qwen2.5-0.5b-instruct-q4_k_m.gguf` | ~490 MB | **Recommended.** Fast; fixes punctuation and casing, misses some backtracking |
   | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | ~1.1 GB | Best quality, about 3× slower. Flagship phones with 8 GB+ RAM |

   From `https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF` (or `…1.5B…`), file
   *Files and versions* → the `q4_k_m` file.
2. Copy it to the phone's `Download/`.
3. *Settings → AI cleanup model… → Import model file…*. The import refuses non-GGUF files
   and checks free space; a 1 GB copy takes a minute.
4. *Settings → Clean up with local AI* → on, pick the strength, *Save*. The main screen
   shows `AI cleanup: qwen2.5-…gguf` once it has loaded (10–60 s).

If cleanup takes longer than 3 s, or changes the text too much, the plain transcript is
inserted instead (History shows `llm_timeout` / `llm_diverged`).

## 7. Use it

Main screen → **Start / show bubble**. Focus any text field and the bubble appears by the
keyboard:

- **tap** starts hands-free dictation; tap again to stop and insert;
- **hold** is push-to-talk; release to insert;
- drag the bubble to move it; drop it on the **zZ** target at the bottom to snooze it for
  10 minutes; **shake** the phone to bring it back (or tap the notification);
- after 5 s of no use it shrinks to a dot; a tap restores it.

While you speak, the line above the bubble shows the live transcript (Settings → Live
preview). "Can't hear you" means 3 s of silence; "No speech detected" means nothing was
inserted. History (search, copy, delete, raw vs cleaned) is on the main screen.

## Device test checklist

**Flavours / developer settings (D17)** — one APK of each flavour, same phone, installed over each other
- [ ] `local`: App info → Permissions shows no network; main screen text says "no internet
      permission"; Developer settings hidden until 7 taps on Settings' version row ("Developer
      settings unlocked"); *Allow server* shows "not in this build"; *Lock* hides it again.
- [ ] `local`: dictation works exactly as before (main screen `Loaded: client: …`); *Test
      session* says `OK: Client (on-device) · … EMPTY (no_speech)`.
- [ ] `hybrid` over `local`: dictionary/history kept. Developer settings visible; host
      validation: `http://x`, `10.0.0.1:7313`, `256.1.1.1`, `a_b.com` show inline errors and
      Save greys out; `8.8.8.8` / `example.com` lock *Use TLS* on.
- [ ] `hybrid`, server up (Tailscale): *Test connection* shows median RTT + server version;
      dictation → last engine `Server …`, optional badge `[S]` on the preview line; Server info
      shows "pipeline ✓".
- [ ] `hybrid`, airplane mode (or `systemctl stop flow-server`): dictation still inserts, last
      engine `Client (on-device) · fallback: connect_failed`, no words lost; a second dictation
      within 5 s shows `circuit_open`; Circuit row "open until hh:mm:ss"; *Reset circuit* closes it.
- [ ] `hybrid`, wrong token: Test connection "auth_failed: token rejected"; dictation falls back
      to the client and the circuit stays open until the server settings change.
- [ ] `hybrid`, kill the server mid-dictation (long hands-free session): the text is complete
      (fallback `disconnected`, buffered audio replayed).
- [ ] Both toggles off → Save shows the red "Dictation disabled" warning; tapping the bubble
      toasts "Dictation disabled — enable server or client in Developer settings".
- [ ] `remote` over `hybrid`: APK ~10x smaller; no Speech model / AI cleanup entries; Allow
      client "not in this build"; server down → bubble error "Server unreachable (connect_failed)".
- [ ] Settings changes apply to the next dictation without reloading the model (main screen
      keeps `Loaded:`; logcat `FlowEngine` shows no reload) unless *Keep … loaded* changed.
- [ ] Token survives an app restart (Keystore); *Clear* + Save removes it.
- [ ] Parakeet (local/hybrid, 6-8 GB phone): folder import, `Loaded: client: Parakeet …`, jfk-style
      dictation accuracy and latency; hotwords work (`TMPDIR` = cache dir); low-RAM phone falls back to whisper.
- [ ] core-p1: "comma"/"period" spoken → punctuation (Settings *Auto-format* on), off → literal
      words; "um uh" only → "Only filler words heard"; "… press enter" sends Enter once in a chat.

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

**AI cleanup** (`tools/build-native.sh --whisper --llm`; host-proven with Qwen2.5 0.5B/1.5B)
- [ ] Main screen shows `AI cleanup: qwen2.5-0.5b-instruct-q4_k_m.gguf`, not "failed to
      load". This is the first `System.loadLibrary("flow_llm")` plus the core's
      `dlopen("libflow_llm.so")` on bionic. Note the load time (Settings save → line appears).
- [ ] Dictate "um so I think we should meet tomorrow at noon". History shows *AI cleaned*
      and `llm … ms`. Note `asr`/`llm` ms, phone and SoC. Over 3 s → `llm_timeout` and the raw
      text (expected on slow phones; then try 0.5B or turn it off).
- [ ] Whisper and the LLM loaded together: no crash, no low-memory kill of the service
      while switching apps (both ggml copies coexist: `libflow_ffi.so` keeps whisper's, the
      plugin its own).
- [ ] No `SIGILL` in the LLM (same dotprod/fp16 arch flags as whisper).
- [ ] AI cleanup off → History shows *Transcript* and no `llm` time.

**Robustness (P1)**
- [ ] Drag the Ready bubble: the zZ target appears at the bottom after ~¼ s and grows when
      the bubble is near; drop → toast "Snoozed for 10 min", bubble gone, notification
      "bubble hidden". Tap the notification → it is back. Snooze again; shake twice → it is
      back with a click haptic. On the lock screen a shake does nothing.
- [ ] With no text field focused, shake → the bubble appears for 10 s, then hides.
- [ ] Ready bubble shrinks to a dot after 5 s; tap → full size and dictation starts.
- [ ] Battery step: the system "Allow app to always run in background?" dialog appears.
- [ ] OEM button opens the vendor's autostart / battery screen (record the phone model and
      which screen opened, or that it fell back to app info).
- [ ] Ghost check: with the service on, force-stop Flow Clone from App info, reopen it.
      Within ~15–60 s the main screen says "switched on but not responding" and the service
      notification shows the ghost text. Toggle off/on → "On".
- [ ] Reboot the phone: the bubble service comes back without opening the app (accessibility
      on), or a "Turn Flow Clone back on" notification appears (accessibility off).
- [ ] Update in place (reinstall with `-r`): the service restarts (MY_PACKAGE_REPLACED).
- [ ] *Stop bubble*: the service stays stopped (the a11y watchdog and the boot receiver
      respect it); *Start / show bubble* brings it back.

**Live preview / silence**
- [ ] While speaking for 5+ s, the preview line fills with words (1.5 s cadence) and
      dims while processing; the final insert can differ from it.
- [ ] Note the extra CPU: with preview on, commit latency rises slightly (partial decode
      aborted). If it's bad on the phone, turn *Live preview* off.
- [ ] Mute the mic / cover it: after 3 s "Can't hear you. Check the microphone."; stop →
      "No speech detected. Nothing was inserted." and nothing is typed.

**Dictionary & history**
- [ ] Add "wisper" → "Wispr", dictate "wisper flow" → "Wispr flow". Edit it, delete it.
- [ ] Add snippet "my address" → multi-line text; say only "my address" → the text.
- [ ] History: search finds the row; *Show the raw transcript* shows the pre-cleanup text;
      Copy and Delete work.

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
