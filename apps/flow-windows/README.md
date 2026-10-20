# flow-windows — Windows shell of the local Wispr Flow clone

Tauri 2 tray app: hold **Ctrl+Win** to dictate, **Ctrl+Win+Space** (or double-tap
Ctrl+Win) for hands-free, **Esc** to cancel. Text is pasted into the focused app.
Nothing touches the network. Speech-to-text comes from `flow-core` through the
`CoreApi` trait. Until `flow-core` is wired in, a stub core pastes
`[stub: N.N s of audio]`.

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
| `src/core_api.rs` | §9 API seam plus `StubCore` |
| `src/core_adapter.rs` | **The one file to change** to plug in `flow-core` |
| `src/app.rs` | Tauri: tray, overlay window (512x614, click-through, `WS_EX_NOACTIVATE`), settings window, commands, autostart |
| `ui/` | Plain TypeScript → `ui/dist/*.js` (committed). No bundler, no CDN, no web fonts; the CSP is `'self'` only |

## Develop on Linux (what CI/agents can run)

```sh
cargo test                       # all pure logic (no Windows needed)
cargo clippy --all-targets
rustup target add x86_64-pc-windows-msvc
cargo clippy --all-targets --target x86_64-pc-windows-msvc          # Win32 code
# The Tauri shell too. tauri-build always runs a resource compiler, so
# point RC at the check-only shim (never use it for real builds):
RC=$PWD/tools/check-shims/fake-windres \
  cargo clippy --all-targets --target x86_64-pc-windows-msvc --features app
npm --prefix ui ci && npm --prefix ui run build    # after editing ui/src/*.ts
python3 tools/gen_icons.py                          # after editing the icon
```

Linking a Windows binary on Linux is not supported (no MSVC linker or SDK).

## Build and run on a real Windows 10/11 machine

1. Install the prerequisites:
   - Rust (`rustup`, MSVC toolchain).
   - Visual Studio 2022 Build Tools with "Desktop development with C++" (this gives MSVC, the Windows SDK and `rc.exe`).
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
   - Hold Ctrl+Win in Notepad and speak. When you release, the stub text is pasted.

### Manual verification checklist (needs real Windows; not covered by tests)

- [ ] Hold Ctrl+Win: the start sound plays, the pill grows to 73x30, and the bars follow your voice.
- [ ] Releasing pastes into Notepad, Word, Chrome and Slack.
- [ ] Ctrl+Win+Space or a double-tap locks: the 102.5x30 pill shows ✕/✓.
- [ ] The same chord stops a locked dictation.
- [ ] Esc cancels, and Esc is not delivered to the app while dictating.
- [ ] Releasing Win after a chord does **not** open the Start menu.
- [ ] Ctrl+Win+Left still switches desktops and aborts dictation.
- [ ] After a paste, the old clipboard is back within about 0.5 s (3 s in Outlook/Word/WhatsApp).
- [ ] Win+V history does not contain the dictated text.
- [ ] Dictating into an elevated window (admin cmd) reports an error instead of hanging.
- [ ] Other apps' audio is muted while dictating and restored afterwards.
- [ ] A volume you change mid-dictation is kept.
- [ ] The pill never takes focus, never shows in Alt+Tab, and is click-through.
- [ ] The UIA context reaches the core: check `StubCore::last_context` in a debugger, or log it in `core_adapter`.
- [ ] Unplugging the mic during capture leaves an error pill, not a stuck "listening" pill.
- [ ] "Launch at login" writes `HKCU\…\Run\FlowClone`.

## Known gaps / v1 limitations

- The overlay window is fully click-through, so the ✕/✓ buttons in hands-free mode are visual only. Wispr uses `setShape` / alpha hit-testing.
- There is no delayed clipboard rendering (`WM_RENDERFORMAT`). The immediate path is used with a sequence-number guard, as the RE checklist recommends.
- RTF and file-drop clipboard contents **are** preserved, unlike Wispr, on purpose: the RE checklist calls Wispr's behaviour lossy.
- There is no UIPI pre-check (integrity-level comparison). A short `SendInput` is reported as a paste error.
- Hook timeout recovery (re-installing after Windows silently drops the hook) is not implemented. Only the stale-key probe exists.
- There is no mouse-button shortcut (`WH_MOUSE_LL`), no Caps Lock single-key shortcut, and no "paste last transcript".
- Cue sounds are synthesised tones, not Wispr's WAVs (those are not redistributable).

## flow-core integration

Edit `src/core_adapter.rs` only:

1. Add `flow-core = { path = "../../crates/flow-core" }` to `Cargo.toml`. Either drop the `[workspace]` table if the crate joins a root workspace, or keep the crate standalone.
2. Implement `CoreApi` for the flow-core handle. The methods map 1:1 to spec §9.
3. Return it from `make_core()`, loading models from `models_dir(%LOCALAPPDATA%)`.
