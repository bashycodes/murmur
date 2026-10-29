# UI refactor to the app design

Reference: `docs/design/app-design/` (export of the "Murmur App Design" canvas).
Goal: desktop (Tauri) and Android UIs match the design. Elements that need backend
features we don't have are left out until a backend chunk adds them; no dead switches.

## Desktop (apps/flow-windows/ui)

| # | Chunk | Depends on | Status |
|---|---|---|---|
| W1 | Split `settings.ts` into `lib/` + `pages/`; theme tokens (light/dark), components.css, sidebar shell, window 1280x800, vendored Figtree/EB Garamond | — | in progress |
| W2 | Flow bar restyle (overlay.*) | — | in progress |
| W3a | Home + history | W1 | |
| W3b | Dictionary + Snippets | W1 | |
| W3c | Style | W1 | |
| W3d | Settings General + System + Privacy | W1 | |
| W3e | Settings Speech + Your server (out from behind the developer unlock) | W1 | |
| W3f | Settings AI cleanup + OpenRouter + model picker | W1 | |
| W3g | Insights (current data) | W1 | |
| W3h | Scratchpad page in main window; Notetaker as "Later" | W1 | |
| W5 | Backend batch (one Rust implementer: `app.rs`/`build.rs`/capabilities are shared): theme pref, route per history row + route share, insights per-day/fixes, dictionary `learned`, retention values, `onboarded` flag, overlay "open settings at page" | W1 | |
| W6 | First-run onboarding (file/folder model pick; downloader is a separate D9-gated chunk) | W5 | |
| W7 | Transforms | spec needed | not started |

## Android (apps/flow-android)

| # | Chunk | Depends on | Status |
|---|---|---|---|
| A1 | Tokens (light + dark derived from `.lw-dark`), fonts, DevUi helpers restyle + new helpers | — | in progress |
| A2 | Shell: top bar, bottom nav (Home/Dictionary/Style/Snippets), drawer, Home tab, Permissions screen | A1 | |
| A3 | Dictionary + Snippets tabs | A2 | |
| A4 | Style tab | A2 | |
| A5 | Settings hub + Speech page (routing moves out of Developer) | A2 | |
| A6 | Bubble restyle | A1 | |
| A7 | Remaining screens (History, Notes, Insights, OpenRouter, model picker) in the new look | A3–A5 | |

Features with no place in the Android design (Notes, Quick note, Insights, History, Start/Stop
bubble, Developer, backup) go into a second drawer card.

## Not backed today (follow-ups, not rendered)

Transforms; model downloader; Retry transcription (no stored audio); bubble opacity;
"Use what's on screen"; "Use dictionary words" switch; dictionary learned flag (shared format);
user display name in greeting; "Up to date" check (no runtime network, D9).
