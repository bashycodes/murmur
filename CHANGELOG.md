# Changelog

All notable changes to Flow Clone (Windows app, Android app, flow-server and
the shared Rust crates) are recorded here. One product version covers the
whole repo; see `docs/versioning.md` (decision D26). The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/) (pre-1.0: a MINOR bump may break).

Write notes under `[Unreleased]` as changes land; `tools/release.sh X.Y.Z`
turns that section into the dated release.

Builds made before the first tag report `0.1.0` (never tagged), plus the
commit (`0.1.0+<sha>`) once versioning landed. The first tagged release is
`0.2.0` and covers everything from the start of the project
(2026-10-17) to that tag.

## [Unreleased]

### Added

- **Android: Scratchpad.** *Main screen → More → Scratchpad* (or long-press
  the app icon → *Scratchpad*) opens one big text field to dictate into,
  edit and copy out: tap it, tap the bubble and talk, like in any other app.
  It is the only screen of Flow Clone's own where the bubble appears. The
  text saves itself as you go (and when you leave the screen) and is there
  again after the app is closed or the phone restarts; it stays on the phone.
  *Copy all* (removed from the clipboard after a minute, like every copied
  transcript), *Share*, *Clear* (asks first) and a word count.
- **Android: learn dictionary words from your edits.** After a dictation is
  typed in, the app re-reads that same field (about once a second, for up
  to 45 s, or until your edit has stopped changing for 8 s). If you corrected
  a word, say "flo clone" → "Flow Clone", it shows up under *Dictionary →
  Suggestions* with *Add* (or *Add as fix*, which replaces "flo clone" from
  then on) and *Dismiss*; a dismissed word is never suggested again, and
  nothing is added without your tap. Watching stops when you move to another
  field or app, start a new dictation, or press Enter in the dictation.
  Password and other sensitive fields and banking apps are never re-read.
  *Settings → Privacy → Learn from my edits* (on by default, like Windows)
  turns it off; then the field is not read again after inserting.
- **Windows: custom cue sounds.** Put `start.wav`, `stop.wav`, `lock.wav`,
  `paste.wav` or `error.wav` in `%APPDATA%\FlowClone\sounds\` to replace
  that cue's built-in tone (read at the first cue, so restart after changing
  them). A file that isn't a RIFF/WAVE or is over 2 MB is ignored with a
  `[sounds]` log line; the Sounds setting still turns every cue off.
- **Windows: push-to-talk on Right Alt alone.** Shortcuts can name one side
  of a modifier: `Right Alt` / `Left Alt` (also `RAlt`, `AltGr`), `Left Ctrl`
  / `Right Ctrl`, `Left Shift` / `Right Shift`. Plain `Ctrl`, `Alt` and
  `Shift` still match either side, so existing shortcuts are unchanged. With
  push-to-talk = `Right Alt`, Left Alt and Alt+Tab don't start dictation,
  the release doesn't focus the menu bar, and on AltGr keyboard layouts the
  fake Left Ctrl Windows sends with Right Alt is ignored (it also no longer
  makes AltGr fire a Ctrl+Alt shortcut) and is never left stuck.
- **Debug: replay harness.** `tools/replay/run.sh --in DIR` re-runs the
  phone's saved dictation recordings on the PC through each speech model
  (Parakeet, whisper, OpenRouter cloud STT), cleanup model (fallback rules,
  local Qwen, OpenRouter models) and prompt version (v4c, v2) on the real
  pipeline, and prints a score table against the expected text and the
  phone's own results, plus the OpenRouter spend (capped, default $0.05).
  Recordings and results stay outside the repo. See `tools/replay/README.md`.
- **Debug log: why an AI cleanup answer was not used.** When AI cleanup's
  answer is rejected, dropped, times out or fails, the Android and Windows
  debug-log line now has `rejected_text` (the model's answer, if any) and
  `guard_detail` (the guard's reason: the unsupported sentence or literal
  mark kind, the dropped clause or word count, the timeout or the error).
  The Android log viewer shows them as "AI answer not used". Both follow the
  `text` rules: only with the debug log on, and `<redacted>` for password
  fields, banking and denylisted apps. flow-core's `CommitResult` and
  `RouteInfo` carry them as additive optional fields, set only next to
  `cloud_fallback`, so results without cloud cleanup are unchanged.
- **Android debug: one-command pull.** `tools/phone/pull-debug.sh` copies
  the phone's debug log, the kept dictation WAVs (only new ones,
  sha256-checked) and the two settings files with every token, key,
  secret, password and auth value removed into a temp directory on the PC,
  and prints how many lines, WAVs and bytes came over. Read-only on the
  phone; needs a debug build. See INSTALL.md, "Keep last dictation audio".
- **Android debug: a WAV file as the microphone.** `tools/phone/play-wav.sh
  hello.wav` pushes a 16 kHz mono WAV to a debug build and runs one normal
  dictation on it instead of the mic (route order, AI cleanup, insert guard,
  insertion into the focused field), in real time or with `--fast`, so a
  dictation can be tested on the phone without anyone speaking. The debug
  log line says `"source":"wav"` (`"mic"` otherwise). Protected fields are
  refused; only the platform tools' shell can trigger it, and release builds
  do not contain it. See INSTALL.md, "Feed a WAV file as the microphone".
- **flow-server debug capture (beta, off by default).** A new `[debug]`
  config section (`capture = true`, `max_files = 30`, `max_age_h = 72`)
  makes the server keep each session's received audio as a WAV plus one
  JSON line (transcript, cleaned text, timings, route) in its state
  directory, laid out like the phone's debug folder so server-route
  dictations can be replayed too. Mode 0600, pruned on every save and at
  start, never the token or any key, nothing uploaded. `check-config`
  validates it. See docs/server-setup.md §9.

### Changed

- **Android: Style per app is an app list.** Settings → Writing style →
  Style per app no longer asks for typed `package = group` lines. Each
  override is a row with the app's icon and name, a group dropdown
  (Personal / Work / Email / Other) and a remove button; "Add app" opens a
  searchable list of the apps on the phone that have a launcher icon, with
  the ones you dictated into recently first, and then asks for the group
  (the app's built-in one preselected). Changes apply at once, no Save.
  Overrides typed before load as rows (same stored format); an app that is
  no longer installed shows its package name and can still be removed. The
  app now declares a launcher-intent `<queries>` entry to see that list (not
  `QUERY_ALL_PACKAGES`), so an override for an app without a launcher icon
  also shows as its package name.
- **Android: more debug audio kept.** Developer settings → "Keep last
  dictation audio" now keeps the last 30 dictations (was 10), each for
  72 hours (was 24), so a replay session's early recordings are not pushed
  out by later dictations. Protected targets, cancelled dictations and
  *Clear audio* are unchanged.

## [0.6.0] - 2026-10-27

### Added

- **AI cleanup prompt v4c.** flow-core has a second cleanup prompt, v4c,
  for AI cleanup, which formats the raw transcript in one pass. In chat apps it keeps capitals and words like
  "Okay," and only relaxes punctuation, and it puts a blank line before a
  list. On Llama-3.3-70B through OpenRouter it passes 90 % of the
  formatting suite (v4b: 76 %). The cloud path uses it (D30); the
  on-device model keeps prompt v2.
- **Android: Shrink to a dot.** Settings → Bubble has a "Shrink to a dot"
  switch (default on) under "Shrink bubble" (the old switch, renamed). When
  on, the bubble shrinks to a small 16 dp dot after 5 s idle instead of the
  smaller mic bubble; it scales with Bubble size and is as easy to tap and
  drag as before. The switch is greyed out while "Shrink bubble" is off.
- **Android: Bubble size.** Settings → Bubble has a "Bubble size" slider
  from 75 % to 150 % (steps of 5 %, default 100 %). Every bubble look
  scales with it, including its buttons, waveform and the space it keeps
  from the screen edges, and the bubble resizes as you move the slider. It
  stays where you left it, and a large bubble never ends up off-screen.

### Removed

- **Android: six toasts that repeated what the app already shows.** No
  more toast when you snooze the bubble (the "bubble hidden" notification
  stays), when a shake brings it back (you still feel the buzz), when you
  copy from the bubble's ⧉ button, when you switched fields while
  dictating (the bubble's ✓ / ⧉ / dismiss buttons stay), when the cloud
  failed and another engine took over (the live preview's route label
  shows it), or when the microphone stopped (what was heard is still
  transcribed). Error toasts and copy toasts from History and the
  notification are unchanged.

### Fixed

- **Auto-format without AI: four common dictation shapes.** "First thing
  …, second thing …, third thing …" makes a list without a leftover
  "Thing" on each item; "he told me and I quote … unquote" becomes `He
  told me, "…,"` without the stray "and I"; amounts keep their space
  ("about $250", not "about$250"); and "a hundred and seventy six" is
  `176`, not "a hundred and 76".
- **AI cleanup no longer turns a literal "in parentheses" or "quote" into
  marks.** When the words are used as words ("The formula is in parentheses
  on the board", "Can you send me a quote"), an AI result that adds `( )`,
  `[ ]` or quotation marks is rejected and the fallback rules' text is used.
  Spoken commands (paired, or between pauses) still convert.

## [0.5.0] - 2026-10-24

### Changed

- **AI cleanup is the main path** (D30). With cloud text cleanup on, the
  raw transcript goes straight to Llama-3.3-70B on OpenRouter with prompt
  v4c, which does all the formatting and light edits in one pass; a
  loosened guard checks it and a blank line is always put before a list.
  If the cloud is off, slow (2.5 s), fails or its answer is rejected, the
  on-device pipeline runs as before; with no local model, the fixed
  rules. The debug log and the route label show which one was used and
  why the cloud was not. Cloud off: unchanged.
- **OpenRouter is now the only cloud service** (D28), for cloud
  speech-to-text and for the new cloud text cleanup. ElevenLabs, Deepgram,
  AssemblyAI, direct OpenAI/Groq and custom server addresses are removed
  from the apps (not just hidden): the cloud builds contain no other
  provider address and the build check fails if one appears. The OpenRouter
  address is fixed, your key is only ever sent there, and redirects are
  never followed.
- **What happens to your existing cloud settings:** on the first start after
  the update (Android and Windows), the old provider list is converted once.
  An OpenRouter entry keeps its key, model and consent. Every other provider
  and **its stored API key is deleted** from the phone's Keystore / Windows
  Credential Manager. If one of those other providers was your active cloud
  speech-to-text, cloud speech-to-text is **switched off** (not silently
  moved to OpenRouter). A model with no zero-retention provider is replaced
  by the default. A notice says what changed until you dismiss it. Nothing
  changes if you never used cloud speech-to-text.
- One **OpenRouter** page replaces *Developer → Cloud providers*: Android
  *Settings → Dictation → Cloud (OpenRouter)* (also in Developer settings),
  Windows a new *OpenRouter* settings tab. It holds the API key (never shown
  again after saving; *Check key* shows the credit left and this month's
  usage), privacy switches (zero-retention providers only and no data
  collection, both on; turning one off asks first), speech-to-text and text
  cleanup as two separate switches with their own consent, and usage split
  speech / cleanup under one monthly budget.
- Default cloud speech-to-text model: `openai/whisper-large-v3-turbo`
  (zero-retention providers; about $0.01-0.04 per hour of audio).

### Added

- **Cloud text cleanup (opt-in, off by default):** your dictation can be
  tidied by an OpenRouter model (default `meta-llama/llama-3.3-70b-instruct`)
  instead of, or before, the on-device AI. Its own consent dialog lists what
  is sent: by default only the dictated words, the kind of app, your writing
  style and your dictionary; text around the cursor, on-screen words and the
  app name each have their own switch, off by default. Choose "OpenRouter
  first, device AI as backup" or "device AI first". After 2.5 s the
  formatted text is pasted without waiting (and the request is cancelled so
  it stops billing). It is never used on your own server route, is checked
  against your budget before each call, and works with the local AI off.
- A model picker with every OpenRouter model that can do the job: dedicated
  transcription models, audio-input chat models (marked "may paraphrase";
  answers instead of a transcript are rejected) and text models for cleanup.
  Search, filters (zero-retention only, fast providers, free), price per hour
  of audio or per dictation, and any model id typed by hand. The list is
  only downloaded when the page opens or a cloud use is on, cached 24 h, and
  a built-in list works offline.
- Exact cost per dictation from OpenRouter, shown in usage and in the debug
  log (`llm_engine`, `llm_model`, `llm_cost_usd`, `llm_ttft_ms`,
  `llm_generation_id`, `stt_model`, `stt_cost_usd`).
- Clear error when a chosen model has no zero-retention provider.

### Fixed

- flow-server's result log line printed `speech_ms=Some(7200)
  peak_dbfs=Some(-2)`; it now prints plain numbers and leaves out unknown
  values.

## [0.4.0] - 2026-10-24

### Added

- Android: a dictation that could not go into the field (you switched
  fields while it was transcribing, the field is protected, or typing it
  failed) is never lost any more. The bubble now offers ✓ insert here,
  ⧉ copy and ✕ dismiss, and a "Dictation not inserted" notification stays
  until you tap it to copy the text (it never shows the text itself). The
  text was already saved in History; before this, dismissing the bubble or
  starting the next dictation left History as the only copy. The
  notification copies the same text as History (a list keeps its line
  breaks).
- Android Developer settings: the Routing row and "Route in use" spell out
  the order, e.g. "Phone first → Server" or "Server only".
- Debug log (Android): `route_order` (the engines tried, in order),
  `speech_ms` (speech the engine's VAD heard) and `peak_dbfs` (peak level),
  so an empty "blank_asr" result can be diagnosed from the log alone.
- Android History: a dictation that was not inserted says why on its detail
  page ("You switched apps", "Protected field", "Couldn't paste") and has
  *Insert with the bubble*: go back to the app, tap the field and press ✓ on
  the bubble. Each row also names where it was transcribed ("Server · AI
  cleaned"). Shown for dictations saved from this version on.
- flow-core: `timings.speech_ms` and `timings.peak_dbfs` in every commit
  result (additive). flow-server logs them, plus `start_to_commit_ms`, on
  each result line.

### Changed

- Android: every screen now has the Settings look (grouped cards, rows that
  show their current value, TalkBack-friendly rows, 48 dp targets, light /
  dark with the system).
  - Main screen: a *Setup* card lists the five permissions, each with its
    state ("Granted") or the action that fixes it ("Grant"). While one that
    dictation needs is missing, a banner at the top names it and *Setup*
    moves above *Status*. The phone-specific "Keep it running" steps fold
    open and closed.
  - Dictionary & snippets: separate *Dictionary* and *Snippets* lists, *Add*
    in the top bar, search once a list reaches 8 entries, an edit page whose
    *Save* turns on only when something changed and is valid (errors appear
    under the field, not as toasts), a prompt before discarding edits, and
    a confirmation before deleting.
  - History: dictations grouped by day (Today, Yesterday, …), one line each
    with the time, app and result. A detail page has the full text, the raw
    transcript and the timings. A dictation that was not inserted says "Not
    inserted — tap to copy" and copies when tapped.
  - Model import (speech, Parakeet, AI cleanup): a status card shows the
    active model, its size and the engine. Recommended models list size,
    speed and quality, and tapping one copies its download link. Deleting
    an imported model now asks first.
  - Cloud provider page: the same layout. The API key field is still never
    exposed to accessibility services.

## [0.3.0] - 2026-10-24

### Added

- Spoken formatting commands work reliably (D27): say "quote … end quote",
  "open paren … close paren", "open bracket … close bracket" or "dash" and
  you get `"…"`, `(…)`, `[…]` and `—`, with or without the AI model. A
  command said on its own ("in parentheses the USB-C one") is converted only
  where the pauses make it an aside, and literal uses ("a quote from the
  CEO") stay words. In our 88-case test set, correctly formatted dictations
  go from 44 % to 92 %, and none of the 16 must-not-change cases regress.
- Writing style per app bucket (D27 layer 1), both apps: Personal messages /
  Work messages / Email / Other, each Formal, Casual or Very casual with an
  example line (Windows also offers Excited). Chats now default to **casual**
  (no stiff final period), email and everything else stay formal. Android
  sends the styles at all for the first time (Settings → Dictation → *Writing
  style*, applies at once; hub reads "Chats casual · Email formal"); Windows
  moves an untouched all-formal set to the new defaults once.
- *Style per app*: put an app into another bucket than the built-in list
  (Android `package = work`, Windows `exe-or-host = work`).
- Debug log fields `style` (style sent for the target) and `llm`
  (`formatted`, the core `fallback_reason`, or null), both platforms
  (additive to the D21 schema).

### Changed

- The formatting rules run without the AI model (D27): with "Clean up with
  local AI" off, Android now gets spoken punctuation, quotes / parentheses /
  dashes, fillers, numbers, lists and the style; the texts say so.
- Windows: LLM budget default 1500 → 3000 ms (a stored 1500 moves once);
  cleanup level "None — exactly what you said" is now **No AI** (formatting
  rules only), plus a separate **Verbatim** (= Auto-format off); a stored None
  opens as Verbatim, the old "Raw mode" box (it only ever skipped the LLM)
  becomes No AI and is removed (`prefs_rev` 3).
- "AI not needed (already clean)" (`llm_skipped_clean`) shows as a normal
  result in the Developer *Active engine* row and Android history, not as a
  failure; other fallbacks read "AI too slow / result not used, formatted text
  used".
- The AI cleanup (prompt v2) keeps formatting you already dictated, makes
  lists only when you actually enumerate, and is skipped when the text is
  already clean, so most short dictations commit with no AI wait (48 of 88
  test cases; commit p50 0 ms, p90 2.1 s with the 1.5B model). Its output
  is rejected when it invents a number you did not say.
- flow-server and the apps take ~15 s longer to load the AI model (its
  instructions are prepared at load so the first dictation is not slower).
- AI cleanup notes mention the slightly longer model load (prompt v2
  instructions are prepared at load).

## [0.2.1] - 2026-10-24

### Fixed

- Windows installers are attached to the release again: the v0.2.0 tag
  build stopped at the clean-tree check (the Windows checkout turned the UI
  files into CRLF, so the front-end build showed `ui/dist` as modified) and
  its release build lost the sherpa-onnx / onnxruntime DLLs after a warm CI
  cache. The UI files are now LF on every checkout, the DLLs are copied from
  the prebuilt package, and the clean-tree check runs on every push. The
  v0.2.0 release has the Android APKs only; 0.2.1 is the same app code with
  both platforms' artifacts.
- `tools/release.sh` dry run prints `a/FILE` instead of `a/a/FILE`.

## [0.2.0] - 2026-10-24

### Added

- Versioning (D26): one product version in the top-level `VERSION` file,
  derived by every crate, the Tauri installer and the Android
  versionName/versionCode (`MAJOR*10000 + MINOR*100 + PATCH`); git commit +
  dirty flag shown on the Android main screen and Developer → Build, the
  Windows About row and Developer build line, flow-server's `ready` log line
  and `hello_ok.server`, and as `app_version` / `build` on every D21 debug-log
  line. `tools/check-version.sh` (CI + `tools/check-build-matrix.sh`),
  `tools/release.sh` (local commit + annotated `vX.Y.Z` tag), tag-triggered CI
  builds with versioned artifacts and a GitHub release.
- Shared Rust core `flow-core` (D2): the §9 v2 session API (start / push /
  late context / partials / commit / cancel), whisper.cpp ASR (D3/D4),
  Parakeet-TDT via sherpa-onnx in the same binary (D12), energy+ZCR VAD and the
  silence-hallucination guard (D16), the whisper encoder window fitted to the
  clip (D20), Wispr's deterministic post-processing (all 108 Windows + 58
  Android vectors), deterministic spoken cleanup (D15), opt-in local LLM
  cleanup through the `flow-llm` llama.cpp plugin with the support/coverage
  guard and the short few-shot prompt (D5, D10, D11, D13, D14), C ABI and JNI
  bindings (`flow-ffi`), `flow-cli` dev harness.
- Hybrid local/server dictation (D17): `flow-router` (one §9 API over local,
  server and cloud routes, buffered replay, circuit breaker), `flow-remote`
  WebSocket client, self-hosted `flow-server` (LAN/Tailscale only, bearer
  token, stores no audio or text, systemd unit, `tools/server/install.sh`),
  wire protocol v1 (`flow-protocol`), equivalence / toggle-matrix / build-matrix
  contract tests.
- Opt-in third-party cloud STT (D18): `flow-cloud` with OpenAI-compatible,
  OpenRouter, ElevenLabs, Deepgram and AssemblyAI adapters, per-provider
  consent, keys in OS secure storage, never in client-only builds.
- Hands-free endpointing (D19) and pause-segmented incremental decoding for
  long dictations (D22), with sentence case and a closing period at segment
  joins (D25).
- Windows app (Tauri 2, D1): overlay pill, tray, push-to-talk / tap-lock
  hotkeys, paste with clipboard restore, history, dictionary, learn from
  edits, settings and Developer tab, client-only and hybrid MSI/NSIS
  installers built in CI.
- Android app (Kotlin, D1/D6): accessibility-service dictation, `local` /
  `hybrid` / `remote` flavours with per-flavour network and native-library
  gates (D7, D17), on-device whisper / Parakeet, Developer settings hub,
  model import, debug APKs built in CI.
- Opt-in local transcript debug log on both platforms (D21).
- `tools/server/install.sh --llm` / `--llm-model FILE`: builds flow-server
  with the `llm` feature, installs the `libflow_llm.so` plugin and (for a new
  config) writes `llm_path` / `llm_lib`; an existing config with `llm_path`
  turns `--llm` on by itself (before, re-running the installer on such a box
  installed a server that refused to start).
- Server-side LLM cleanup cost measured (`docs/findings/2026-10-23-server-llm-cleanup-cost.md`;
  `hybrid_latency --remote-only / --reps / --jsonl`): Qwen2.5-1.5B on the
  home box adds about +3.1 s median release→text and 1.8 GB RSS. The user
  keeps LLM cleanup on, on the server and the phone (2026-10-24).

### Changed

- Android main Settings is now a hub with focused sub-pages, in the Developer
  settings look: status card, Dictation (Language, Speech engine, AI cleanup,
  Timing & limits), Bubble & apps, Privacy, Advanced, About (version + build,
  7-tap Developer unlock). Each row shows its current value; switches and
  choices apply at once, typed fields save from the top bar with a discard
  prompt. The main screen's Language / AI cleanup lines open their pages.
  Same prefs keys.
- ASR model defaults refreshed from the benchmarks (D4, D23): Parakeet-unified
  English on the server, greedy decoding without hotwords, long clips decoded
  in ≤ 20 s pieces.

### Fixed

- Never fail a dictation while the server is unreachable and on-device is
  allowed (D24): on-demand phone-model load with buffered replay, longer
  adaptive connect timeout, connection pre-warm.
- All P1/P2 findings of the 2026-10-22 review (`docs/reviews/2026-10-22-integration.md`).

[Unreleased]: https://github.com/bashycodes/murmur/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/bashycodes/murmur/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/bashycodes/murmur/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/bashycodes/murmur/compare/v0.3.0...v0.4.0
