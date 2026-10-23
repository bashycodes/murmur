# flow-windows — Windows shell of the local Wispr Flow clone

Tauri 2 tray app: hold **Ctrl+Win** to dictate, **Ctrl+Win+Space** (or double-tap
Ctrl+Win) for hands-free, **Esc** to cancel, **Shift+Alt+Z** to paste the last
transcript again, **Shift+Alt+X** to copy it. Text is pasted into the focused app.
Speech-to-text runs either on this PC (the real `crates/flow-core`: whisper.cpp
or Parakeet) or on **your own** `flow-server` over LAN/Tailscale (hybrid mode,
`docs/hybrid-architecture.md`, D17). Every dictation goes through
`flow_router::Router` (`src/router_adapter.rs`). Nothing reaches a third party
unless you opt in to a cloud speech-to-text provider in Developer settings
(D18, `docs/cloud-stt.md`; off by default). A client-only build contains no
network code at all.

## Build flavours (Cargo features)

| Build | Command | Engines |
|---|---|---|
| Hybrid (default) | `cargo build --features app` | this PC + your server, fallback between them, plus the opt-in cloud provider (feature `cloud`) |
| Client only | `cargo build --no-default-features --features local,app` | this PC only; no network code linked |
| Server only | `cargo build --no-default-features --features remote,app` | your server only; no whisper / `flow_llm.dll` / models needed |

`parakeet` adds the Parakeet-TDT engine to `local` (needs `SHERPA_ONNX_LIB_DIR`
at build time; ship `sherpa-onnx-c-api.dll` + `onnxruntime.dll` next to the exe).
`local-engine` is the local backend without a native ASR engine (what the MSVC
cross-check builds).

## Developer settings (hybrid mode)

Settings → **Developer** (always visible in builds with `remote`; in a
client-only build tap the version line under General → App 7 times, "Lock"
hides it again):

* **Allow server** / **Allow client** — both on: the first entry of the route
  order runs, the other is the fallback (server first by default; audio pushed
  before a failure is replayed locally, nothing is lost). One on: only that
  engine. Both off: dictation shows "Dictation disabled — enable server or
  client in Developer settings".
* Server **host/IP** (IPv4, IPv6, or a name such as `box.tailnet.ts.net`;
  no `ws://`, path or port) and **port** (default 7313), TLS + optional
  certificate pin (required and forced for a public address), **token**
  (from `flow-server gen-token`; stored DPAPI-encrypted for your Windows
  account, never logged), connect / commit timeouts, health cache, keep the
  local model warm, S/L badge on the Flow bar.
* **Test connection** probes the unsaved form (hello + 3 pings: median RTT,
  server version, ASR engine). **Test session** sends 1 s of silence through
  the saved settings and should answer `EMPTY (no_speech)` and say which
  engine ran. Status rows: build capabilities, last dictation's engine /
  fallback / timings, server info (pipeline-version match), circuit breaker.
* Saving applies to the next dictation (no restart; the model reloads
  only when the Parakeet decoding below changed).
* **Speech decoding** (D22, `docs/proposals/streaming-asr-shell-api.md`):
  **Decode while speaking** (default on; `developer.decode_while_speaking`)
  sends `segment_pause_ms: 800` with every session, push-to-talk and
  hands-free, so each finished phrase is transcribed while you keep
  talking and only the tail is left at release; off omits the field (one
  decode after you stop, as before). The core turns it off by itself for
  cloud providers. **Parakeet decoding** (`developer.parakeet_decoding`):
  `auto` (default: beam with dictionary words, greedy otherwise; keeps a
  second recognizer, ≈ +0.6 GB RAM) or `greedy` (one recognizer, no
  dictionary biasing). Whisper ignores it.
* Active engine row leads with the D22 history line `total {client_ms or
  total_ms} ms · asr {asr_ms} (+{pre_asr_ms} early) · wait {wait_ms} ·
  {segments} seg`; `client_ms` is release → result on this PC (network
  included), `engine` is the engine's own `total_ms` when it differs.

### Cloud providers (sends audio to a third party)

Default build only (feature `cloud`; never in `local,app`, enforced by
`tools/check-build-matrix.sh`). Off by default.

* **Allow cloud** puts *Cloud provider* first in the route order (reorder as
  you like) and asks for consent: "Send audio to <provider>?" names the
  provider's host. Consent is stored per provider for that host; changing
  the base URL host or the adapter clears it and the dialog asks again.
* **Add provider** from a preset: OpenRouter, ElevenLabs Scribe, Groq,
  OpenAI, Deepgram, AssemblyAI, or Custom (any OpenAI-compatible
  `/audio/transcriptions` base URL). Per provider: name, model, language,
  send dictionary / screen words, zero retention where supported.
* **API key**: Save key / Clear key. Keys go to Windows Credential Manager
  as `flow-clone/cloud/<id>` (generic credential, this user on this PC) and
  into the router via `set_cloud_key`. They are never in `prefs.json`, never
  logged, and never sent back to the page (it shows `••••` + last 4).
  Deleting a provider deletes its credential.
* **Test provider** sends a bundled 3 s public-domain clip with the unsaved
  form (and a typed, unsaved key): transcript, latency, cost or the error
  with a hint.
* **Usage** this month per provider (dictations, minutes, cost, `≈` when
  estimated) is kept in `%APPDATA%\FlowClone\cloud_usage.json` (restored
  at startup, never sent anywhere); **Reset counters**; optional **monthly
  budget** (cloud pauses with "monthly budget reached").
* Active engine row: `cloud · <provider> · <model> · $0.0012`. The Flow bar
  shows **C** after every cloud dictation (regardless of the S/L setting).
  A cloud failure that fell back shows "Cloud STT failed (key rejected);
  used this PC" above the pill; cloud-only failures are an error pill.

## Debug log (opt-in, includes transcripts)

Settings → Developer → **Debug log (includes transcripts)**, off by default
(`prefs.json` → `developer.debug_log`; saved as soon as it is toggled). When
on, each dictation appends one JSON line to
`%APPDATA%\FlowClone\debug\dictations.jsonl` (the last 500 are kept) and
prints it to stdout as `[debug] {json}`. Non-dictation events (engine/LLM
load, server probe, hotkey hook, audio stream, paste-last, settings) get a
`"kind":"event"` line with `ts, event, error, stack`. The tab shows the line
count, **Open log folder** and **Clear**; turning it off offers to delete
the file. Off = no file, no line, no text anywhere (tested).

Schema and value conventions are shared with the Android app (decision D21
in `docs/DECISIONS.md`; every key always present, `null` when unknown):
`kind` (`dictation`), `ts` (UTC ISO 8601), `mode` (`hold` push-to-talk /
`tap` double-tap lock / `handsfree`), `route` (`local` / `remote` /
`cloud`), `fallback` (router failure code: `connect_failed`,
`disconnected`, `circuit_open`, … — from the route, else the live route
the pill showed, else the error text, so a server-only failure still names
it), `rtt_ms`, `audio_ms`, `endpoint_wait_ms` (last speech → stop; same
−45 dBFS rule as Android, `null` when no speech), `asr_ms` (with
decode-while-speaking: only the tail decoded at release), `wait_ms`
(commit waited for a decode already in flight), `segments` (pieces sent
to ASR, 0 = off), `pre_asr_ms` (ASR done while you were still talking),
`client_ms` (router release → result incl. network; `null` when the
router did not measure it) — these four are `null` without a result, like
Android (D22) —, `post_ms`
(LLM + post-processing = `total_ms - asr_ms`), `total_ms` (engine),
`perceived_ms` (stop → outcome), `target` (foreground exe), `insert`
(`{"outcome", "steps"}`; outcome `inserted` / `paste_failed` /
`target_changed` / `target_protected` / `empty` / `error` / `timeout` /
`start_failed` / `cancelled`, steps = the paste plan such as
`["ctrl_v", "key Enter"]`), `raw`, `text`, `error`, `stack` (always `null`
here).

It can never affect a dictation: `DebugLog::log` only pushes onto a
bounded queue (oldest dropped when full) and a writer thread does the
serialising and I/O; every failure there is swallowed and counted (shown
next to the count). Never logged: the server token and cloud API keys
(scrubbed from every line), clipboard contents; a password field gives
`target`, `raw` and `text` = `"<redacted>"` and no steps. Nothing is uploaded anywhere.
Code: `src/debug_log.rs`, lines built in `src/controller.rs`.

## Speech model (never downloaded automatically)

Put a whisper.cpp ggml model at `%APPDATA%\FlowClone\models\ggml-base.bin`
(the default: multilingual, CPU tier per `docs/DECISIONS.md` D4), or choose
any ggml file in Settings → Speech model (`model_path` in `prefs.json`;
`ggml-small-q5_1.bin` is the accuracy step up). The repo's
`tools/fetch-models.sh` downloads models on explicit request only.
The model loads in the background at startup. A missing or broken file
shows in the tray menu and tooltip, opens Settings, and makes a dictation end
in an error pill that names the path.

## AI cleanup (optional, off by default)

Settings → AI cleanup turns on llama.cpp cleanup (fillers, "scratch that",
spoken lists, per-app style; DECISIONS D5/D10/D11). It needs two files, and
neither is downloaded automatically:

* the GGUF model, default `%APPDATA%\FlowClone\models\qwen2.5-0.5b-instruct-q4_k_m.gguf`
  (`qwen2.5-1.5b-instruct-q4_k_m.gguf` is more accurate, ~2x slower);
  `tools/fetch-models.sh --llm` fetches both on explicit request;
* the engine plugin `flow_llm.dll` **next to `flow-windows.exe`** (or the
  path in "Engine library"). llama.cpp cannot be linked into the exe:
  whisper.cpp and llama.cpp each bundle a different ggml (D10).

Speech recognition is ready long before the LLM (10-50 s load on CPU);
until then, and whenever cleanup fails or exceeds its budget, the plain
transcript is pasted. The tray line "AI cleanup: …" and Settings show the
state (off / loading / ready / model missing / engine missing / failed).

## Layout

| Path | What |
|---|---|
| `src/hotkeys/chord.rs` | Pure chord matcher: PTT / hands-free / cancel, Win/Alt release masking, foreign-key abort |
| `src/hotkeys/win.rs` | `WH_KEYBOARD_LL` hook thread. It skips our own `dwExtraInfo`-tagged input, does masked Win/Alt releases (`[LCtrl↓, key↑, LCtrl↑]`) and runs a 1 s stale-key probe |
| `src/dictation.rs` | Pure state machine `idle → listening(ptt\|locked) → processing → pasted\|error → idle`. Wispr's timings: 500 ms tap / double-tap lock / triple-press, warning at 19 min, hard stop at 20 min, 35 s processing timeout, 500 ms error |
| `src/controller.rs` | Runs the machine's effects against a `Platform` trait plus `CoreApi`; integration-tested with fakes |
| `src/audio/` | Resampler to 16 kHz mono i16 and a Wispr-style level meter (pure); `capture.rs` = cpal/WASAPI |
| `src/paste/` | Pure strategy, restore guard and per-app restore delay; `win.rs` = clipboard snapshot → CF_UNICODETEXT plus history/monitor opt-outs → masked Alt/Win release → `[Ctrl↓,V↓]` 50 ms `[V↑,Ctrl↑]` → restore if the sequence number is unchanged. Falls back to KEYEVENTF_UNICODE |
| `src/context/` | Truncation, password suppression, app kind and style bucket from `data/app-lists.json` (pure); `uia.rs` = UI Automation snapshot with a 300 ms timeout, and `UiaFieldReader` (re-reads the pasted-into control, `CompareElements` for focus) |
| `src/ducking/` | Mute plan and restore rule (pure); `win.rs` = WASAPI sessions on an MTA thread |
| `src/sounds.rs` | Synthesised cue WAVs (pure) played with `PlaySoundW(SND_MEMORY)` |
| `src/prefs.rs` | `%APPDATA%\FlowClone\prefs.json`: tolerant load, atomic save, validation, `prefs_rev` migration (rev 1, D22: a stored hands-free auto-stop of exactly 2000 ms, the old default, becomes 1500 once; later saves keep any value) |
| `src/core_api.rs` | §9 API seam (shell types) plus `StubCore` for tests |
| `src/core_adapter.rs` | `CoreApi` on `flow_core::FlowCore`: two-phase background load (whisper engine, then the optional `flow_llm` cleaner, composed with `FlowCore::from_engines`), `ModelStatus` / `LlmStatus`, shell→core type mapping |
| `src/history.rs` | Local SQLite history (`history.sqlite`, 10-clone-spec §2 columns, no audio), keep / 24 h / never retention; list/search/get/delete/clear/set_edited for the History tab |
| `src/learn.rs` | Learn-from-edits (03 §3), pure: anchor-bounded region, Wispr end reasons, watch state machine in injected time, semi-global trim of text typed after the dictation |
| `src/suggestions.rs` | Learned words waiting for an explicit accept (`suggestions.json`); dismissed words are never re-suggested |
| `src/dictionary.rs` | Import/export of the shared `[{word, replacement, isSnippet}]` JSON: tolerant parse, validation, case-insensitive merge |
| `src/overlay.rs` | Pill hit-testing so the hands-free ✕/✓ are clickable while the canvas stays click-through |
| `src/hotkeys/watchdog.rs` | Hook-liveness watchdog (probe + re-install after a `LowLevelHooksTimeout` removal) |
| `src/router_adapter.rs` | D17: `CoreApi` on `flow_router::Router` (every build), `LocalBackend` over the `FlowCoreAdapter` slot, remote = `flow_remote::factory` (feature `remote`), router errors → pill text, last-route record (Developer page, S/L badge) |
| `src/dev_settings.rs` | Developer settings: `prefs.json` `"developer"` section, form validation (host/port/pin/token/timeouts, public host ⇒ TLS), mapping to the router's `RouterConfig` JSON (pinned by `crates/flow-router/fixtures`) |
| `src/credentials.rs` | D18 `CredentialStore`: Credential Manager (`flow-clone/cloud/<id>`, `CredWriteW`/`CredReadW`/`CredDeleteW`) on Windows, `MemoryStore` stand-in elsewhere; masking, key validation |
| `src/debug_log.rs` | Opt-in debug log: bounded queue + writer thread, JSONL file sink (last 500), schema, secret scrubbing |
| `src/cloud.rs` | D18 pure helpers: failure texts, engine row, `cloud_usage.json` load/save, usage view, presets, provider ids |
| `tests/cloud_settings.rs` | Developer form JSON → prefs (no keys) → router JSON == `router_config_cloud.json`; consent cleared on host/adapter change |
| `tests/cloud_integration.rs` | Controller → router → flow-cloud (Groq preset) → flow-cloud's mock HTTP provider: cloud paste, usage restore, 500/401 → local fallback with all audio, no key/consent → no request, cloud-only error pills, Test provider; key never shown |
| `src/secrets.rs` | DPAPI (`CryptProtectData`) blobs for secrets in prefs (server token; reusable for later API keys) |
| `tests/router_integration.rs` | Controller → router → flow-remote → in-process flow-server (MockAsr) vs local MockAsr: all four toggle combos, server down / killed mid-dictation (no lost audio), error pills, Test connection / Test session |
| `tests/core_integration.rs` | Controller + real flow-core (MockAsr, and whisper tiny.en on `jfk.wav` when present): audio → commit → paste plan → history |
| `src/app.rs` | Tauri: tray, overlay window (512x614, click-through, `WS_EX_NOACTIVATE`), settings window, commands, autostart |
| `build.rs`, `capabilities/` | Every command is declared in the app manifest; the settings window gets all of them, the overlay only `overlay_action` (`tests/capabilities.rs`) |
| `tools/ci/` | CI helpers: Parakeet DLL bundle overlay, installer collection (`.github/workflows/windows-build.yml`) |
| `ui/` | Plain TypeScript → `ui/dist/*.js` (committed). No bundler, no CDN, no web fonts; the CSP is `'self'` only. Settings tabs: General, AI cleanup, History, Dictionary, Developer |
| `tauri.llm.conf.json` | Config overlay that bundles `flow_llm.dll` next to the exe (`cargo tauri build --config tauri.llm.conf.json`) |
| `tools/probes/settings_mock/` | Serves `ui/dist` with a fake `__TAURI__` for checking the settings page in a plain/headless browser |

## Develop on Linux (what CI/agents can run)

```sh
cargo test                       # pure logic + flow-core integration (builds whisper.cpp)
cargo clippy --all-targets -- -D warnings
rustup target add x86_64-pc-windows-msvc
# Win32 + Tauri code. Check-only shims (never use them for real builds):
# tauri-build always runs a resource compiler (RC), and rusqlite's bundled
# SQLite is archived with lib.exe (AR). whisper-rs-sys (cmake + C++) cannot
# cross-build to MSVC from Linux at all, so the cross-check turns the
# default `whisper` feature off; the Rust side of whisper is covered by the
# host build.
# Each flavour without whisper/sherpa (they can't cross-build): hybrid
# with cloud STT (reqwest + rustls/ring), hybrid with and without TLS,
# client-only and server-only.
for f in app,local-engine,remote,cloud app,local-engine,remote,tls app,local-engine,remote app,local-engine app,remote; do
  RC=$PWD/tools/check-shims/fake-windres AR_x86_64_pc_windows_msvc=$PWD/tools/check-shims/fake-lib \
    cargo clippy --all-targets --target x86_64-pc-windows-msvc --no-default-features --features $f -- -D warnings
done
npm --prefix ui ci && npm --prefix ui run build    # after editing ui/src/*.ts
python3 tools/gen_icons.py                          # after editing the icon
```

The real-model test looks for `ggml-tiny.en.bin` + `jfk.wav` in `../../models/`
(or the main checkout's `models/`, or `FLOW_MODEL` / `FLOW_WAV`) and skips
with a note when they are missing. The real-LLM tests additionally need the
plugin (`cargo build --release -p flow-llm` in the repo root →
`../../target/release/libflow_llm.so`, or `FLOW_LLM_LIB`) and
`qwen2.5-0.5b-instruct-q4_k_m.gguf` (or `FLOW_LLM_MODEL`); building
llama.cpp needs `BINDGEN_EXTRA_CLANG_ARGS` on this box (see
`docs/core-architecture.md`).

Linking a Windows binary on Linux is not supported (no MSVC linker or SDK).

## Live test on Windows (CI installers)

The `windows-build` workflow (`.github/workflows/windows-build.yml`) builds the
real MSVC app on `windows-latest` (on every push to `integration` that touches
`apps/flow-windows/**` or `crates/**`, or by hand: `gh workflow run
windows-build.yml --ref <branch>`), runs `cargo test` for both feature sets
and clippy, and uploads two artifacts, each with an MSI and an NSIS setup:

| Artifact | Flavour | Engines |
|---|---|---|
| `flow-clone-windows-hybrid` | default (`app,parakeet`) | this PC (whisper.cpp + Parakeet) + your flow-server + opt-in cloud STT |
| `flow-clone-windows-client` | `--no-default-features --features local,app,parakeet` | this PC only; no network code |

Both ship `sherpa-onnx-c-api.dll` + `onnxruntime.dll` next to the exe
(Parakeet). They do **not** ship `flow_llm.dll` (AI cleanup reports "engine
missing"; build it as in step 3 below if you want it) and no model files.
The installers are unsigned: SmartScreen says "Windows protected your PC" →
More info → Run anyway.

1. **Download**: `gh run download <run-id> -n flow-clone-windows-hybrid`
   (or the run page → Artifacts), unzip, run `Flow-Clone_0.1.0_x64-setup_hybrid.exe`
   (per-user, no admin) or the `.msi`. Install only one flavour at a time
   (same app id). The tray icon appears; Settings opens on first start.
2. **Model** (only for "this PC"): copy `ggml-base.bin` (or `ggml-tiny.en.bin`
   + set the path in Settings → Speech model) to
   `%APPDATA%\FlowClone\models\`, or the Parakeet folder
   `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` there and pick Engine =
   Parakeet. The tray's first line turns into "Speech model: …".
3. **Server** (hybrid build): on the server, `flow-server gen-token --out
   <file>` and start it bound to its Tailscale/LAN address (see
   `crates/flow-server/dist/config.example.toml`). On Windows: Settings →
   Developer → Host = the server's Tailscale IP or name (no `ws://`, no
   port), Port = 7313, Token = the file's contents (Paste), TLS only if the
   server has `[tls]` (then paste the certificate pin), Save. **Test
   connection** must say `OK · median N ms · flow-server … · parakeet`;
   **Test session** must say `EMPTY (no_speech) via server`.

### 10-minute smoke checklist

- [ ] (1 min) Hold Ctrl+Win in Notepad, say "testing one two three", release → the text is pasted; the line above the pill said "Transcribing via server…" (hybrid, server up) or "Transcribing via this PC…".
- [ ] (1 min) Stop the server (or quit Tailscale), dictate again → still pasted; the line says "Transcribing via this PC… (server unreachable)"; Developer → Active engine row = client, fallback `connect_failed`.
- [ ] (1 min) Start the server again; hold Ctrl+Win, stop the server while speaking, keep talking, release → the whole sentence pastes and the line switched to "this PC" mid-dictation.
- [ ] (1 min) Hands-free: Ctrl+Win+Space, speak, then stay silent → it stops and pastes by itself after ~1.5 s (Settings → General → "Hands-free: stop after … s of silence"; 0 = never). Push-to-talk never stops on silence.
- [ ] (1 min) Esc while the pill is processing → nothing is pasted, no error pill; the next dictation works at once (the server is not left busy).
- [ ] (1 min) Dictate, and while it is processing Alt+Tab to another window → nothing is typed there; the pill says "the focus moved…", Ctrl+V pastes the text. Dictate into a normal field and tab into a password box before it finishes → nothing is pasted or copied.
- [ ] (1 min) Copy something, dictate → afterwards Ctrl+V gives your original clipboard. Press Shift+Alt+Z twice quickly → still your original clipboard afterwards. Shift+Alt+X then Win+V → the transcript is **not** in clipboard history.
- [ ] (1 min) Run `cmd` as administrator, hold Ctrl+Win over it and release → the "runs as administrator" pill; then type `dle` in Notepad: plain letters appear (a stuck Win key would show the desktop / lock the PC), and Start did not open.
- [ ] (1 min) Cloud (optional, costs cents): Developer → Add provider (Groq) → paste key → Save key → Test provider `OK · "And so, my fellow Americans"…`; Allow cloud → accept consent → Save; dictate → the line says "Transcribing via Groq (whisper-large-v3-turbo)…", the pill shows **C**. Change the base URL host, Save → "the API key of Groq was removed … enter the key again".
- [ ] (1 min) Client-only build: Developer tab hidden; dictation works with the model; `netstat -ano | findstr <pid>` shows no outbound connection.

Report failures with `%APPDATA%\FlowClone\prefs.json` (no secrets in it) and the
console output (`"C:\Program Files\Flow Clone\Flow Clone.exe"` started from a
terminal prints `[audio]`, `[hotkeys]`, `[cloud]` lines).

## Build and run on a real Windows 10/11 machine

1. Install the prerequisites:
   - Rust (`rustup`, MSVC toolchain).
   - Visual Studio 2022 Build Tools with "Desktop development with C++" (this gives MSVC, the Windows SDK and `rc.exe`).
   - CMake (whisper.cpp; bundled with the VS C++ workload) and LLVM/clang (`bindgen` in whisper-rs-sys; `winget install LLVM.LLVM`, set `LIBCLANG_PATH`).
   - Node.js 18+.
   - WebView2 Runtime (preinstalled on Windows 11).
2. Build the front-end:
   ```powershell
   cd apps\flow-windows
   npm --prefix ui ci
   npm --prefix ui run build
   ```
3. Build the AI cleanup plugin (optional; skip it and AI cleanup reports "engine missing"):
   ```powershell
   cd ..\..                                  # repo root
   cargo build --release -p flow-llm          # -> target\release\flow_llm.dll
   cd apps\flow-windows
   ```
   llama-cpp-sys-2 needs CMake, clang (`LIBCLANG_PATH`) and MSVC, like whisper. It is built with OpenMP, so `vcomp140.dll` (Visual C++ Redistributable) must be present on the target PC.
4. Run it:
   ```powershell
   cargo run --features app
   copy ..\..\target\release\flow_llm.dll target\debug\   # plugin next to the exe
   ```
   (Or set Settings → AI cleanup → Engine library to the DLL's path.) Optionally install the CLI first (`cargo install tauri-cli --version "^2"`); then `cargo tauri dev`, or for an NSIS installer in `target\release\bundle\nsis\` that ships the plugin next to the exe:
   ```powershell
   cargo tauri build --config tauri.llm.conf.json
   ```
   (Plain `cargo tauri build` builds an installer without AI cleanup.)
5. First launch opens Settings, because there is no `prefs.json` yet.
   - The tray icon lives in the notification area.
   - Copy a model to `%APPDATA%\FlowClone\models\ggml-base.bin` (see above). The tray menu's first line says "Speech model: ggml-base.bin" once it is loaded.
   - Hold Ctrl+Win in Notepad and speak. When you release, the transcript is pasted.

### Manual verification checklist (needs real Windows; not covered by tests)

- [ ] Hold Ctrl+Win: the start sound plays, the pill grows to 73x30, and the bars follow your voice.
- [ ] Releasing pastes into Notepad, Word, Chrome and Slack.
- [ ] Ctrl+Win+Space or a double-tap locks: the 102.5x30 pill shows ✕/✓.
- [ ] Clicking ✓ pastes, clicking ✕ cancels, and neither click moves focus or the caret out of the target app (WebView2 + `WS_EX_NOACTIVATE`); clicks next to the pill still reach the app below.
- [ ] The live transcript appears above the pill while speaking (Settings → "Show live transcript").
- [ ] Shift+Alt+Z pastes the last transcript again (also after a restart, unless history retention is "Never").
- [ ] Without a model file: tray line "⚠ Speech model missing", Settings opens, and Ctrl+Win ends in an error pill naming the path.
- [ ] `%APPDATA%\FlowClone\history.sqlite` gains a row per dictation; retention "Never" empties it.
- [ ] The same chord stops a locked dictation.
- [ ] Esc cancels, and Esc is not delivered to the app while dictating.
- [ ] Releasing Win after a chord does **not** open the Start menu.
- [ ] Ctrl+Win+Left still switches desktops and aborts dictation.
- [ ] After a paste, the old clipboard is back within about 0.5 s (3 s in Outlook/Word/WhatsApp).
- [ ] Win+V history does not contain the dictated text.
- [ ] Dictating into an elevated window (admin cmd) shows the "runs as administrator" error and leaves the text on the clipboard.
- [ ] Hook watchdog: after forcing a hook timeout (e.g. `HKCU\Control Panel\Desktop\LowLevelHooksTimeout` = 1 and a debugger break in `hook_proc`), shortcuts work again within ~2 s of typing ("re-installing" on stderr).
- [ ] Other apps' audio is muted while dictating and restored afterwards.
- [ ] A volume you change mid-dictation is kept.
- [ ] The pill never takes focus, never shows in Alt+Tab, and is click-through.
- [ ] The UIA context reaches the core: check `StubCore::last_context` in a debugger, or log it in `core_adapter`.
- [ ] Unplugging the mic during capture leaves an error pill, not a stuck "listening" pill.
- [ ] "Launch at login" writes `HKCU\…\Run\FlowClone`.
- [ ] Shift+Alt+X puts the last transcript on the clipboard (no paste); Ctrl+V in Notepad shows it.
- [ ] AI cleanup off (default): tray "AI cleanup: off", results unchanged.
- [ ] AI cleanup on without `flow_llm.dll`: tray "⚠ AI cleanup engine (flow_llm.dll) missing", dictation still pastes.
- [ ] AI cleanup on with the DLL + Qwen 0.5B GGUF: tray goes "loading…" → "AI cleanup: qwen2.5-0.5b…"; dictation works during the load; afterwards "um so i think we should uh meet tomorrow" pastes cleaned up, and History shows "AI edited".
- [ ] The NSIS installer built with `--config tauri.llm.conf.json` puts `flow_llm.dll` beside `Flow Clone.exe`, and AI cleanup loads on a PC without Rust/VS (only the VC++ redistributable).
- [ ] Per-app styles: with Personal = Very casual, dictating into WhatsApp gives lowercase without a final period; Email stays formal.
- [ ] History tab: search filters, Copy, Re-paste (Settings hides, text lands in the previously focused app), Delete, Delete all (second click), retention change prunes.
- [ ] Undo AI edit: the diff shows removed/added words; "Re-paste original" inserts the raw ASR text.
- [ ] Learn from edits: dictate "I use whisper flow daily" into Notepad, fix it to "Wispr Flow" within ~45 s, click elsewhere → tray "1 dictionary suggestion…"; Dictionary tab lists it; nothing was added until "Add word". Same in Chrome (textarea), Word and Slack.
- [ ] Learn from edits ignores password fields, and a chat message sent with "press enter".
- [ ] Dictionary editor: add/edit/delete rows, snippet with a multi-line body, Save; export writes `Downloads\flow-dictionary.json`; importing that file (merge) changes nothing; importing the Android app's export works.
- [ ] Dictating into silence shows "No speech heard" above the pill for ~1.5 s, with no error sound.
- [ ] (2 min) Decode while speaking (D22): with the Developer toggle on, dictate ~30 s with pauses → text appears ≈ 0.5-1 s after release; the debug line shows `segments` ≥ 2 and `pre_asr_ms` > 0. Toggle off → `segments` 0 and release → text grows with length again.
- [ ] Settings → General: "Auto-format punctuation, fillers and corrections" off → "comma" stays a word; Engine = Parakeet with the model folder in `models\` (parakeet build) → tray shows the folder name and jfk-style speech pastes.

- [ ] Debug log: Developer → tick "Debug log (includes transcripts)", dictate into Notepad → the count says 1 entry, Open log folder shows `dictations.jsonl` with `"mode":"hold"`, `"insert":{"outcome":"inserted",…}`, `"target":"notepad.exe"`; stop the server and dictate (server-first) → `"fallback":"connect_failed"`; untick → the delete prompt appears, Delete → the file is gone; dictate again → no file.

#### Developer settings / hybrid mode (needs a `flow-server` on the LAN or Tailscale)

- [ ] Hybrid build: Developer tab is visible; Build row says "client + server"; header line no longer says "nothing is sent over the network".
- [ ] Host field: `ws://1.2.3.4`, `1.2.3.4:7313`, `256.1.1.1`, `my box` each show an inline error and disable Save / Test connection; `100.64.0.10`, `[fd7a:115c:a1e0::1]`, `box.tailnet.ts.net` are accepted (brackets dropped on save). Port 0 / 70000 rejected.
- [ ] Host `example.com` ticks and locks "Use TLS"; a 100.x / 192.168.x host shows "Unencrypted: OK on Tailscale…".
- [ ] Token: Show / Paste / Clear work; a 10-char token is rejected; after Save, `prefs.json` contains `token_dpapi` and **not** the token text; copying `prefs.json` to another Windows account leaves the token empty there.
- [ ] Test connection with the server up: "OK · median N ms (a-b) · flow-server 0.1.0 · parakeet"; with a wrong token: "auth_failed … token rejected"; with the server stopped: "connect_failed … is flow-server running / Tailscale up?".
- [ ] Test session: "EMPTY (no_speech) via server"; with the server stopped: "via client (fallback: connect_failed)".
- [ ] Both toggles on, server up: dictate in Notepad → Active engine row = server, rtt shown; with the S/L badge on, the pill shows **S**.
- [ ] `systemctl stop flow-server` (or quit Tailscale), dictate again → text still pastes (local), row = client, fallback `connect_failed`, badge **L** with a red dot; the next dictation within 5 s shows `circuit_open` and does not wait for the connect timeout; "Reset circuit" clears it.
- [ ] Kill the server mid-dictation (hold Ctrl+Win, stop it, keep speaking, release): the whole utterance pastes, nothing from before the kill is lost.
- [ ] Allow client off + server down → error pill "Server unreachable (…)"; Allow server off → dictation never opens a connection (check with `netstat -n` / Resource Monitor); both off → "Dictation disabled — enable server or client in Developer settings".
- [ ] Changing toggles / host and pressing Save takes effect on the next dictation without restarting; the local model is not reloaded (tray line unchanged).
- [ ] Route order "Move up" (client first) with both on → dictation runs on this PC even with the server up.
- [ ] Client-only build (`--no-default-features --features local,app`): Developer tab hidden; 7 taps on the version line unlock it ("not in this build" next to Allow server); Lock hides it; `flow-windows.exe` has no ws/tls code (`cargo tree -e normal --no-default-features --features local,app | findstr tungstenite` prints nothing).
- [ ] Server-only build (`--no-default-features --features remote,app`): starts without any model file or `flow_llm.dll`, tray says "Speech: your flow-server"; dictation works through the server; Allow client is disabled ("not in this build").

#### Cloud providers (needs internet and a real provider key)

- [ ] Default build: Developer tab shows "Cloud providers (sends audio to a third party)"; the client-only build does not, and its exe has no reqwest/rustls (`cargo tree … --features local,app | findstr reqwest` prints nothing).
- [ ] Add OpenRouter, paste a key, Save key: Credential Manager (Control Panel → Credential Manager → Windows Credentials) lists `flow-clone/cloud/openrouter-1`; `prefs.json` contains no key text; reopening Settings shows `Key set: ••••abcd` only.
- [ ] Test provider with the key: `OK · "And so, my fellow Americans" · N ms · $…`; with a wrong key: `auth_failed — key rejected`; with the network off: `connect_failed — no internet / base URL wrong`.
- [ ] Allow cloud → consent dialog "Send audio to OpenRouter?"; Cancel unticks Allow cloud; accept, Save, dictate in Notepad → text pastes, pill shows **C** (tooltip "Sent to OpenRouter"), Active engine row `cloud · OpenRouter · openai/gpt-transcribe · $0.000…`, usage row +1.
- [ ] Change the custom base URL host and Save → the dialog asks again; Cancel → no request leaves the PC (Resource Monitor / proxy).
- [ ] Clear key (or revoke it at the provider) → next dictation still pastes (local), bubble "Cloud STT failed (key rejected); used this PC"; the following dictation does not contact the provider until a new key is saved.
- [ ] Monthly budget below this month's cost → "Cloud STT failed (monthly budget reached); used this PC".
- [ ] Quit and restart: usage counters are unchanged (`%APPDATA%\FlowClone\cloud_usage.json`); keys still set.
- [ ] Repeat a dictation with Groq, ElevenLabs, Deepgram, AssemblyAI and a custom OpenAI-compatible server (speaches/LocalAI on the LAN over `http://`).

## Known gaps / v1 limitations

- Cloud (D18): the **C** badge appears after the commit (like S/L); while recording the line above the pill names the provider. A key saved for a provider that is never saved stays in Credential Manager (bound to the host it was typed for) until "Clear key" or the provider is deleted. HTTPS uses webpki roots, so a corporate TLS-inspection CA in the Windows store is not trusted.
- Hands-free auto-stop (D19) applies to dictations *started* hands-free (Ctrl+Win+Space). A push-to-talk dictation locked later (double tap, or Space added to the chord) has no endpoint, because the router fixes `endpoint_silence_ms` at session start; stop it with the key or ✓.
- Paste target (S13): the guard compares the foreground window (HWND + process) with the one at dictation start; switching tabs *inside* the same window (browser tabs) is not detected. The password-field re-check needs UI Automation to answer within 300 ms; otherwise only the window check applies.
- Clipboard privacy formats (S14) are honoured by Windows' own history/cloud sync; third-party clipboard managers that ignore `ExcludeClipboardContentFromMonitorProcessing` still see the text.

- Hands-free ✕/✓ use a 30 ms cursor poll that toggles click-through over the pill (not `setShape`). A click in the first ~30 ms after the cursor arrives can fall through.
- There is no delayed clipboard rendering (`WM_RENDERFORMAT`). The immediate path is used with a sequence-number guard, as the RE checklist recommends.
- RTF and file-drop clipboard contents **are** preserved, unlike Wispr, on purpose: the RE checklist calls Wispr's behaviour lossy.
- The UIPI pre-check compares token integrity levels only. UIAccess targets and protected processes (level unreadable) fall through to the normal paste.
- The hook watchdog probes only while the user is active and at most every 5 s, so a dead hook is noticed after the next input, not instantly.
- There is no mouse-button shortcut (`WH_MOUSE_LL`) and no Caps Lock single-key shortcut.
- Learn-from-edits polls (default every 1 s for 45 s) instead of subscribing to UIA `TextChanged` events, and uses no local LLM to vet candidates ("is this a proper noun?", 03 §6): flow-core's alignment filters plus the user's explicit accept do that job. Controls without TextPattern/ValuePattern (some Electron/canvas editors) are not watched.
- LLM cleanup load is 10-50 s on CPU and quality with the default 0.5B model is 4/8 on the core's messy-dictation set (1.5B: 8/8, ~2x slower); see `docs/core-architecture.md`.
- "Re-paste" from the History tab hides Settings and pastes into whatever window Windows activates next (normally the one used before Settings).
- Cue sounds are synthesised tones, not Wispr's WAVs (those are not redistributable).

## flow-core integration

`Cargo.toml` takes `flow-core = { path = "../../crates/flow-core" }`; the crate
stays a standalone workspace (Tauri/Win32 deps stay out of the core lockfile).
`src/core_adapter.rs` maps the shell's §9 types onto flow-core's:

* `SessionConfig`: the ordered `[{word, replacement, isSnippet}]` list, languages, styles (strings → `Style`), cleanup level, raw mode, `max_samples` from the 20-min policy, `platform: windows`, `partial_interval_ms` from the live-preview pref.
* `FieldContext`: exe stem as `app_id`, URL host, `AppKind`/`StyleBucket` → `AppCategory`, before/selected/after.
* `CommitResult`: `FORMATTED`/`RAW_TRANSCRIPT` paste the core's `segments` plan (text runs, Tab/Break, trailing Enter for spoken "press enter"; `press_enter` itself is not read), history keeps `plain_text`; `EMPTY` pastes nothing (`empty_reason: no_speech` shows "No speech heard"), `ERROR` shows the error pill; `route` (router) says which engine ran.

Requests for core changes go to `docs/core-change-requests.md` (the core is
developed on another branch).
