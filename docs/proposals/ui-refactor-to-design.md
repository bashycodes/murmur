# UI refactor to the app design

Reference: `docs/design/app-design/` (export of the "Murmur App Design" canvas).
Goal: desktop (Tauri) and Android UIs match the design. Elements that need backend
features we don't have are left out until a backend chunk adds them; no dead switches.

## Desktop (apps/flow-windows/ui)

| # | Chunk | Depends on | Status |
|---|---|---|---|
| W1 | Split `settings.ts` into `lib/` + `pages/`; theme tokens (light/dark), components.css, sidebar shell, window 1280x800, vendored Figtree/EB Garamond | — | done |
| W2 | Flow bar restyle (overlay.*) | — | done |
| W3a | Home + history | W1 | done |
| W3b | Dictionary + Snippets | W1 | done |
| W3c | Style | W1 | done |
| W3d | Settings General + System + Privacy | W1 | done |
| W3e | Settings Speech + Your server (out from behind the developer unlock) | W1 | done |
| W3f | Settings AI cleanup + OpenRouter + model picker | W1 | done |
| W3g | Insights (current data) | W1 | done |
| W3h | Scratchpad page in main window; Notetaker as "Later" | W1 | done |
| W5 | Backend batch (one Rust implementer: `app.rs`/`build.rs`/capabilities are shared): theme pref, route per history row + route share, insights per-day/fixes, dictionary `learned`, retention values, `onboarded` flag, overlay "open settings at page" | W1 | done |
| W6 | First-run onboarding (file/folder model pick; downloader is a separate D9-gated chunk) | W5 | done (downloader: D31) |
| W7 | Transforms (`docs/proposals/transforms.md`, D32): page, editor, opt-in, auto apply, view changes | W1, W5 | done |

## Android (apps/flow-android)

| # | Chunk | Depends on | Status |
|---|---|---|---|
| A1 | Tokens (light + dark derived from `.lw-dark`), fonts, DevUi helpers restyle + new helpers | — | done |
| A2 | Shell: top bar, bottom nav (Home/Dictionary/Style/Snippets), drawer, Home tab, Permissions screen | A1 | done |
| A3 | Dictionary + Snippets tabs | A2 | done |
| A4 | Style tab | A2 | done |
| A5 | Settings hub + Speech page (routing moves out of Developer) | A2 | done |
| A6 | Bubble restyle | A1 | done |
| A7 | Remaining screens (History, Notes, Insights, OpenRouter, model picker) in the new look | A3–A5 | done (OpenRouter + model picker with A5) |

Features with no place in the Android design (Notes, Quick note, Insights, History, Start/Stop
bubble, Developer, backup) go into a second drawer card.

## Not backed today (follow-ups, not rendered)

Transforms and the model downloader are done on Windows (W7 / D32, D31); Android has
neither, so both stay out of the Android UI.

Still unbacked: Retry transcription (no stored audio); dictionary learned
flag (shared format); user display name in the greeting; "Up to date" check (no runtime
network, D9); Home's monthly trend pill, usage by category and "Add a word to Dictionary" on a
row (desktop); Scratchpad "Add to Flow bar" (desktop). Android only: bubble opacity; "Use
what's on screen"; "Use dictionary words" switch (Parakeet decoding stays in Developer → Engine
options); the Style tab's "English only" footer (the core has no such limit).
