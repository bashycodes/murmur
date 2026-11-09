# Air-gap readiness, 2026-11-04

The owner asked whether Murmur is ready to move to a network with no
internet, which features are still worth waiting for, what the transfer
steps are, and which models to download now. Code read on
`integration` at `df49069` (CI green: windows-build, android-build,
repo-checks). Owner guide updated in the same change: `docs/air-gapped.md`
("Transfer checklist", "Download the models", "Also bring", server config) and the model
list `docs/air-gapped-models.txt`.

## Verdict

**Ready after one release cut from `integration`** (and the model download).
No open ticket blocks the move. v1.0.0 works offline, but `integration`
has 96 commits since then with fixes that matter on a closed network
(below), and the new model list relies on two of them.

## Models

15 models, 27 files, 6.98 GB (6 983 398 636 bytes). Every URL and SHA-256
in `docs/air-gapped-models.txt` is the Windows catalogue's
(`apps/murmur-windows/src/model_download.rs` `CATALOG` + `LID_FILES`),
checked by script against the source; Android's `ServerCatalog` pins the
same hashes (`ServerCatalogTest`), and the ids are
`murmur_protocol::library::CATALOGUE`'s. On 2026-11-04 each of the 27 links
answered (HTTP 200) with the pinned size, checked without downloading
the files.

Not pinned in code (listed in the guide as optional or "bring"):

| Item | Where named | Note |
|---|---|---|
| Silero VAD `ggml-silero-v5.1.2.bin` (0.9 MB) | `crates/murmur-core/src/config.rs:362` (example name only) | Optional `vad_model_path`; the default energy VAD needs no model (`crates/murmur-core/src/vad.rs:1`). No URL or hash in code. |
| sherpa-onnx 1.13.8 Linux libs (9.8 MB) | `crates/murmur-asr-sherpa/Cargo.toml:20` (version) | URL from `sherpa-onnx-sys` 1.13.8 `build.rs`; no hash. Host install only. |
| Android "Recommended" links | `apps/murmur-android/app/src/main/java/dev/murmur/data/ModelPages.kt:15-16`, `EngineLogic.kt:24,29` | `resolve/main` and a GitHub tarball, not pinned; the app never downloads them (D9). Same files as the pinned list. |
| `tools/fetch-models.sh` (all but `--hebrew`) | `tools/fetch-models.sh:80-106` | `main` / tarball, no hash. |
| WebView2 offline installer (215 MB) | not in code | Tauri's default `downloadBootstrapper` (no `webviewInstallMode` in `apps/murmur-windows/tauri.conf.json:44`) needs internet on a Windows 10 PC without WebView2. |

## What still reaches for the internet

Nothing runs by itself. Every path below is opt-in or a user action, and
each fails without breaking dictation.

| Path | Trigger | Offline behaviour | Where |
|---|---|---|---|
| OpenRouter cleanup / speech (Windows, Android hybrid) | Off by default; needs a key and per-purpose consent | Connect error is "fast unavailable": the router moves to the next engine in the order | `crates/murmur-cloud/src/http.rs:97`, `crates/murmur-router/src/cloud.rs:306`, `apps/murmur-windows/src/openrouter.rs:176` |
| OpenRouter model catalogue (Windows) | Opening Settings → OpenRouter, or a cloud use switched on | ≤ 10 s, then the baked snapshot / 24 h cache; error code shown | `apps/murmur-windows/src/app.rs:2342`, `app.rs:3025`, `crates/murmur-cloud/src/catalog.rs:14` |
| OpenRouter key info (Windows) | OpenRouter page with a key stored | Error code shown | `apps/murmur-windows/src/app.rs:2446` |
| Hugging Face model download (Windows) | Download pressed **and no server set**; with a server set it never contacts HF (D41) | Download fails with a message; nothing kept | `apps/murmur-windows/src/model_download.rs:953`, `:667`; `server_library.rs:42` |
| Links (GitHub, Hugging Face, OpenRouter) | A click | Opens the browser, which fails | `apps/murmur-windows/src/shell.rs:159`, Android `data/ShellPages.kt:22` |
| Gemini Nano / ML Kit GenAI probe (Android) | **hybrid debug builds only**, Developer settings → English speech probe | Probe reports its error; dictation unchanged. Note: the release APKs are debug builds, so the next hybrid APK carries the ML Kit library (added after 1.0.0); `local` and `remote` do not | `apps/murmur-android/app/build.gradle.kts:150`, `ui/DeveloperActivity.kt:189` |
| WebView2 bootstrapper (Windows installer) | Install on a PC without WebView2 | Install fails → bring the offline installer | `apps/murmur-windows/tauri.conf.json:44` |

Checked and absent: telemetry, crash reporting, update checks, NTP-like
calls (no matches in `apps/` or `crates/`); Google Play services, Firebase
and the platform speech recognizer outside the debug probe (the only
Android dependency besides coroutines is the ML Kit line above); fonts or
CDNs in the Windows UI (fonts are bundled, `ui/dist/theme.css:8,15`; CSP
`default-src 'self'`, `connect-src ipc:` at `tauri.conf.json:13`; the only
external strings in `ui/dist` are click-to-open links and a placeholder).
murmur-server makes no outbound connections (`docs/server-setup.md:6`; the
strace audit in `2026-11-01-airgap-e2e.md`). The apps' server link
(`crates/murmur-remote/src/conn.rs:73`) goes only to the configured server.

## v1.0.0 or a new release?

**Cut a new release before the move: yes.** Material changes in
`CHANGELOG.md` [Unreleased]:

* **Windows whisper on the CPU 6-18× faster** (1.0.0 built whisper.cpp with
  `/Od`). On a PC without a GPU every whisper model, Hebrew included, is
  slow in 1.0.0.
* **Server library: `catalogue_id` alone is enough** (engine filled in),
  and the **catalogue gains Parakeet unified and the four q5_1 whisper
  files**. 1.0.0 apps don't know those five ids, so the phone's default
  English model can't be served as a checked catalogue copy.
* **Windows AI cleanup order now applies to dictations made on the PC**: in
  1.0.0 the server never cleaned a local dictation when the PC's own AI
  model was off.
* **AI cleanup always runs on the server** (no "already clean" skip).
* **Quiet speech no longer dropped**; Hebrew kept in Command Mode; Android
  bubble no longer stuck on red during model load.
* **Server logs every connection's end** with a reason, the main way to
  debug the closed network without remote help; **keys show Last used /
  Last connected**.
* New features that work offline: Notetaker (Windows meetings, detection
  off by default), Calls auto-transcribe (Android), meeting route order.
* Version labels no longer end in `-dirty`.

The release also puts `docs/air-gapped-models.txt` into the source zip.

## Open tickets

| Ticket | Matters air-gapped? | Wait? |
|---|---|---|
| English 1: Gemini Nano spike | No: needs Google services and a model download | No |
| Calls 2: auto-transcribe (owner checks pending) | Works offline (on the phone) | No; the owner can check it there |
| Notetaker 3: Zoom / Jabber detection (needs a real call) | Only if meetings run on the closed network; detection is off by default, manual recording works | No, unless auto-detect is wanted there: a fix would need another transfer |
| Notetaker 4: AI summary (not started) | Would run on the server or PC model | No, unless summaries are needed soon |
| 1.0.0 #8: guide + e2e | Guide merged; e2e findings and `PROJECT_CONTEXT.md` row present. The real-device checklist ("internet blocked") is not run yet | No: it is step 11 of the transfer checklist, run on the closed network itself |

## Owner steps

The numbered "Transfer checklist" at the top of `docs/air-gapped.md`.
