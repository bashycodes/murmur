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
