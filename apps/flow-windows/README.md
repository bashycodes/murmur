# flow-windows — Windows shell of the local Wispr Flow clone

Tauri 2 tray app: hold **Ctrl+Win** to dictate, **Ctrl+Win+Space** (or double-tap
Ctrl+Win) for hands-free, **Esc** to cancel, **Shift+Alt+Z** to paste the last
transcript again. Text is pasted into the focused app. Nothing touches the
network. Speech-to-text is the real `crates/flow-core` (whisper.cpp), reached
through the `CoreApi` trait in `src/core_adapter.rs`.

## Speech model (never downloaded automatically)

Put a whisper.cpp ggml model at `%APPDATA%\FlowClone\models\ggml-base.bin`
(the default: multilingual, CPU tier per `docs/DECISIONS.md` D4), or choose
any ggml file in Settings → Speech model (`model_path` in `prefs.json`;
`ggml-small-q5_1.bin` is the accuracy step up). The repo's
`tools/fetch-models.sh` downloads models on explicit request only.
The model loads in the background at startup. A missing or broken file
shows in the tray menu and tooltip, opens Settings, and makes a dictation end
in an error pill that names the path.

## Layout

| Path | What |
|---|---|
| `src/hotkeys/chord.rs` | Pure chord matcher: PTT / hands-free / cancel, Win/Alt release masking, foreign-key abort |
| `src/hotkeys/win.rs` | `WH_KEYBOARD_LL` hook thread. It skips our own `dwExtraInfo`-tagged input, does masked Win/Alt releases (`[LCtrl↓, key↑, LCtrl↑]`) and runs a 1 s stale-key probe |
| `src/dictation.rs` | Pure state machine `idle → listening(ptt\|locked) → processing → pasted\|error → idle`. Wispr's timings: 500 ms tap / double-tap lock / triple-press, warning at 19 min, hard stop at 20 min, 35 s processing timeout, 500 ms error |
| `src/controller.rs` | Runs the machine's effects against a `Platform` trait plus `CoreApi`; integration-tested with fakes |
| `src/audio/` | Resampler to 16 kHz mono i16 and a Wispr-style level meter (pure); `capture.rs` = cpal/WASAPI |
| `src/paste/` | Pure strategy, restore guard and per-app restore delay; `win.rs` = clipboard snapshot → CF_UNICODETEXT plus history/monitor opt-outs → masked Alt/Win release → `[Ctrl↓,V↓]` 50 ms `[V↑,Ctrl↑]` → restore if the sequence number is unchanged. Falls back to KEYEVENTF_UNICODE |
| `src/context/` | Truncation, password suppression, app kind and style bucket from `data/app-lists.json` (pure); `uia.rs` = UI Automation snapshot with a 300 ms timeout |
| `src/ducking/` | Mute plan and restore rule (pure); `win.rs` = WASAPI sessions on an MTA thread |
| `src/sounds.rs` | Synthesised cue WAVs (pure) played with `PlaySoundW(SND_MEMORY)` |
| `src/prefs.rs` | `%APPDATA%\FlowClone\prefs.json`: tolerant load, atomic save, validation |
| `src/core_api.rs` | §9 API seam (shell types) plus `StubCore` for tests |
| `src/core_adapter.rs` | `CoreApi` on `flow_core::FlowCore`: background model load, `ModelStatus` (loading/ready/missing/failed), shell→core type mapping |
| `src/history.rs` | Local SQLite history (`history.sqlite`, 10-clone-spec §2 columns, no audio), keep / 24 h / never retention |
| `src/overlay.rs` | Pill hit-testing so the hands-free ✕/✓ are clickable while the canvas stays click-through |
| `src/hotkeys/watchdog.rs` | Hook-liveness watchdog (probe + re-install after a `LowLevelHooksTimeout` removal) |
| `tests/core_integration.rs` | Controller + real flow-core (MockAsr, and whisper tiny.en on `jfk.wav` when present): audio → commit → paste plan → history |
| `src/app.rs` | Tauri: tray, overlay window (512x614, click-through, `WS_EX_NOACTIVATE`), settings window, commands, autostart |
| `ui/` | Plain TypeScript → `ui/dist/*.js` (committed). No bundler, no CDN, no web fonts; the CSP is `'self'` only |

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
RC=$PWD/tools/check-shims/fake-windres AR_x86_64_pc_windows_msvc=$PWD/tools/check-shims/fake-lib \
  cargo clippy --all-targets --target x86_64-pc-windows-msvc --no-default-features --features app -- -D warnings
npm --prefix ui ci && npm --prefix ui run build    # after editing ui/src/*.ts
python3 tools/gen_icons.py                          # after editing the icon
```

The real-model test looks for `ggml-tiny.en.bin` + `jfk.wav` in `../../models/`
(or the main checkout's `models/`, or `FLOW_MODEL` / `FLOW_WAV`) and skips
with a note when they are missing.

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
3. Run it:
   ```powershell
   cargo run --features app
   ```
   Optionally install the CLI first (`cargo install tauri-cli --version "^2"`); then `cargo tauri dev` or `cargo tauri build` produces an NSIS installer in `target\release\bundle\nsis\`.
4. First launch opens Settings, because there is no `prefs.json` yet.
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

## Known gaps / v1 limitations

- Hands-free ✕/✓ use a 30 ms cursor poll that toggles click-through over the pill (not `setShape`). A click in the first ~30 ms after the cursor arrives can fall through.
- There is no delayed clipboard rendering (`WM_RENDERFORMAT`). The immediate path is used with a sequence-number guard, as the RE checklist recommends.
- RTF and file-drop clipboard contents **are** preserved, unlike Wispr, on purpose: the RE checklist calls Wispr's behaviour lossy.
- The UIPI pre-check compares token integrity levels only. UIAccess targets and protected processes (level unreadable) fall through to the normal paste.
- The hook watchdog probes only while the user is active and at most every 5 s, so a dead hook is noticed after the next input, not instantly.
- There is no mouse-button shortcut (`WH_MOUSE_LL`) and no Caps Lock single-key shortcut. There is no "copy last transcript" (Wispr Shift+Alt+X) and no history UI yet.
- History rows have `edited = NULL`: learn-from-edits (re-reading the field after the paste) is not implemented.
- LLM cleanup is off: flow-core's llama.cpp backend is not wired yet, so results are `RAW_TRANSCRIPT` (deterministic post-processing only).
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
