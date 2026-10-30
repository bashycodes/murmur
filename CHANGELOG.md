# Changelog

All notable changes to Murmur (Windows app, Android app, flow-server and
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

- **Speech in languages other than English** no longer goes through the
  English-only clean-up rules (filler words, spoken punctuation, number
  formatting, capitalisation). Whitespace tidy, the closing full stop, your
  dictionary and "press enter" still apply. Groundwork for Hebrew in 0.10.0;
  per-language models are not selectable in the apps yet.

### Fixed

- **Windows build:** a merge slip in 0.9.0's source broke the Windows
  installer build, so 0.9.0 shipped without Windows installers. 0.9.1 has
  them.
- The model-downloader tests now pass on Windows.

## [0.9.0] - 2026-10-30

### Added

- **Windows: Transforms.** Select text anywhere and press a shortcut to have
  AI rewrite it with your own instruction. Two built-ins come ready:
  *Polish* (Win+Alt+1) and *Prompt Engineer* (Win+Alt+2); edit them or make
  up to 20 of your own on the new *Transforms* page (sidebar, between Style
  and Scratchpad), and try one on sample text with *Try it out*. Transforms
  are **off until you turn on *Opt in***, so an upgrade never takes your
  Win+Alt+digit keys. *Auto Apply After Dictation* runs a chosen transform on
  every dictation; if it fails or takes longer than 8 s, your words are pasted
  as dictated and the Flow bar says "Transform skipped", so nothing is lost.
  Win+Alt+O opens Home on the latest transform's before / after. Transforms
  use the same AI cleanup engines (OpenRouter, then this PC) and the same
  privacy check as Command Mode; with nothing selected the Flow bar says
  "Select text to transform". Decision D32.
- **Windows: download a speech model in the app.** First-run setup and
  Settings → Speech to text offer *Download* for Parakeet TDT 0.6B v2 (661 MB),
  Whisper base (148 MB) or Whisper small (190 MB). Nothing downloads until you
  press *Download* and confirm a dialog that shows the size, source and folder;
  then a progress bar with *Cancel* runs while you carry on. Every file is
  pinned to a fixed Hugging Face version and checked against its SHA-256
  before it is used; when it finishes the app switches to the new model. The
  client-only (`local`) build has no downloader. Decision D31.
- **Windows: first-run setup.** A fresh install opens a short guided setup
  over the app window: pick how speech is turned into text (your server, this
  PC, OpenRouter), point the app at a model, choose AI cleanup, and try a
  dictation. *Set up later* skips it; Settings → General → *Run setup* opens
  it again. Existing installs don't see it.
- **Windows: Browse buttons and links that open.** Model and folder fields in
  Settings and setup have *Browse* / *Choose folder* / *Choose file* buttons
  that open the Windows file picker, instead of typing a path. Links to
  Hugging Face, OpenRouter keys and the server guide now open in your browser
  instead of being copied (only links to those sites and this project are
  allowed).
- **Windows: Scratchpad page.** Notes are now in the app window's sidebar:
  pinned then recent notes with their first line and time, search, pin, and a
  click opens the note (or a new one) in the notes window. The page shows your
  Scratchpad shortcut. The notes window itself matches the new look, light
  and dark. *Notetaker* has a "Later" page; nothing behind it yet.
- **Windows: Light / Dark / System theme.** Settings → General → *Theme*. The
  app window, the notes window and the Flow bar all follow it; *System*
  follows Windows as before.
- **Windows: Fix links on the Flow bar.** When a dictation falls back or
  can't run, the bubble has a link to the settings page that fixes it ("Fix
  →", or "Choose a model →" when a model is missing): Speech to text, AI
  cleanup, Your server or OpenRouter. The link is clickable; the rest of the
  Flow bar still lets clicks through to the app underneath.
- **Windows: shorter history retention.** Settings → Data and privacy → keep
  history *Forever*, *30 days*, *7 days* or *Don't keep* (an old "24 hours"
  choice keeps working).
- **Windows: where each dictation ran.** New history rows remember whether
  speech ran on this PC, your server or OpenRouter, and Home shows it as a
  pill on each row (older rows show none).
- **Android: Permissions page.** Setup and status moved off the main screen
  to their own page (menu ☰ → *Permissions*): the five permissions with their
  state, *Keep it running* for your phone maker, and the engine / language /
  AI cleanup / version status. The drawer shows how many permissions need
  fixing, Home shows a notice that leads there while dictation can't start,
  and the bubble's mic-permission and Accessibility-repair taps open it
  directly.
- **Android: Settings → Speech to text.** Turning your server, this phone and
  OpenRouter on or off, and the order they are tried in, moved from Developer
  settings to this page: one switch per engine, *Change order*, the model on
  this phone with *Import another model*, *Keep loaded while the server is
  used* and *Transcribe while I speak*. Turning OpenRouter on asks for the
  same consent as the OpenRouter page. Engine badge, Parakeet decoding and
  *Test session* stay in Developer settings → *Engine options*.

### Changed

- **New look on Windows and Android.** Both apps now follow the Murmur
  app design (`docs/design/app-design/`): warm light and dark colours, the
  Figtree and EB Garamond typefaces, rounded cards. Every feature works as
  before; the notes below say where things moved.
- **Windows: the settings window is now the app window.** It opens at
  1280×800 with a sidebar (Home, Insights, Dictionary, Snippets, Style,
  Transforms, Scratchpad, Notetaker "Later", an Engines card, Settings).
  Settings is a dialog over it with its own pages: General, System, Data and
  privacy, Speech to text, AI cleanup, Your server, OpenRouter, Developer.
  Changes save as you make them (no footer *Save*); only the server address
  keeps a *Save* button so a half-typed host never reconnects. The tray's
  links open the matching page.
- **Windows: Home and Insights.** Home's history rows show the time, the
  text, hover actions and a *This PC* / *Your server* / *Cloud* pill; app and
  word count are in the time's tooltip, and only "You corrected it" and "Not
  pasted" keep a badge. The side panel adds "Where speech ran this week".
  Insights shows your speed against typing, fixes made (AI cleanup /
  dictionary), total words split by where speech ran, "Where you dictate" by
  app, and your streak with an 18-week activity heatmap.
- **Windows: Dictionary and Snippets.** Tabs (All / Words / Replacements),
  search, A–Z sort, read-only *heard → word* and *say → paste* rows that open
  an edit card, and a *Learn from my corrections* card holding the pending
  suggestions. Every add, edit, delete and import saves at once with an
  *Undo*. Replace-all import and export are under *Import and export*.
- **Windows: Style.** A tab per app group, a banner naming the apps in that
  group, and a card per style with a sample message; a click saves it. *Style
  per app* stays below.
- **Windows: Settings pages.** General shows shortcuts as keycaps with
  *Change*, languages by name, and hands-free silence as a list (1–5 s,
  Never). Data and privacy has a tile per route with *Edit →*, retention, and
  the debug log (moved here from Developer). Speech to text says what each
  engine is doing (this PC's model and whether it is loaded), and the
  Parakeet decoding choice is now a *Use dictionary words when listening*
  switch. Your server shows host and port in one row, timeouts as lists, the
  last health result, and *Test a dictation* next to *Test connection*. AI
  cleanup has the levels Verbatim / Rules only / Light / Medium / High with a
  "You said / Pasted" example and OpenRouter / This PC as a ranked list.
  OpenRouter shows the saved key masked with *Remove* / *Check key*, and this
  month's cost against a budget meter. The model picker has filter chips,
  badges and prices, and says how many models the zero-retention filter
  hides.
- **Windows: Flow bar.** Restyled for every state (idle, hold to talk,
  hands-free with cancel / stop-and-paste, working, done with the engine
  letter, fell back, can't dictate), in light and dark.
- **Android: new app shell.** The main screen is now a top bar with the menu,
  a bottom bar (Home / Dictionary / Style / Snippets) and a drawer (menu ☰)
  with your words / wpm / day streak, an Engines card, Settings, Permissions,
  Help and, under *More*: Notes, Quick note, Insights, History, Start / Stop
  bubble, Speech model and Developer settings. Home lists recent dictations as
  cards by day with the text, time and app, where it ran, *Copy* and ⋮ (*Open
  details*, *Delete*). Back closes the drawer first.
- **Android: Dictionary and Snippets are tabs.** Each has search, cards
  (*heard → word*, *say → paste*) and a + button; learned-word suggestions
  show in a card above the words while any wait. Import / export JSON is the
  top-bar button. The edit page has *Save* and *Delete* in its top bar.
- **Android: Style tab.** Pills per group (Personal / Work / Email / Other),
  the phone's apps in that group (+ adds one), and a card per style with its
  rule and an example; a tap saves it.
- **Android: Settings.** The hub is grouped General (Language, Writing style,
  Bubble, Excluded apps, Timing & limits), Models (Speech to text, AI
  cleanup), Connections (Your server, OpenRouter), Data & privacy (Keep
  history, Learn from my edits, Debug log), Back up & restore, Advanced and
  About. Rows show live state: the server row reads the last check
  ("Connected · 41 ms"), the OpenRouter row this month's cost. *Save* is in
  the top bar on form pages. OpenRouter, the model picker and model import use
  the same top bar; OpenRouter shows this month's cost (total / speech /
  cleanup, with the budget) above the request counts.
- **Android: dictation bubble.** A dark pill with light waveform and controls
  and a dark preview tooltip above it, so it reads on light and dark apps.
  Sizes, positions and behaviour are unchanged; red is used only for errors
  and the repair alert.

- **Android: History, Notes, the note editor and Insights in the new look.**
  Each has the design's top bar with back. History shows dictations as cards
  grouped by day, like Home: the text, the time and app, where it ran, and
  Copy and ⋮ (*Open details*, *Delete*). *Clear all* moved to the top bar.
  Notes are cards too, and *New note* is now the + button. Insights opens
  with the words / wpm / day-streak numbers and how your speed compares with
  typing. Top apps have bars. Everything else works as before, in light and
  dark.

### Fixed

- **Windows: the "max recording" countdown shows.** The warning before a
  dictation hits its length limit was sent but never drawn; the Flow bar now
  shows a countdown chip.

## [0.8.0] - 2026-10-29

### Added

- **Android: back up and restore all your data.** Settings → *Back up &
  restore* → *Export all data* saves one file (`flow-clone-backup-<date>.json`)
  wherever you pick: Notes and the Quick note, History, Dictionary and
  snippets, learned-word suggestions, Insights and all settings. *Restore
  from a backup* checks the whole file first, shows what it holds and, once
  you confirm, replaces the app's data with it, so a reinstall that wiped
  everything gets it all back. API keys and the server token are never in
  the file: enter them again after a restore (models are not included
  either). A backup from a newer app version is refused with a clear
  message. Format: `docs/backup-format.md` (meant for the Windows app too).

### Changed

- **Renamed to Murmur.** Every "Flow Clone" the apps show (Windows
  window titles, tray, Settings About row, installer and exe name; Android
  app name, notifications, accessibility notice, setup and battery help) and
  the release titles now say "Murmur". Data and settings are unaffected:
  the Windows data folder (`%APPDATA%\FlowClone`), the stored keys, the
  launch-at-login entry and the Android package (`dev.flowclone`) keep their
  old internal names, so Android updates in place. On Windows the installer
  is now `Murmur_<version>_…` and installs to a new "Murmur"
  folder: uninstall "Flow Clone" afterwards (settings carry over), and turn
  "Launch app at login" off and on once so it points at the new exe.
- Product text no longer names a specific VPN. App strings (Android main
  screen and Developer settings, Windows settings), server messages, the
  example config, the systemd unit and the docs now say "your LAN or a VPN".
  The address rules are unchanged: 100.64.0.0/10 and fd7a:115c:a1e0::/48
  still count as private when reached through a VPN interface.
- `tools/server/install.sh` without `--bind` now picks the server's first
  100.64.0.0/10 IPv4 from `ip -4 addr` (any mesh VPN) instead of asking one
  VPN's CLI; on such a machine it binds the same address as before.
- The packaged `flow-server.service` no longer orders itself after one VPN's
  daemon; a bind to a VPN address that is not up yet still retries every 3 s.

### Fixed

- AI cleanup with a model entered by id (not in the catalogue) now turns
  reasoning off when the model allows it. Before, the lookup lost the model's
  parameter list, so models that reason by default (e.g. Nemotron 3.5
  Lightning) spent the whole output budget reasoning and returned nothing,
  and every dictation fell back to the raw transcript.

## [0.7.1] - 2026-10-28

### Fixed

- **Windows release build.** The release's version check stopped on the
  Windows build server: it read CHANGELOG.md as Windows-1252 text and failed
  on characters like → and ✎, so 0.7.0 shipped without the Windows
  installer. The version tool now always reads, writes and prints UTF-8.

## [0.7.0] - 2026-10-28

### Added

- **Windows: Command Mode.** Select text, hold **Ctrl+Win+Alt** (change it
  in Settings → General → Shortcuts), say what to do ("make this more
  formal", "translate to Spanish", "make it a bulleted list") and let go:
  the selection is replaced by the rewritten text. With nothing selected
  it writes new text at the cursor ("write a short thank-you note"). Uses
  the AI cleanup model on this PC, or OpenRouter text cleanup if you turned
  that on (the selection only goes to OpenRouter with *Send text around the
  cursor* on). The pill turns orange while it works; Esc cancels. Without
  an AI, or if it doesn't answer in 25 s, you hear the error sound and the
  selected text stays as it was; your clipboard is always put back.
  History shows the instruction, the original text and the result.
- **Android: Command Mode.** Select text in any app and a **✎** button
  appears next to the bubble's mic: tap it, say what to do ("make it
  shorter", "make this more formal", "translate to Spanish"), and the
  selection is replaced by the rewrite; the text around it stays as it was.
  The instruction ends on a pause or ✓ (✕ cancels); the line above the
  bubble says *Command: …* while it listens. *Settings → Bubble → Command
  mode*: *When text is selected* (default), *Always* (also without a
  selection: say what to write at the cursor) or *Off*. Tap, hold and drag
  on the mic work as before. Uses the same AI and rules as Windows Command
  Mode (the on-device AI cleanup model, or OpenRouter text cleanup only with
  *Send text around the cursor* on). Password, PIN, number, phone and URL
  fields, banking apps and your excluded apps are refused before anything is
  read. Without an AI, if it doesn't answer in time, or if the selection
  changed meanwhile, you hear the error tone, the bubble says why and the
  text is left untouched (an answer that could not go in can still be
  copied). History shows the instruction, the original text and the result;
  commands don't count in Insights.
- **Windows: Scratchpad.** A plain-text window to dictate into, edit and
  copy out: tray → *Scratchpad…*, or a shortcut you set under *Settings →
  General → Shortcuts → Open Scratchpad* (none by default). Dictation types
  into it like any other app (style *Other*); its edits are not used to
  learn dictionary words and it is not listed under *Style per app*. The
  text is saved to `%APPDATA%\FlowClone\notes\scratchpad.txt` shortly after
  you stop typing, when the window closes and when you quit, and is there
  again next time. *Copy all*, *Clear* (with *Undo clear*) and a word count.
- **Windows: Notes.** Tray → *Notes…* lists your saved notes (the
  Scratchpad is the pinnable *Quick note* at the bottom, nothing lost):
  title (first line), last edited, word count; search by content; *New
  note*; *Pin* (pinned first); *Export…* `.txt` / `.md` (a copy in
  Downloads, never overwriting a file); *Delete* (click twice). A note opens
  in the Scratchpad editor (← *All notes* to go back) and saves itself the
  same way. Optional shortcut *Settings → General → Shortcuts → New note +
  dictate* (none by default) creates a note, opens it and starts hands-free
  dictation into it. Each note is a plain `.txt` in
  `%APPDATA%\FlowClone\notes\` (plus a small `index.json` for pins and
  times; `.txt` files copied in by hand show up too). An existing
  `scratchpad.txt` is moved there on first start.
- **Android: Scratchpad.** *Main screen → More → Scratchpad* (or long-press
  the app icon → *Scratchpad*) opens one big text field to dictate into,
  edit and copy out: tap it, tap the bubble and talk, like in any other app.
  It is the only screen of Flow Clone's own where the bubble appears. The
  text saves itself as you go (and when you leave the screen) and is there
  again after the app is closed or the phone restarts; it stays on the phone.
  *Copy all* (removed from the clipboard after a minute, like every copied
  transcript), *Share*, *Clear* (asks first) and a word count.
- **Android: Notes.** *Main screen → More → Notes* keeps as many dictated
  notes as you like, in the same editor as the Scratchpad (the bubble works
  in every note, and still nowhere else in Flow Clone). The list shows each
  note's first line and when it last changed, pinned notes first; search
  finds words anywhere in a note; *New note*, and per note *Pin*, *Share* and
  *Delete* (asks first). Long-press the app icon → *New note* opens a fresh
  note with the keyboard up (tap the bubble to dictate; it doesn't start
  recording by itself). The Scratchpad is now the *Quick note*: same text,
  pinned at the top of Notes, still on the main screen and the icon
  shortcut. Notes stay on the phone (no sync), saved as you go. An untouched
  new note isn't kept.
- **Windows: Insights tab.** *Settings → Insights* shows total words
  dictated, words in the last 7 days, average speaking speed (words per
  minute of recording), current and longest daily streak, an estimate of
  time saved versus typing (assumes 40 wpm, shown on the page), the number
  of dictations, your top apps by words, and a 7-day words-per-day chart.
  Everything is counted on the PC in `%APPDATA%\FlowClone\insights.json`
  (words, recording length and app per local day; no text) and never leaves
  it. The counts are kept apart from History, so deleting History or a
  short retention setting doesn't reset them; on first start they are
  filled in once from the History you already have. *Reset stats* (with a
  confirmation) clears them. The stats math lives in `flow-core` so Android
  can reuse it.
- **Android: Insights page.** *Main screen → More → Insights* shows the same
  stats as the Windows Insights tab: total words dictated, words in the last 7
  days, average speaking speed, current and longest daily streak (a streak
  isn't broken before today's first dictation), time saved versus typing
  (assumes 40 wpm, shown on the page), the number of dictations, your top
  apps by name, and a 7-day words-per-day chart. Days follow the phone's time
  zone. The counts live on the phone in their own file (words, recording
  length and app per day; no text), so clearing History or setting it to
  *Never* keeps them; the first time, the History you already have is counted
  in once. *Reset stats* (asks first) clears them. The math mirrors
  `flow-core`'s, checked against the same test vectors.
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
- **Debug: replay harness.** `tools/replay/run.sh --in DIR` re-runs the
  phone's saved dictation recordings on the PC through each speech model
  (Parakeet, whisper, OpenRouter cloud STT), cleanup model (fallback rules,
  local Qwen, OpenRouter models) and prompt version (v4c, v2) on the real
  pipeline, and prints a score table against the expected text and the
  phone's own results, plus the OpenRouter spend (capped, default $0.05).
  Recordings and results stay outside the repo. See `tools/replay/README.md`.
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

- **Windows: per-app style picker.** AI cleanup → Writing style → *Style
  per app* is now a table instead of `app = group` text lines: pick a group
  (Personal / Work / Email / Other) per app, remove rows, and add apps from
  the ones you recently dictated into without typing an exe name (or type
  one, a web host or a URL). The built-in exe → group mapping is listed
  read-only so you can see why an app is in a group. Only exe names are
  remembered (`recent_apps.json`, newest 12, nothing kept with history
  retention "Never"). Same `app_buckets` pref: existing entries load as rows
  unchanged.
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

[Unreleased]: https://github.com/bashycodes/murmur/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/bashycodes/murmur/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/bashycodes/murmur/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/bashycodes/murmur/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/bashycodes/murmur/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/bashycodes/murmur/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/bashycodes/murmur/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/bashycodes/murmur/compare/v0.3.0...v0.4.0
