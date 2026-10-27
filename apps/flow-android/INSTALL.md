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

## Live test on phone (Tailscale, ~10 minutes)

The fastest end-to-end test of the whole system: phone → your `flow-server` over Tailscale, with
the phone's own model as the fallback.

**Which APK.** Install **`hybrid`** (`dist/flow-clone-hybrid-debug.apk`): it talks to your server
and falls back to the on-device engine (the mock engine works without any model), and it is the
only flavour that can exercise both routes and the live "via …" label. Use `remote` only to test
the server path alone (no models, smaller), `local` to prove the no-network build.

APKs built on this PC (`tools/dist.sh`, after `tools/build-native.sh --flavor local|hybrid|remote`;
`dist/` is git-ignored; debug-signed with this PC's debug key):

| APK | Size | SHA-256 |
|---|---|---|
| `flow-clone-hybrid-debug.apk` | 47.6 MB (47591933 bytes) | `9becffa7e5d2fcbc702c43551e902904eb4c40f9d9df7a03743775aedf5cc794` |
| `flow-clone-hybrid-release-unsigned.apk` | 46.6 MB (46569077 bytes) | `cdcf4e7dfaba8832ad318f55eb549d9b76ae6ba0b7858be20d6b8f36f0595962` |
| `flow-clone-local-debug.apk` | 43.2 MB (43151837 bytes) | `b4caaf0ac0c45de57869358884d3546688204f2f2c9483a42ac531dee2afa0b1` |
| `flow-clone-local-release-unsigned.apk` | 42.1 MB (42128985 bytes) | `a94b827b8f4587e063189409804b41116a8f0dfe83f6c1c6250c28ed9b1ebf9e` |
| `flow-clone-remote-debug.apk` | 13.9 MB (13886807 bytes) | `acf571ce97f8337fbadc6290ab00ef7c54b491cc39eb0a746e0805a71ae4f990` |
| `flow-clone-remote-release-unsigned.apk` | 12.9 MB (12863951 bytes) | `53f236a1f3c13a946649e85f8d289629eb277eaf46f0cd6ceba53fb848a40c17` |

All six built 2026-10-24 from tag `v0.5.0` (914fb5e; D28 OpenRouter-only cloud: one OpenRouter page, opt-in cloud text cleanup, one-time migration of old provider settings; versionName 0.5.0, versionCode 500, native engine `flow-ffi 0.5.0+914fb5e`; natives rebuilt from the same tree for all three flavours; native gate: 41/41 JNI symbols, 8 `flow_llm_*`; D28 host scan clean for all six; same debug key as before (cert SHA-256 `63f0719b…`), so the debug APKs install over the previous build). The `*-release-unsigned.apk` files are not installable until signed, see "Release build" below.

Check a copy with `sha256sum flow-clone-hybrid-debug.apk` before installing it. An APK from
another machine or from the CI artifact (`.github/workflows/android-build.yml`) has a different
debug key: Android refuses it as an update, so uninstall the old app first (this deletes its
settings, dictionary and history).

**1. Server (on the PC).** Install Tailscale on the PC and the phone and log both into the same
tailnet. Then on the PC:

```sh
flow-server gen-token --out ~/.config/flow-server/token        # writes a new token (mode 0600), never overwrites
# config.toml: bind = ["127.0.0.1:7313", "<the PC's 100.x Tailscale IP>:7313"], token_file = that path
flow-server --config config.toml                                # crates/flow-server/dist/config.example.toml
```

`tailscale ip -4` prints the 100.x address; the token is the content of that file. Copy it to the phone over a channel you trust
(e.g. a Taildrop file or typing it); never paste it into a chat.

**2. Install + permissions (phone).** Copy the APK to the phone and open it in Files ("Install
unknown apps" for Files), or use the platform-tools install command over USB. Open Flow Clone
and grant, in this order: *Microphone* → *Display over other apps* → *Accessibility service*
(Settings → Accessibility → Flow Clone → on; if the switch is greyed out: Settings → Apps →
Flow Clone → ⋮ → *Allow restricted settings*, then retry) → *Notifications* → *Battery: allow
background* (each is a row in the main screen's *Setup* card: tap it). The main screen's *Status* card must show
*Native engine* `flow-ffi <version>+<commit>` (e.g. `flow-ffi 0.2.0+1a2b3c4`; D26).

**3. Point it at the server.** Main screen → *Developer settings* → *Server*: host = the PC's
`100.x.y.z` (or its MagicDNS name), port `7313`, *Paste* the token, *Use TLS* off, *Save* (top
bar), then *Test connection* (bottom) → `OK · median N ms … · parakeet`. *Routing & engines*:
*Allow server* + *Allow on-device* on (the defaults in hybrid), *Save*. Plain `ws://` is
only allowed when the address is private or tailnet **and** the phone reaches it through its
Tailscale interface: "Server needs TLS — turn on Use TLS …" means Tailscale is off on the phone
or the host is a public address. For on-device fallback, either import a model (§2, §5) or tick
*Settings → Mock engine*.

**4. Smoke checklist (10 minutes).** Note the phone model, Android version and pass/fail per line.
- [ ] Notes app, **tap** the bubble, say a sentence, stop talking: the preview says
      `via server`, and after ~1.5 s of silence the dictation ends by itself and the text is
      inserted (hands-free endpointing; *Settings → Timing & limits → Stop after silence*, default
      1500 ms since D22; a value you set yourself is kept, an untouched old 2000 becomes 1500).
- [ ] Long dictation (30 s+, with pauses), *Settings → Timing & limits → Decode while speaking* on (default): the
      text appears well under a second after you stop; the debug line shows `segments` ≥ 2 and
      `pre_asr_ms` > 0. Toggle it off: `segments` 0 and the wait grows with the length.
- [ ] **Hold** the bubble, speak, release: inserted; holding never stops by itself.
- [ ] The *Developer settings* status card says *Last dictation* `Server · client N ms`
      (*Status & server info* → *Active engine* has the full `Server 100.x…:7313 · …` line).
- [ ] Start a long tap dictation, stop `flow-server` (Ctrl+C) mid-sentence: the preview relabels to
      `via phone · server unreachable` while you are still talking, and the inserted text is
      complete (buffered audio replayed on the phone).
- [ ] Start the server again; the next dictation goes `via server` (after the 5 s circuit backoff,
      or *Developer settings → Status & server info → Reset circuit*).
- [ ] Start dictating in Notes, tap ✓, and switch to another app's text field before the text
      appears: nothing is inserted there; a toast says you switched fields and the bubble offers
      *Paste* (S12). In a banking app or a password field the bubble stays hidden and Paste refuses.
- [ ] Force a paste failure (e.g. dictate, then close the keyboard and leave the app): the toast
      says the text is on the clipboard **for 1 minute**; after a minute the clipboard holds your
      previous clip again, or is empty (S16; note the result, Android may refuse background
      clipboard access).
- [ ] Tap ✕ while "Transcribing…": nothing is inserted, no history row is added.
- [ ] Airplane mode on (Tailscale down): dictation still inserts from the phone engine, label
      `via phone · server unreachable`.
- [ ] **Never-fail fallback (D24)**: *Developer settings → Keep the on-device model loaded* **off**,
      Save, stop `flow-server` (or airplane mode). Tap the bubble and talk right away: the preview
      shows `Loading phone model… · server unreachable` for a few seconds (Parakeet ≈ 1 GB), then
      `via phone · …`; nothing you said is lost and the text is inserted. The debug line has
      `connect_ms` (≈ the connect timeout), `local_cold: true` and `local_load_ms`. The next
      dictation starts on the phone at once (`circuit_open`, model already loaded). Tap ✕ while
      it is still loading: nothing is inserted.
- [ ] Same with no model imported: the error says "Server unreachable (…) — phone fallback
      unavailable: No speech model imported…".
- [ ] logcat (`FlowService`, `FlowEngine`, `FlowA11y`): no `blocking native call on the main
      thread`, no crash; the token never appears.

Record the results (and the stop → text latency for server vs phone) in `PROJECT_CONTEXT.md`.

### Debug log (what you said and how long it took)

*Developer settings → Debug log & audio → Debug log* (includes transcripts) (off by default; in the
`local` flavour unlock Developer settings first, see below). While it is on, every dictation
writes **one JSON line** to logcat tag **`FlowDebug`** and to the app-private file
`files/debug/dictations.jsonl` (newest 500 entries). Engine load failures, model import
failures, failed server / cloud probes and accessibility health changes are logged there too.
Nothing is uploaded: the file leaves the phone only when you tap *Share / Export*.

Read it on the PC (USB or wireless debugging, platform tools): `logcat -s FlowDebug:I` (run it
through the platform tools' shell). On the phone: *View* lists the entries newest first;
*Share / Export* sends the `.jsonl` to an app you choose; *Clear* empties it. Turning the
toggle off asks whether to clear the file.

One line per dictation; the fields are the same in the Windows app:

| Field | Meaning |
|---|---|
| `ts` | dictation start, UTC (`2026-10-23T10:15:02.123Z`) |
| `mode` | `tap` (hands-free) or `hold` |
| `route` / `fallback` | `local` / `remote` / `cloud`; why the first route was not used (`connect_failed`, `disconnected`, `circuit_open`, …) or why the start failed (`server_unavailable`, …) |
| `engine` | Android: what produced the text: `whisper` / `parakeet` (phone), `server`, `cloud`, `mock`; `null` when no engine ran |
| `model` | Android: the whisper file or Parakeet folder name, the server's ASR model from the last successful *Test connection* / save probe (`null` before one), or the cloud model |
| `language` | Android: the language sent to the engine (`auto` = whisper detects it) |
| `rtt_ms` | server round trip (server route only) |
| `audio_ms` | audio length |
| `endpoint_wait_ms` | from your last word to the stop (hands-free: the silence wait). Silence is measured against the room's noise floor (as the core's VAD), so it is no longer always 0 on a phone; a very loud room still reads 0 |
| `asr_ms` / `post_ms` / `total_ms` | speech recognition / cleanup (rules + AI) / whole commit |
| `perceived_ms` | stop or release → text inserted: what you waited |
| `client_ms` | release → result on the phone, network included (router; `null` without a result). The latency to compare between routes: a server's `total_ms` is only its own commit |
| `wait_ms` | how long the commit waited for a decode already running (live preview or a segment); part of `total_ms` |
| `segments` | pieces sent to speech recognition (*Decode while speaking*: finished phrases + the tail); `0` = off, or the cloud route |
| `pre_asr_ms` | recognition time spent while you were still speaking; not part of `asr_ms`, which is now only the tail |
| `target` | app package; `<redacted>` in password fields, banking and denylisted apps (text too) |
| `insert` | `{"outcome": inserted / paste_failed / target_changed / target_protected / empty / error / timeout / start_failed / cancelled, "steps": [insertion trail]}` |
| `raw` / `text` | recogniser output / text inserted |
| `error` (+ `stack`) | what went wrong |
| `audio` | Android: file name of the kept WAV in `files/debug/audio/` (*Keep last dictation audio*), else `null` |
| `llm` | `formatted` (AI text used), `llm_skipped_clean` (nothing for the AI to do: a normal result), `llm_timeout`, `llm_error: <code>` (cloud cleanup refused up front: no consent / key / budget, open circuit, `no_private_route`), `null` = AI off |
| `llm_engine` / `llm_model` / `llm_cost_usd` | who ran the AI cleanup (`local`, `cloud` = OpenRouter, `server`), its model, and the OpenRouter cost |
| `llm_ttft_ms` / `llm_generation_id` | cloud cleanup: time to the first token, OpenRouter's generation id |
| `stt_cost_usd` | what cloud speech-to-text cost for this dictation |

Android also writes an `engine_mismatch` event when the phone engine that loaded is not the
one selected, e.g. `selected parakeet, loaded whisper ggml-base-q5_1.bin: model load failed: …`
or `… no Parakeet model folder imported`, or `selected whisper, loaded none: …` when the phone
engine failed behind the server. (D21 schema additions on Android, 2026-10-23: `engine`,
`model`, `language`, `audio`, event `engine_mismatch`.)

**Keep last dictation audio (debug)** (*Developer settings → Debug log & audio*, off by default): saves
what the microphone heard for the last 30 dictations as 16 kHz mono WAV files in the app's
private storage, `files/debug/audio/<ts with - for :>.wav` (e.g. `2026-10-23T10-15-02.123Z.wav`
for `"ts":"2026-10-23T10:15:02.123Z"`), linked from the line's `audio` field. Never for password
fields, banking or excluded apps, nor for cancelled dictations; files older than 72 hours are
deleted; *Clear audio* deletes them all. Nothing is uploaded. To replay one on the PC: copy it
out with the platform tools' `exec-out run-as dev.flowclone cat files/debug/audio/<name>.wav >
<name>.wav` (debug builds), then run it through `flow-cli` with the same model and language.

**Pull everything in one command** (debug builds): `tools/phone/pull-debug.sh [--since TS] [--out DIR]`
copies `dictations.jsonl`, the kept WAVs and both settings files (`flow_settings.xml`, `dev.xml`)
to `DIR` (default: a new temp dir; a directory inside the repo is refused) and prints one line,
e.g. `pulled 42 lines, 3 wavs (1 new, 2 already here), 2 prefs → /tmp/flow-debug.Ab12Cd (61234
bytes from phone)`. Read-only on the phone: the recordings stay there and the host copies are
yours to delete. Pull into the same `--out DIR` again and only WAVs not already there (by sha256)
cross the link; every pulled WAV is checked against the phone's sha256 (a mismatch is dropped and
the script exits 1). `--since 2026-10-27T10:00Z` keeps only lines and WAVs from then on. Settings
are redacted in memory before they reach disk: every entry whose name contains token, key,
secret, passw, auth, sealed, cred, bearer or cookie (the sealed server token
`server.token_sealed`, the cloud keys `cloud.key_sealed.<id>`) keeps its name but loses its value
(`<redacted>`, numbers 0, sets emptied), JSON inside a value (`cloud`) is redacted by key the same
way, and anything shaped like an API key (`sk-…`) or bearer token is replaced in the log too.
Pick the phone with `FLOW_PHONE`: a serial (`FLOW_PHONE=100.64.0.5:5555`) or transport options
(`FLOW_PHONE="-t 60"`); `FLOW_PKG` changes the package. Test: `tools/phone/test-pull-debug.sh`
(fake platform tool, no phone).

The server token and cloud keys are scrubbed from every line, and the clipboard is never read.
With the toggle off nothing is written; `FlowService commit …` (timings only, never text) is
always logged and since D22 also carries `wait=…ms segments=… pre_asr=…ms client=…ms`
(`client=-` when the result did not come through the router).

**Decode while speaking** (*Settings → Timing & limits*, default on, stored on this phone only): the
engine decodes each finished phrase (cut in the middle of pauses ≥ 0.8 s) while you keep
talking, so after you stop it only decodes the last phrase. Same text, less waiting on long
dictations. Used on the phone engine and your own server; the cloud route never gets audio
before you stop, so it is off there. Live preview and ✕ behave as before.

### Release build

`tools/dist.sh` also builds `assemble<Flavour>Release`. Signing comes only from outside git:
Gradle properties `flow.release.storeFile`, `flow.release.storePassword`, `flow.release.keyAlias`,
`flow.release.keyPassword` (e.g. in `~/.gradle/gradle.properties`) or the environment variables
`FLOW_RELEASE_STORE_FILE`, `FLOW_RELEASE_STORE_PASSWORD`, `FLOW_RELEASE_KEY_ALIAS`,
`FLOW_RELEASE_KEY_PASSWORD`. Without them the release APKs come out as
`*-release-unsigned.apk`, which Android will not install. To sign: create a key once with
`keytool -genkeypair -v -keystore <outside the repo>/flow-release.jks -alias flow -keyalg RSA
-keysize 4096 -validity 10000`, set the four properties and rerun `tools/dist.sh` (or sign an
unsigned APK with the SDK's `apksigner sign --ks …`). A release-signed APK and a debug one cannot
update each other: switching between them means uninstalling (and losing app data). The CI
workflow never signs release builds (no secrets in the repo or in Actions).

## 1. Build the APK (on the PC)

```sh
cd apps/flow-android
export SHERPA_ANDROID_LIBS=<sherpa-onnx-v1.13.8-android>/jniLibs/arm64-v8a   # Parakeet runtime (local/hybrid)
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"  # this box's libclang
tools/build-native.sh --flavor local     # -> app/src/local/jniLibs/arm64-v8a/
tools/build-native.sh --flavor hybrid    # -> app/src/hybrid/jniLibs/arm64-v8a/ (+ OpenRouter client)
tools/build-native.sh --flavor remote    # -> app/src/remote/jniLibs/arm64-v8a/ (libflow_ffi.so only, + OpenRouter client)
./gradlew testDebugUnitTest assembleDebug          # all three flavours + their gates
tools/probes/check_native_so.sh --flavor local app/build/outputs/apk/local/debug/app-local-debug.apk
```

`--no-parakeet` / `--no-llm` build smaller local/hybrid libraries. The outputs are
`app/build/outputs/apk/<flavour>/debug/app-<flavour>-debug.apk`, debug-signed. The build
fails if a flavour breaks its promise: `verifyLocalDebugNoNetworkPermissions` (+
`…NoNetworkCode`), `verifyHybridDebugOnlyInternetPermission`,
`verifyRemoteDebugOnlyInternetPermission` and `verifyRemoteDebugNoAsrLibs` (no
whisper/sherpa/onnxruntime/flow_llm/libc++ in the APK, no `whisper_`/`llama_` exports).
`…NoNetworkCode` also rejects the cloud STT client (`reqwest` / `webpki` / `flow-cloud`) in the
local library (D18 C2); `check_native_so.sh --flavor hybrid|remote` requires it there.

If you skip `build-native.sh`, the APK still installs, but the main screen's *Native engine* row
says "Unavailable". Only the mock engine works in that build. Without the LLM plugin
(`--no-llm`) dictation works, and turning on AI cleanup makes the main screen's *AI cleanup* row
say "Failed to load … plugin missing". If the main screen says **"Native library flavour
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
| `ggml-base-q5_1.bin` | ~57 MB | Multilingual. Language defaults to the phone's (see below) |
| `ggml-tiny.en.bin` / `ggml-base.en.bin` | 75 / 142 MB | Unquantised. The host tests use these, so they are the proven builds |

The `.en` models are English only. The multilingual ones (no `.en`) also auto-detect the
language, but detection is unreliable on short phone clips (a 2 s English dictation came out
as Bulgarian on `base-q5_1` + auto). So *Settings → Language* now defaults to **Phone
language**: with a multilingual whisper model loaded, the phone's language is used (if whisper
supports it); with an English-only file, Parakeet, or no phone engine it stays auto. Pick
*Auto-detect* or a language to override; an explicit choice is kept. A stored *Auto-detect*
from before this change is treated as the default once. The main screen shows the language the
next dictation uses ("Language: English (phone language)"). A direct link looks like
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

Open **Flow Clone**. The main screen's **Setup** card lists each permission with its state on
the right (*Granted*, *Allowed*, *On*) or the action that fixes it (*Grant*, *Allow*, *Turn on*);
tap the row. While a required one is missing, a red banner at the top names it and *Setup* sits
above *Status*.

1. **Microphone**: tap the row (*Grant*), then *While using the app*.
2. **Display over other apps**: tap the row (*Allow*) and switch it on. The bubble needs it.
3. **Accessibility service**: tap the row (*Turn on*) → *Flow Clone* → on.
   - If you installed from Files or a browser (step 3, second option), Android 13+ greys
     the switch out as a **restricted setting**. Fix it in *Settings → Apps → Flow Clone →
     ⋮ (top right) → Allow restricted settings*, then try again. USB installs aren't
     restricted.
4. **Battery optimisation**: tap the row (*Allow*) → *Allow*. Without it Android may stop the
   service and the bubble disappears.
5. **Notifications** (optional) show the status, the session-limit warning and the
   "Accessibility needs fixing" alert.
6. **Keep it running: <phone maker>** (Xiaomi, Samsung, Huawei/Honor, OPPO/realme/OnePlus, vivo):
   the last row of *Setup*; tap it to expand the phone-specific steps (Autostart, Never sleeping
   apps, App launch…) and the *Open phone settings* button.

If the *Accessibility service* row later says **Not responding** (banner: "the Accessibility
service stopped responding"), the phone killed it without turning the switch off. Tap *Fix*,
turn Flow Clone off and on again in Accessibility settings, then redo steps 4 and 6. The bubble
shows a red **!** in that state; tapping it opens the main screen.

## 5. Import the model (SAF)

Main screen → *More* → **Speech model** → **Import model file…**. The system file picker opens.
Choose `Download/ggml-base.en-q5_1.bin`, or the model you copied. The *Recommended* rows list
whisper files with size / speed / quality; tapping one copies its download link (for the PC).

The app copies the file into its private storage (`files/models/`) and loads it. The
original in `Download/` can be deleted afterwards. The import page's *Status* card shows it as
*Active model*; the main screen's *Status* card shows *Native engine* `flow-ffi <version>+<commit>`
(e.g. `flow-ffi 0.2.0+1a2b3c4`; D26) and *Speech engine* `Loaded: ggml-base.en-q5_1.bin`. To switch
models, tap *Use* under another imported file. *Delete* (asks first) frees the space.

### Parakeet (optional, local / hybrid)

English-only, more accurate engine (D12). On the PC, get the folder
`sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` (`tools/fetch-models.sh --parakeet`; ~661 MB:
`encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`, `tokens.txt`) and copy the whole
folder to the phone. *Settings → Speech engine → Parakeet model folder → Import model folder…* and pick the
folder; the app copies the four files into `files/parakeet/<name>/` and switches the engine.
*Settings → Speech engine → Whisper* switches back. If Parakeet fails to load (RAM, files), the
app loads the whisper model instead and the main screen says "Parakeet failed: …".
On the phone Parakeet decodes **greedy** (one recognizer; saves ≈ 0.6 GB RAM, dictionary words
are not used as hotwords). *Developer settings → Routing & engines → On-device decoding (Parakeet)* switches to *Auto* (beam
search with dictionary hotwords, plus a greedy recognizer) or *Beam only*; changing it reloads
the engine.

## 5b. Server mode (hybrid / remote)

1. On the server box: run `flow-server` (see `docs/protocol.md` / `crates/flow-server`), bound
   to its Tailscale `100.x` address or the LAN; `flow-server gen-token` prints the token.
2. Phone: install Tailscale (or be on the same LAN). No extra permission is needed.
3. Main screen → **Developer settings** (visible by default in hybrid/remote) → **Server**:
   host = the server's `100.x.y.z` or `name.tailnet.ts.net`, port `7313`, *Paste* the token,
   *Save* (top bar), then *Test connection* → "OK · median N ms (…) · flow-server … · parakeet".
4. Dictate: the hub's status card shows *Last dictation* `Server · client N ms`; *Status &
   server info → Active engine* shows `Server 100.x.y.z:7313 · …`.
5. **Server → Timeouts → Connect timeout** (default **2500 ms** since D24; a stored 800, the old default, becomes 2500
   once). Over a Tailscale DERP relay the handshake is 3+ round trips of 150-300 ms, which 800 ms
   could not fit. After a successful *Test connection* the router raises it further to
   `RTT × 6` (1.5-10 s) on slow paths; it never lowers what you set.
6. **The server being down never fails a dictation** while *Routing & engines → Allow on-device* is on (D24): the phone
   records at once and the audio is kept; if the server can't be reached (or its circuit is open,
   e.g. a failed pre-warm when you focused the field) the phone model takes over. With *Keep
   on-device model warm* off it is loaded on demand ("Loading phone model…") and then stays
   loaded. Dictation fails only when no route is possible (both toggles off, or no phone model),
   and the error says which.

## 5c. Cloud → OpenRouter (hybrid / remote, opt-in, third party)

OpenRouter (openrouter.ai) is the **only** cloud service the app can use (D28), for two separate
opt-ins: **cloud speech-to-text** (your audio) and **text cleanup** (the dictated text, not your
voice). Both are off by default and each has its own consent; turning one on never turns the
other on. No other provider is in the app any more, and no extra permission: it uses `INTERNET`.
The `local` flavour has no cloud at all.

Open it from *Settings → Dictation → Cloud (OpenRouter)* or *Developer settings → OpenRouter (cloud)*:

1. **API key**: paste a key from openrouter.ai/keys (*Paste*), *Save key*. It is stored only
   Keystore-encrypted; the page shows `••••` + the last 4 characters and other accessibility
   services only see "set (hidden)". *Check key* → "Key OK · <label> · credit left $x of $y ·
   this month $z" (and a warning if the key has no credit limit). One key serves both uses.
2. **Privacy**: *Zero-retention providers only* and *Deny data collection* are on; turning
   either off asks first ("Some providers may keep or train on your audio and text").
3. **Speech-to-text**: switch on *Cloud speech-to-text*. The dialog "Send your voice to
   OpenRouter?" says the audio leaves the phone; *Send audio to OpenRouter* records consent and
   puts *Cloud* first in the route order (reorder under *Developer settings → Routing & engines*).
   *Model* opens the model picker (default `openai/whisper-large-v3-turbo`, about $0.012-0.04
   per hour of audio). *Test speech-to-text* uploads a 3 s public-domain clip → "OK · heard "And
   so, my fellow Americans" · N ms · $… · <model>".
4. **Text cleanup**: switch on *Clean up text with OpenRouter*. Its own dialog ("Send dictated
   text to OpenRouter?") lists exactly what is sent: the dictated words, the kind of app, your
   style and your dictionary words; *What cleanup sends* adds text around the cursor, on-screen
   words or the app name, each off by default. *OpenRouter first* (phone AI as backup) or *Phone AI
   first*; *Provider choice* (fastest by default); *Time limit* (2500 ms, 800-8000: after it the
   formatted text is pasted and the request is cancelled). It runs on phone and cloud dictations,
   never on your own server's. With it on, the cleanup step runs even when the local AI is off.
5. **Model picker**: every OpenRouter model that can do the job, from the list OpenRouter
   publishes (cached 24 h; an offline list is built in; refreshed only when the OpenRouter page
   or the picker opens, or a use is switched on). Search by name or id; chips *Transcription* /
   *Chat audio* (speech) or *Text* (cleanup), *Zero-retention only* (on), *Fast providers*,
   *Free*. Each row: name (★ = recommended), id, price (per hour of audio, or ≈ per dictation),
   context length, badges (`ZDR`, `fast`, `may paraphrase` for chat models used as speech-to-text,
   `blocked by account`). *Use custom model id…* takes any id (e.g. a `:nitro` variant), checked
   as you type.
6. **Usage**: "Speech $x · Cleanup $y · this month", one *Monthly budget* for both (they pause
   once it is reached), *Reset counters*.

Upgrading from 0.4.0 or older: on the first start the old provider list is converted once. An
OpenRouter entry keeps its key, model and consent; every other provider and its stored key is
deleted. If another provider was the active one, cloud speech-to-text is switched off (not moved
to OpenRouter). A banner on *Developer settings* and on the OpenRouter page says what changed
until you tap *Dismiss*.

## Settings layout

Main screen → *Settings*: the same look as Developer settings. A *Status* card on top (engine
state, the language the next dictation uses, warnings such as "AI cleanup is on but no GGUF
model is imported"), then grouped rows that each show their current value and open a page:

| Section · row | Page contents | Applies |
|---|---|---|
| Dictation · *Language* | Auto-format; dictation language (Phone language first) | at once |
| Dictation · *Writing style* | Formal / Casual / Very casual for Personal messages, Work messages, Email, Other, each with an example line (default: chats casual, email + other formal); *Style per app* | at once |
| Writing style · *Style per app* | `package = personal`, `work`, `email` or `other`, one per line (checked); overrides the built-in app list | *Save* |
| Dictation · *Speech engine* (local / hybrid) | Whisper or Parakeet; *Speech model (whisper)*, *Parakeet model folder* imports | at once (reloads) |
| Dictation · *AI cleanup* (local / hybrid) | Clean up with local AI; *Clean up with OpenRouter* (hybrid, → the OpenRouter page); *AI cleanup model* import; strength. The row says "OpenRouter (<model>), then on-device" while cloud cleanup is on | at once (reloads) |
| Dictation · *Cloud (OpenRouter)* (hybrid / remote) | The OpenRouter page (§5c): key, privacy, speech-to-text, text cleanup, usage | at once |
| Dictation · *Timing & limits* | Stop after silence (ms, 0 or 300–10000), Decode while speaking, session limit (1–20 min) | *Save* |
| Bubble & apps · *Bubble* | Live preview, Shrink to a dot, Shake to restore | at once |
| Bubble & apps · *Excluded apps* | package names, one per line (checked) | *Save* |
| Privacy · *History* | Keep / 24 hours / Never save (asks before deleting) | at once |
| Advanced | *Developer settings* (when visible), *Mock engine* switch | at once |
| About | version + flavour; 7 taps unlock Developer settings (local) | – |

The rule: a page with typed fields has *Save* in the top bar (grey until something changed and
every field is valid; errors under the field; leaving with unsaved changes asks *Discard
changes?*); switches and choices apply at once, like Developer → *Debug log & audio*. The main
screen's *Language* and *AI cleanup* lines open their pages.

## Developer settings layout

A hub with grouped cards; each row shows its current value and opens a focused page:

| Hub row | Page contents | Save |
|---|---|---|
| *Status* card | route in use, last dictation engine + `client_ms`, circuit; *Status & server info* → active engine, circuits, server info (last probe), native library; *Reset circuit* | – |
| *Routing & engines* | Allow server / on-device, route order (↑ / ↓), keep model warm, engine badge, Parakeet decoding (applies at once); *Test session* | top bar |
| *Server* | host, port, Use TLS, certificate pin, token (masked; Show / Paste / Clear), timeouts; *Test connection* | top bar |
| *OpenRouter (cloud)* | The OpenRouter page (§5c); the row says "Speech on/off · Cleanup on/off" (+ "no key", budget). A one-time migration notice sits on top of the hub until dismissed | at once |
| *Debug log & audio* | Debug log, Keep last dictation audio (apply at once); View / Share / Clear | – |
| *Build* | flavour + native library, *Reset to defaults* (asks first), *Lock* (local) | – |

*Save* stays grey until something on the page changed and every field is valid (errors show
under the field; a bad stored value on another page is named in the page's banner). Leaving a
page with unsaved changes asks *Discard changes?*. Secrets never appear in the accessibility
tree (other accessibility services only see "set (hidden)").

## Local flavour: Developer settings

In the `local` flavour Developer settings are hidden: tap the version row (*Settings → About*)
7 times to show them (only the on-device toggle and timeouts apply there).

## 6. AI cleanup (optional)

Formatting runs without it: while *Settings → Language → Auto-format* is on, the phone already
handles spoken punctuation, "quote … unquote", "in parentheses", "dash", fillers, "scratch that",
numbers, lists and your *Writing style* (chats casual: no final period). A small local language
model can additionally polish each transcript: run-on sentences, leftover fillers, corrections.
It is **off by default** and never uses the network.

1. On the PC, download one GGUF file in a browser:

   | File | Size | Use |
   |---|---|---|
   | `qwen2.5-0.5b-instruct-q4_k_m.gguf` | ~490 MB | **Recommended.** Fast; fixes punctuation and casing, misses some backtracking |
   | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | ~1.1 GB | Best quality, about 3× slower. Flagship phones with 8 GB+ RAM |

   From `https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF` (or `…1.5B…`), file
   *Files and versions* → the `q4_k_m` file.
2. Copy it to the phone's `Download/`.
3. *Settings → AI cleanup → AI cleanup model → Import model file…*. The import refuses non-GGUF files
   and checks free space; a 1 GB copy takes a minute.
4. *Settings → AI cleanup → Clean up with local AI* → on, pick the strength (both apply at once). The main screen's
   *AI cleanup* row shows `qwen2.5-…gguf` once it has loaded (10–60 s; a little longer than in 0.2,
   because the model's instructions are prepared at load).

When the formatted text leaves nothing for the model to do, it is skipped: History and
Developer → *Active engine* show *AI not needed (already clean)*, a normal result. If cleanup
takes longer than 3 s, or changes the text too much, the formatted transcript is inserted
instead (History shows *AI too slow, formatted text used* / *AI result not used (llm_dropped)*).

## 7. Use it

Main screen → **Start / show bubble**. Focus any text field and the bubble appears by the
keyboard:

- **tap** starts hands-free dictation; tap again to stop and insert;
- **hold** is push-to-talk; release to insert;
- drag the bubble to move it; drop it on the **zZ** target at the bottom to snooze it for
  10 minutes; **shake** the phone to bring it back (or tap the notification);
- after 5 s of no use it shrinks to a dot; a tap restores it.

While you speak, the line above the bubble shows the live transcript (*Settings → Bubble → Live
preview*). "Can't hear you" means 3 s of silence; "No speech detected" means nothing was
inserted. Main screen → *More* → *History* lists dictations by day; tap one for the full text,
the raw transcript and per-stage timings. A dictation that never reached its field (you switched
fields, a protected field) says **Not inserted — tap to copy**: tap the row to copy it. Its detail
page says why (*You switched apps*, *Protected field*, *Couldn't paste*) and offers *Insert with
the bubble*: go back to the app, tap the field, then ✓ on the bubble. The badge names where it was
transcribed (*Phone*, *Server*, *Cloud*) for dictations saved from 0.4.0 on.

## Screens

Every screen uses the Settings look (grouped cards, rows with their current value):

| Screen | Contents |
|---|---|
| Main | banner while a required permission is missing; *Status* (Native engine, Speech engine, Language → Settings · Language, AI cleanup → Settings · AI cleanup, Version); *Setup* (5 permissions + *Keep it running*); *Run* (Start / show bubble, Stop bubble); *More* (Settings, Dictionary & snippets, History, Speech model (local / hybrid), Developer settings (when visible)) |
| Dictionary & snippets | *Dictionary* and *Snippets* rows with counts; *Import JSON*, *Export JSON*. Each list: *Add* in the top bar, search from 8 entries, tap to edit, TalkBack action / long-press to delete. Edit page: *Save* in the top bar when changed + valid, Discard prompt, *Snippet* switch, *Delete* |
| History | *Keep dictations* (→ Settings · History), *Show the raw transcript*, search from 8 entries, rows grouped by day (first line, time · app, result badge); *Clear all* in the top bar. Detail: text, raw transcript, when / app / result / inserted / timings, *Copy*, *Delete dictation* |
| Speech model / Parakeet model / AI cleanup model | *Status* (Active model, Size, Engine), *Import model file…* / *Import model folder…* with progress and errors under it, *Recommended* (tap = copy download link), *Imported* (*Use*, *Delete* asks first) |
| OpenRouter (Settings → Cloud (OpenRouter), Developer settings) | *API key* (masked; Show / Paste / Clear, *Save key*, *Check key*), *Privacy*, *Speech-to-text* (opt-in + consent, Model, Language, Send dictionary words, *Test*), *Text cleanup* (opt-in + its own consent, Model, order, Provider choice, Time limit, Status), *What cleanup sends*, *Usage* (budget, Reset counters) |
| Model picker (OpenRouter → Model) | *In use*, list source + *Refresh*, search, filter chips, rows (name, id, price, context, badges), *Use custom model id…* |

## Device test checklist

**Flavours / developer settings (D17)** — one APK of each flavour, same phone, installed over each other
- [ ] `local`: App info → Permissions shows no network; main screen text says "no internet
      permission"; Developer settings hidden until 7 taps on the version row in *Settings → About* ("Developer
      settings unlocked"); *Routing & engines → Allow server* shows "Not in this build"; *Lock* hides it again.
- [ ] `local`: dictation works exactly as before (main screen `Loaded: client: …`); *Test
      session* (*Routing & engines*) says `OK: Client (on-device) · … EMPTY (no_speech)`.
- [ ] `hybrid` over `local`: dictionary/history kept. Developer settings visible; host
      validation (*Server* page): `http://x`, `10.0.0.1:7313`, `256.1.1.1`, `a_b.com` show inline
      errors and the top-bar *Save* greys out; `8.8.8.8` / `example.com` lock *Use TLS* on.
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
- [ ] Both toggles off (*Routing & engines*) → the red "Dictation disabled" banner (also on the hub's status card); tapping the bubble
      toasts "Dictation disabled — enable server or client in Developer settings".
- [ ] `remote` over `hybrid`: APK ~10x smaller; no Speech model / AI cleanup entries; Allow
      client "not in this build"; server down → bubble error "Server unreachable (connect_failed)".
- [ ] *Settings* hub: every row's grey line matches its page; toggling *Bubble → Live preview*
      changes the hub row on Back without *Save*; *Timing & limits* *Save* stays grey for 25 min
      (red "Enter 1–20 minutes") and Back after an edit asks *Discard changes?*; TalkBack reads
      each switch row as one switch and each choice as a radio button; dark mode follows the phone.
- [ ] Settings changes apply to the next dictation without reloading the model (main screen
      keeps `Loaded:`; logcat `FlowEngine` shows no reload) unless *Keep on-device model warm* changed.
- [ ] Token survives an app restart (Keystore); *Clear* + *Save* removes it. With TalkBack (or a
      UI dump) the token field reads "set (hidden)", also after *Show*.
- [ ] Parakeet (local/hybrid, 6-8 GB phone): folder import, `Loaded: client: Parakeet …`, jfk-style
      dictation accuracy and latency; hotwords work (`TMPDIR` = cache dir); low-RAM phone falls back to whisper.
- [ ] core-p1: "comma"/"period" spoken → punctuation (*Settings → Language → Auto-format* on), off → literal
      words; "um uh" only → "Only filler words heard"; "… press enter" sends Enter once in a chat.

None of this can be checked on the build machine. Record the results in
`PROJECT_CONTEXT.md`.

Logcat tags: `FlowA11y` (insertion step trail), `FlowService`, `FlowMic`, `FlowEngine`.

**OpenRouter (D28)** — `hybrid` (and `remote`) APK and a real OpenRouter key; record model,
latency (stop → text) and the cost shown, in `docs/cloud-stt.md` / `PROJECT_CONTEXT.md`
- [ ] `local`: no *Cloud (OpenRouter)* row in Settings; Developer settings → OpenRouter says "Not in this build";
      App info shows no network permission (unchanged).
- [ ] Upgrade over a 0.4.0 install that had a Groq provider active: the hub shows "Cloud speech-to-text now
      uses OpenRouter only. Groq was removed and its key deleted…"; cloud speech-to-text is off; *Dismiss* hides it.
- [ ] Fresh install: both opt-ins off, both privacy switches on. *Paste* + *Save key*: the field empties and
      `Stored key: ••••abcd`; *Check key* → "Key OK · … · credit left …"; a wrong key → "auth_failed: key rejected".
- [ ] *Cloud speech-to-text* on: "Send your voice to OpenRouter?"; *Cancel* leaves it off; accept → *Routing &
      engines* shows `1. Cloud: OpenRouter (whisper-large-v3-turbo)`. Text cleanup stays off.
- [ ] *Test speech-to-text* → "OK · heard "And so, my fellow Americans…" · N ms · $…".
- [ ] Dictate 10 s in Notes: preview badge `[C]` even with *Engine badge* off; *Active engine* `cloud ·
      OpenRouter · openai/whisper-large-v3-turbo · $…`; Usage "Speech $… · Cleanup $0".
- [ ] *Clean up text with OpenRouter* on: its own dialog lists what is sent; turn *Send app name* on and
      reopen the dialog (off → on): it now names the app's name. Dictate a messy sentence ("um so like
      send it tomorrow comma ok"): History shows *AI cleaned*; the debug line has `llm_engine: cloud`,
      `llm_cost_usd`; Usage "Cleanup $…". A clean sentence logs `llm: llm_skipped_clean` and costs nothing.
- [ ] Model picker: list shows *Offline list (snapshot 2026-10-24)* until refreshed, then *Live list ·
      refreshed hh:mm*; *Chat audio* chip lists chat models with "may paraphrase" (picking one warns first);
      *Zero-retention only* off shows more; *Use custom model id…* rejects `Bad Id`, accepts
      `meta-llama/llama-3.1-8b-instruct:nitro`.
- [ ] Airplane mode, cloud speech first and on-device second: text from the phone model, toast "Cloud STT
      failed (no internet); used on-device"; cleanup falls back to the phone AI (or formatted text) within
      the time limit, no words lost.
- [ ] Pick a model without a zero-retention provider with ZDR on (custom id `openai/gpt-transcribe`): toast /
      status "no zero-retention provider serves this model".
- [ ] Monthly budget lower than this month's cost → "monthly budget reached"; *Reset counters* resumes.
- [ ] *Reset to defaults* (Developer) deletes the key (`no key`) and turns both uses off; usage survives.
- [ ] logcat (`FlowEngine`, `FlowService`, `FlowDebug`) never shows the key (search for its last 8 characters).

**Native engine / packaging**
- [ ] The main screen's *Native engine* row shows `flow-ffi <version>+<commit>` (e.g. `flow-ffi 0.2.0+1a2b3c4`; D26), not "Unavailable". This is
      the first time `System.loadLibrary` runs on bionic: `libc++_shared.so` is resolved,
      and there is no `dlopen` error under `FlowEngine`.
- [ ] Android 15 / 16 KB-page device (e.g. Pixel 8+ with the 16 KB developer option): the
      app installs and no "page size compatibility" dialog appears.
- [ ] Model import: progress under *Import model file…*, then *Active model* in the page's *Status* card. Note the model load time ("Loading
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
- [ ] The main screen's *AI cleanup* row shows `qwen2.5-0.5b-instruct-q4_k_m.gguf`, not "Failed to
      load". This is the first `System.loadLibrary("flow_llm")` plus the core's
      `dlopen("libflow_llm.so")` on bionic. Note the load time (switch on → line appears).
- [ ] Dictate "um so I think we should meet tomorrow at noon". History shows *AI cleaned*
      and its detail page `llm … ms`. Note `asr`/`llm` ms, phone and SoC. Over 3 s → `llm_timeout` and the raw
      text (expected on slow phones; then try 0.5B or turn it off).
- [ ] Whisper and the LLM loaded together: no crash, no low-memory kill of the service
      while switching apps (both ggml copies coexist: `libflow_ffi.so` keeps whisper's, the
      plugin its own).
- [ ] No `SIGILL` in the LLM (same dotprod/fp16 arch flags as whisper).
- [ ] AI cleanup off → History shows *Transcript* and no `llm` time.
- [ ] AI cleanup off, Auto-format on: "she said quote hello unquote" comes out with `"hello"` in quotation marks
  (formatting rules run without the model, D27).

**Writing style** (D27)
- [ ] *Settings* hub row *Writing style* reads `Chats casual · Email formal` on a fresh install.
- [ ] WhatsApp / Messages: "sounds good see you there" ends without a final period; Gmail: the same words keep it.
- [ ] *Writing style → Work messages → Formal* applies at once: the next Slack dictation keeps the period.
- [ ] *Style per app*: `com.google.android.keep = personal` → *Save*; a Keep dictation now drops the final period.
- [ ] Developer debug log line shows `style` (`casual` in a chat app) and `llm` (`llm_skipped_clean` reads
  *AI not needed (already clean)* in the viewer).

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
- [ ] *Dictionary & snippets → Dictionary → Add*: "wisper" → "Wispr" (*Save* enables only once valid;
      a duplicate word shows its error under the field), dictate "wisper flow" → "Wispr flow". Edit it, delete it.
- [ ] *Snippets → Add*: "my address" → multi-line text; say only "my address" → the text.
- [ ] History: rows grouped under *Today*; search finds the row; *Show the raw transcript* shows the
      pre-cleanup text; a row's detail page has *Copy* and *Delete dictation* (asks first).
- [ ] Switch fields while a dictation is processing: its History row says *Not inserted — tap to copy*;
      tapping it copies the text (toast "Copied"). Its detail page says *You switched apps*; *Insert with
      the bubble* → open another app's field → ✓ on the bubble types it there. The notification's copy
      and History's copy give the same text (a list keeps its line breaks).
- [ ] TalkBack: main screen rows read as one item each ("Microphone, Granted…"); *Keep it running*
      announces expanded / collapsed; Dictionary rows offer *Delete* in the actions menu.

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
