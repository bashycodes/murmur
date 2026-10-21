# flow-windows — Windows shell of the local Wispr Flow clone

Tauri 2 tray app: hold **Ctrl+Win** to dictate, **Ctrl+Win+Space** (or double-tap
Ctrl+Win) for hands-free, **Esc** to cancel, **Shift+Alt+Z** to paste the last
transcript again, **Shift+Alt+X** to copy it. Text is pasted into the focused app.
Speech-to-text runs either on this PC (the real `crates/flow-core`: whisper.cpp
or Parakeet) or on **your own** `flow-server` over LAN/Tailscale (hybrid mode,
`docs/hybrid-architecture.md`, D17). Every dictation goes through
`flow_router::Router` (`src/router_adapter.rs`); nothing reaches a third party,
and a client-only build contains no network code at all.

## Build flavours (Cargo features)

| Build | Command | Engines |
|---|---|---|
| Hybrid (default) | `cargo build --features app` | this PC + your server, fallback between them |
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
* Saving applies to the next dictation (no restart, no model reload).

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
| `src/prefs.rs` | `%APPDATA%\FlowClone\prefs.json`: tolerant load, atomic save, validation |
| `src/core_api.rs` | §9 API seam (shell types) plus `StubCore` for tests |
| `src/core_adapter.rs` | `CoreApi` on `flow_core::FlowCore`: two-phase background load (whisper engine, then the optional `flow_llm` cleaner, composed with `FlowCore::from_engines`), `ModelStatus` / `LlmStatus`, shell→core type mapping |
| `src/history.rs` | Local SQLite history (`history.sqlite`, 10-clone-spec §2 columns, no audio), keep / 24 h / never retention; list/search/get/delete/clear/set_edited for the History tab |
| `src/learn.rs` | Learn-from-edits (03 §3), pure: anchor-bounded region, Wispr end reasons, watch state machine in injected time, semi-global trim of text typed after the dictation |
| `src/suggestions.rs` | Learned words waiting for an explicit accept (`suggestions.json`); dismissed words are never re-suggested |
| `src/dictionary.rs` | Import/export of the shared `[{word, replacement, isSnippet}]` JSON: tolerant parse, validation, case-insensitive merge |
| `src/overlay.rs` | Pill hit-testing so the hands-free ✕/✓ are clickable while the canvas stays click-through |
| `src/hotkeys/watchdog.rs` | Hook-liveness watchdog (probe + re-install after a `LowLevelHooksTimeout` removal) |
| `tests/core_integration.rs` | Controller + real flow-core (MockAsr, and whisper tiny.en on `jfk.wav` when present): audio → commit → paste plan → history |
| `src/app.rs` | Tauri: tray, overlay window (512x614, click-through, `WS_EX_NOACTIVATE`), settings window, commands, autostart |
| `ui/` | Plain TypeScript → `ui/dist/*.js` (committed). No bundler, no CDN, no web fonts; the CSP is `'self'` only. Settings tabs: General, AI cleanup, History, Dictionary |
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
# Each flavour without whisper/sherpa (they can't cross-build): hybrid,
# client-only and server-only.
for f in app,local-engine,remote app,local-engine app,remote; do
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
- [ ] Settings → General: "Auto-format punctuation, fillers and corrections" off → "comma" stays a word; Engine = Parakeet with the model folder in `models\` (parakeet build) → tray shows the folder name and jfk-style speech pastes.

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

## Known gaps / v1 limitations

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
* `CommitResult`: `FORMATTED`/`RAW_TRANSCRIPT` paste `final_text` (then Enter when `press_enter`), `EMPTY` pastes nothing, `ERROR` shows the error pill.

Requests for core changes go to `docs/core-change-requests.md` (the core is
developed on another branch).
