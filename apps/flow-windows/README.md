# flow-windows — Windows shell of Murmur (a local Wispr Flow clone)

Tauri 2 tray app: hold **Ctrl+Win** to dictate, **Ctrl+Win+Space** (or double-tap
Ctrl+Win) for hands-free, **Ctrl+Win+Alt** for [Command Mode](#command-mode)
(rewrite the selected text by voice), **Esc** to cancel, **Shift+Alt+Z** to
paste the last transcript again, **Shift+Alt+X** to copy it. Text is pasted
into the focused app.
Speech-to-text runs either on this PC (the real `crates/flow-core`: whisper.cpp
or Parakeet) or on **your own** `flow-server` over your LAN or a VPN (hybrid mode,
`docs/hybrid-architecture.md`, D17). Every dictation goes through
`flow_router::Router` (`src/router_adapter.rs`). Nothing reaches a third party
unless you opt in to **OpenRouter** (the only cloud provider, D28) on the
OpenRouter tab: speech-to-text and text cleanup are two separate opt-ins
(`docs/cloud-stt.md`; both off by default). A client-only build contains no
network code at all.

## Shortcuts

Settings → General → Shortcuts takes keys joined with `+`: Ctrl, Win, Alt,
Shift, Space, Esc, A–Z, 0–9, F1–F24, Enter, Tab. At most 3 keys, at least one
modifier (the cancel key may be a single key).

`Ctrl`, `Alt` and `Shift` match either side. `Left Ctrl` / `Right Ctrl`,
`Left Alt` / `Right Alt` and `Left Shift` / `Right Shift` match only that
side (also typed `LCtrl`, `RAlt`, `right-alt`; `AltGr` = `Right Alt`).
Settings shows them as `Right Alt` etc. A key can't be named with and
without its side in one shortcut (`Alt+Right Alt`).

Push-to-talk on **Right Alt alone**: type `Right Alt` (or `RAlt`) as the
push-to-talk shortcut. Left Alt and Alt+Tab don't start dictation; Right
Alt with another key (Right Alt+Tab, an AltGr letter) cancels the dictation
it started and the key reaches the app as usual. Its release is masked like
Win's (`[LCtrl↓, RAlt↑, LCtrl↑]`), so the menu bar doesn't take focus. On
keyboard layouts with AltGr, Windows sends a fake Left Ctrl (scan code
0x21D) with every Right Alt press: the hook ignores it for shortcuts and
lets it through untouched, so it never stops Right Alt from matching, never
makes AltGr look like Ctrl+Alt, and is released normally. A shortcut that
needs Ctrl therefore no longer fires on AltGr alone.

## Scratchpad and Notes

Tray → **Scratchpad…** (or the optional *Open Scratchpad* shortcut, unset by
default, Settings → General → Shortcuts) opens the Quick note (id
`scratchpad`). Dictate into it like any app: the transcript is pasted at the
caret, style bucket *Other*. Learn-from-edits never watches our own windows
and they are not recorded as recent apps. The text is written 750 ms after
the last change, when the window closes and on Quit (`AutoSave` debounce).
*Copy all*, *Clear* / *Undo clear*, word count, *Export .txt / .md*.

Tray → **Notes…** shows the list in the same window (`scratchpad.html?view=notes`;
a note's editor is `scratchpad.html?note=<id>`, ← *All notes* goes back).
It is a view of the Scratchpad window rather than a Settings tab so notes can
be opened and dictated into with Settings closed, and so only that window
holds the `note_*` commands. Rows: title (first line), updated time, word
count, *Pin* (pinned first, then newest), *Export…* (`.txt`/`.md` copy in
Downloads, `<title> (2).md` rather than overwriting), *Delete* (second click;
the Quick note can't be deleted). Search matches the whole text,
case-insensitive. Optional *New note + dictate* shortcut
(`shortcuts.new_note`, unset by default): creates a note, opens it, waits
(≤ 2 s) for its window to be in front — a dictation only pastes into the
window it started in — then starts hands-free; pressed while dictating it
only opens the note.

Storage (`src/notes.rs`): `%APPDATA%\FlowClone\notes\<id>.txt` per note
plus `notes\index.json` (`{"notes": {id: {pinned, created_ms,
updated_ms}}}`). The `.txt` files are the truth: a file without an index
entry is listed (times from the file), an entry without a file is dropped on
the next write, a corrupt index only loses pins. Ids are `[a-z0-9_-]{1,64}`
(no path traversal); new ones are `note-<epoch ms>`. A pre-Notes
`FlowClone\scratchpad.txt` is moved into `notes\` once at startup, never over
an existing one.

## Build flavours (Cargo features)

| Build | Command | Engines |
|---|---|---|
| Hybrid (default) | `cargo build --features app` | this PC + your server, fallback between them, plus opt-in OpenRouter STT / text cleanup (feature `cloud`) and the click-only speech-model downloader (feature `download`, D31) |
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
* Server **host/IP** (IPv4, IPv6, or a name such as `mybox.local`;
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

### OpenRouter (cloud speech-to-text and text cleanup)

Default build only (feature `cloud`; never in `local,app`, enforced by
`tools/check-build-matrix.sh`). OpenRouter (`https://openrouter.ai/api/v1`, a
constant: no base URL field, no presets, no other provider compiled in) is
the only third party the app can reach (D28). Settings → **OpenRouter** tab
(shown only in cloud builds), applied at once:

* **Account**: one API key (password field, Paste, Save key, Clear) in
  Windows Credential Manager as `flow-clone/cloud/openrouter` (bound to
  openrouter.ai + the openrouter adapter), handed to the router with
  `set_cloud_key("openrouter", …)`. Never in `prefs.json`, never logged,
  never sent back to the page (it shows `••••` + last 4). **Check key**
  (`GET /api/v1/key`): label, credit limit / remaining, usage this month,
  free tier, and a warning when the key has no credit limit.
* **Privacy** (both uses): "Zero-retention providers only" (default on,
  confirm before turning off) and "Only providers that don't collect data"
  (default on): every request carries `provider: {zdr, data_collection}`.
* **Speech-to-text (sends audio)**: opt-in with its own consent dialog
  ("Send audio to OpenRouter?"); model picker; language; send dictionary
  words; path (auto / transcription endpoint / chat model with audio);
  **Test** (bundled 3 s public-domain clip, typed key allowed). Turning it on
  puts *OpenRouter* first in Developer → Engines (reorder there).
* **Text cleanup (sends text)**: opt-in with a separate consent ("Send
  dictated text to OpenRouter?"; audio consent does not count, nor the
  reverse); what is sent (transcript, app category + style, dictionary;
  text around the cursor / on-screen words / app name each off by default);
  order *OpenRouter first, then this PC's model* or the reverse; provider
  sort; time limit (800-8000 ms, default 2500); **Test** with a fixed sample
  sentence; status (blocked reason, circuit, last call). Your own server
  keeps using its own AI (no cloud cleanup on the server route).
* **Model picker**: every OpenRouter model that can do the job (live
  catalogue, `flow_cloud::CatalogStore`): search, filter chips
  (Transcription / Chat audio for STT, Text for cleanup; ZDR only (default),
  Fast providers, Free), price, context length, badges (`ZDR`, `fast`,
  `chat model, may paraphrase`, `reasoning`, …), recommended first, and a
  custom model id field. Fetched only when the tab opens, a use is switched
  on, or a use is on at startup (24 h cache in
  `%APPDATA%\FlowClone\cache\openrouter-catalog.json`, baked offline list
  otherwise; the header says Live / Cached / Offline list).
* **Usage this month**: `Speech $x · Cleanup $y · total`, from
  `%APPDATA%\FlowClone\cloud_usage.json` (exact `usage.cost`, `≈` when
  estimated; restored at startup, never sent anywhere); optional monthly
  budget for both uses; **Reset counters**.
* Active engine row: `cloud · OpenRouter · <model> · $0.0012`, and for the
  LLM step `AI cleaned · OpenRouter · <model> · $0.0007` (`AI not needed
  (already clean)` for `llm_skipped_clean`, a normal result). The Flow bar
  shows **C** after every cloud STT dictation. A cloud failure that fell
  back shows "Cloud STT failed (key rejected); used this PC" above the pill
  (`no_private_route`: "no zero-retention provider serves this model").
* **One-time migration** (first start of a D28 build): a D18 provider list
  in `prefs.json` becomes the single OpenRouter config
  (`RouterConfig::migrate_d28`). An OpenRouter entry keeps its model,
  language, dictionary setting and audio consent, and its key moves from
  `flow-clone/cloud/<old id>` to `flow-clone/cloud/openrouter` (only if it
  was saved for openrouter.ai). Every other provider and its stored key is
  deleted; their cost this month stays in a `removed` counter so the budget
  is unchanged. If another provider was active, cloud STT is switched off
  (never moved to OpenRouter silently). A model without a zero-retention
  provider (`openai/gpt-transcribe`) becomes the default. The notice ("Cloud
  speech-to-text now uses OpenRouter only. …") shows on the OpenRouter tab
  until you press OK.

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
here); D27 adds `style` (the style sent for the target's bucket: `formal` /
`casual` / `very_casual` / `excited`) and `llm` (`formatted` = the model's text
was used, the core `fallback_reason` such as `llm_skipped_clean` — nothing for
the model to do, a normal result — or `llm_timeout`, or `null` = LLM off),
same names as Android.

It can never affect a dictation: `DebugLog::log` only pushes onto a
bounded queue (oldest dropped when full) and a writer thread does the
serialising and I/O; every failure there is swallowed and counted (shown
next to the count). Never logged: the server token and cloud API keys
(scrubbed from every line), clipboard contents; a password field gives
`target`, `raw` and `text` = `"<redacted>"` and no steps. Nothing is uploaded anywhere.
Code: `src/debug_log.rs`, lines built in `src/controller.rs`.

## Custom sounds

To replace a cue with your own, drop a WAV file into
`%APPDATA%\FlowClone\sounds\` named `start.wav`, `stop.wav`, `lock.wav`
(hands-free locked), `paste.wav` or `error.wav`, then restart the app (the
sounds are loaded once, at the first cue). Plain PCM WAV is recommended; the
file must be a RIFF/WAVE file of at most 2 MB, otherwise the built-in tone
plays and a `[sounds]` line on stderr says why. Any cue without a file keeps
its built-in tone, and turning sounds off in Settings silences custom ones too.

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

The formatting rules run without it (D27): while "Auto-format punctuation,
quotes, fillers and corrections" is on, spoken punctuation, quote / unquote,
parentheses, brackets, dash, fillers, "scratch that", numbers, lists, email
layout and the writing style are applied on this PC. Settings → AI cleanup
turns on llama.cpp cleanup on top (run-on sentences, leftover fillers,
corrections, tone per app; DECISIONS D5/D10/D11). It needs two files, and
neither is downloaded automatically:

* the GGUF model, default `%APPDATA%\FlowClone\models\qwen2.5-0.5b-instruct-q4_k_m.gguf`
  (`qwen2.5-1.5b-instruct-q4_k_m.gguf` is more accurate, ~2x slower);
  `tools/fetch-models.sh --llm` fetches both on explicit request;
* the engine plugin `flow_llm.dll` **next to `flow-windows.exe`** (or the
  path in "Engine library"). llama.cpp cannot be linked into the exe:
  whisper.cpp and llama.cpp each bundle a different ggml (D10).

Speech recognition is ready long before the LLM (10-50 s load on CPU,
a few seconds more since prompt v2, whose instructions are prefilled at
load); until then, and whenever cleanup fails or exceeds its budget, the
formatted transcript is pasted. The tray line "AI cleanup: …" and Settings
show the state (off / loading / ready / model missing / engine missing /
failed). When the formatted text leaves nothing to fix the model is skipped
(`llm_skipped_clean`; Developer → Active engine shows "AI not needed
(already clean)", not a failure).

Settings → AI cleanup:

* **Time budget** default 3000 ms (was 1500, which timed out on nearly every
  dictation; a stored 1500 moves to 3000 once, `prefs_rev` 3).
* **Cleanup level** *No AI — formatting rules only* (was "None — exactly
  what you said"; since D27 the core still formats at `none`), Light /
  Medium / High, or *Verbatim — exactly what you said* = `cleanup_level:
  none` + Auto-format off (the two controls stay in step). Migration
  (`prefs_rev` 3): a stored None becomes Verbatim (what it did before); the
  old "Raw mode" box, which only ever skipped the LLM, becomes No AI and is
  gone from the UI.
* **Writing style** per bucket (Personal messages / Work messages / Email /
  Everything else: Formal, Casual, Very casual, Excited, each with an example
  line). Defaults: chats casual (no final period), email and everything else
  formal; an untouched all-formal set moves to these once. **Style per app**:
  a table of apps with a group dropdown (Personal / Work / Email / Other) and
  Remove. *Add* offers the apps you recently dictated into (exe names only,
  newest 12, `recent_apps.json`; memory only with history retention
  "Never"; *Forget recent apps* clears it) or takes a typed exe name / path,
  web host or URL; duplicates and `=` are refused. A collapsed *Built-in
  groups* list shows the default mapping read-only (overridden rows struck
  through, *Change* copies one into your table). Saved as `app_buckets`
  (exe stem or host → group), over the built-in lists
  (`data/app-lists.json`: WhatsApp, Telegram, Signal, Messenger, Discord,
  Instagram, Google Messages → personal; Slack, Teams → work; Outlook,
  Gmail → email).

## Command Mode

Select text, **hold Ctrl+Win+Alt** (Wispr's default; Settings → General →
Shortcuts → *Command Mode*), say what to do ("make this more formal",
"translate to Spanish", "turn this into bullet points") and release. The
pill turns orange while it listens and while the AI works; the answer
replaces the selection (pasted like a dictation, your clipboard restored
afterwards). With nothing selected it writes new text at the cursor
("write a short thank-you note to the team"). Holding push-to-talk and
adding Alt turns that dictation into a command. Esc cancels at any point
before the paste.

* The instruction is transcribed on the normal speech route, word for word
  (no cleanup); the selection is read with UI Automation (TextPattern) and,
  for controls without it, with a Ctrl+C whose clipboard change is undone
  right away (only if nothing else wrote to the clipboard meanwhile).
* The rewrite needs an AI: the local model from [AI cleanup](#ai-cleanup-optional-off-by-default),
  or OpenRouter text cleanup (same order, consent and budget as cleanup).
  The selected text goes to OpenRouter only with *Send text around the
  cursor* on; otherwise the command uses the local model, or says it
  can't. Prompt and answer cleaning (`crates/flow-core/src/command.rs`)
  strip preambles ("Sure, here is…"), wrapping quotes, code fences and
  trailing notes, so only the result is pasted.
* No AI, model still loading, a selection over 4000 characters, a password
  field, a non-text selection or no answer within 25 s: the error sound
  plays, the pill says why, and nothing is pasted (the selection stays as
  it was).
* History records a command as instruction (`raw`), result and the
  replaced text (`original`, column `kind = 'command'`); Paste last
  repeats the result.

## Layout

| Path | What |
|---|---|
| `src/hotkeys/chord.rs` | Pure chord matcher: PTT / hands-free / Command Mode / cancel, Win/Alt release masking, foreign-key abort, either-side vs side-specific keys, AltGr fake-Ctrl filter |
| `src/hotkeys/win.rs` | `WH_KEYBOARD_LL` hook thread. It skips our own `dwExtraInfo`-tagged input, does masked Win/Alt releases (`[LCtrl↓, key↑, LCtrl↑]`) and runs a 1 s stale-key probe |
| `src/dictation.rs` | Pure state machine `idle → listening(ptt\|locked) → processing → pasted\|error → idle`. Wispr's timings: 500 ms tap / double-tap lock / triple-press, warning at 19 min, hard stop at 20 min, 35 s processing timeout, 500 ms error |
| `src/controller.rs` | Runs the machine's effects against a `Platform` trait plus `CoreApi`; integration-tested with fakes. Command Mode: selection capture after the chord is up, `CoreApi::command`, history row |
| `src/audio/` | Resampler to 16 kHz mono i16 and a Wispr-style level meter (pure); `capture.rs` = cpal/WASAPI |
| `src/paste/` | Pure strategy, restore guard and per-app restore delay; `win.rs` = clipboard snapshot → CF_UNICODETEXT plus history/monitor opt-outs → masked Alt/Win release → `[Ctrl↓,V↓]` 50 ms `[V↑,Ctrl↑]` → restore if the sequence number is unchanged. Falls back to KEYEVENTF_UNICODE |
| `src/context/` | Truncation, password suppression, app kind and style bucket from `data/app-lists.json` (pure); `uia.rs` = UI Automation snapshot with a 300 ms timeout, and `UiaFieldReader` (re-reads the pasted-into control, `CompareElements` for focus) |
| `src/ducking/` | Mute plan and restore rule (pure); `win.rs` = WASAPI sessions on an MTA thread |
| `src/sounds.rs` | Synthesised cue WAVs (pure), optional user overrides from `%APPDATA%\FlowClone\sounds`, played with `PlaySoundW(SND_MEMORY)` |
| `src/prefs.rs` | `%APPDATA%\FlowClone\prefs.json`: tolerant load, atomic save, validation, `prefs_rev` migration (rev 1, D22: a stored hands-free auto-stop of exactly 2000 ms, the old default, becomes 1500 once; rev 3, D27: LLM budget 1500 → 3000, all-formal styles → chats casual, None → Verbatim, raw mode → No AI; later saves keep any value) |
| `src/core_api.rs` | §9 API seam (shell types) plus `StubCore` for tests |
| `src/core_adapter.rs` | `CoreApi` on `flow_core::FlowCore`: two-phase background load (whisper engine, then the optional `flow_llm` cleaner, composed with `FlowCore::from_engines`), `ModelStatus` / `LlmStatus`, shell→core type mapping |
| `src/history.rs` | Local SQLite history (`history.sqlite`, 10-clone-spec §2 columns, no audio), keep / 24 h / never retention; list/search/get/delete/clear/set_edited for the History tab |
| `src/insights.rs` | Insights tab store (`insights.json`: per-local-day words / audio ms / dictations / words per app, no text), separate from History so deleting it keeps the stats; one-time backfill from History; the math is `flow_core::insights` |
| `src/learn.rs` | Learn-from-edits (03 §3), pure: anchor-bounded region, Wispr end reasons, watch state machine in injected time, semi-global trim of text typed after the dictation |
| `src/recent_apps.rs` | Exe names recently dictated into (`recent_apps.json`, names only) and the picker's built-in exe → group rows |
| `src/suggestions.rs` | Learned words waiting for an explicit accept (`suggestions.json`); dismissed words are never re-suggested |
| `src/notes.rs` | Notes: one `<id>.txt` per note in `%APPDATA%\FlowClone\notes\` + `index.json` (pinned, created/updated), tolerant of missing/stale entries; list/search/pin/delete/export; Scratchpad migration; atomic save, debounced `AutoSave` with injected time, flush on close/quit |
| `src/dictionary.rs` | Import/export of the shared `[{word, replacement, isSnippet}]` JSON: tolerant parse, validation, case-insensitive merge |
| `src/overlay.rs` | Pill hit-testing so the hands-free ✕/✓ are clickable while the canvas stays click-through |
| `src/hotkeys/watchdog.rs` | Hook-liveness watchdog (probe + re-install after a `LowLevelHooksTimeout` removal) |
| `src/router_adapter.rs` | D17: `CoreApi` on `flow_router::Router` (every build), `LocalBackend` over the `FlowCoreAdapter` slot, remote = `flow_remote::factory` (feature `remote`), router errors → pill text, last-route record (Developer page, S/L badge) |
| `src/dev_settings.rs` | Developer settings: `prefs.json` `"developer"` section, form validation (host/port/pin/token/timeouts, public host ⇒ TLS), mapping to the router's `RouterConfig` JSON (pinned by `crates/flow-router/fixtures`) |
| `src/credentials.rs` | `CredentialStore`: Credential Manager (`flow-clone/cloud/openrouter`, `CredWriteW`/`CredReadW`/`CredDeleteW`) on Windows, `MemoryStore` stand-in elsewhere; masking, key validation, host binding, D28 key migration |
| `src/debug_log.rs` | Opt-in debug log: bounded queue + writer thread, JSONL file sink (last 500), schema, secret scrubbing |
| `src/cloud.rs` | Pure OpenRouter helpers: failure texts, engine / LLM rows, `cloud_usage.json` load/save, usage split view, catalogue cache path, the startup D28 migration (settings + keys + usage) |
| `src/openrouter.rs` | The OpenRouter page: `OrView`, validated edits (STT / cleanup with separate consents, privacy, budget, model) |
| `tests/cloud_settings.rs` | Page edits + Developer form → prefs (no key) → router JSON == `router_config_cloud.json`; separate consents; the D28 migration of every old preset fixture (settings, Credential Manager, usage) |
| `tests/cleanup_order.rs` | D36 cleanup order: prefs hold this PC / your server / OpenRouter with a switch each (default this PC → server → OpenRouter off); pre-D36 `allow_cloud_cleanup` + `cleanup_order` migrate on load (cloud-vs-local order kept, server right after this PC, cloud on only if it was allowed); the router always gets all three (`cleanup_order` + `cleanup_off`). UI: `tools/probes/settings_mock/check_cleanup_order.py` |
| `tests/cloud_integration.rs` | Controller → router → flow-cloud → flow-cloud's mock OpenRouter (via `flow_cloud::testing::override_base_url`, never a config URL): cloud paste with exact cost, ZDR body, usage restore, 500/401/503-routing → local fallback with all audio, no key/consent → no request, cloud-only error pills, Test; key never shown |
| `src/shell.rs` | `pick_path` (Win32 `IFileOpenDialog` on its own STA thread, owned by the settings window; validated title / `*.ext` filters) and `open_external` (`ShellExecuteW`, only https/http links to `github.com/bashycodes/murmur…`, `huggingface.co`, `openrouter.ai`) |
| `src/model_download.rs` | D31 speech-model downloader: fixed catalogue (Parakeet v2 int8, Whisper base, Whisper small q5_1) pinned to Hugging Face commits with size + SHA-256 per file; `.part` → verify → rename; one at a time, cancellable; `model:download` progress events. HTTP only with feature `download` |
| `src/secrets.rs` | DPAPI (`CryptProtectData`) blobs for secrets in prefs (server token; reusable for later API keys) |
| `tests/router_integration.rs` | Controller → router → flow-remote → in-process flow-server (MockAsr) vs local MockAsr: all four toggle combos, server down / killed mid-dictation (no lost audio), error pills, Test connection / Test session |
| `tests/core_integration.rs` | Controller + real flow-core (MockAsr, and whisper tiny.en on `jfk.wav` when present): audio → commit → paste plan → history |
| `src/app.rs` | Tauri: tray, overlay window (512x614, click-through, `WS_EX_NOACTIVATE`), settings window, Scratchpad/Notes window (created on demand), commands, autostart |
| `build.rs`, `capabilities/` | Every command is declared in the app manifest; the settings window gets all of them except the note commands, the overlay only `overlay_action` + `overlay_theme` (not `get_prefs`), the Scratchpad/Notes window only `note_*` + `clipboard_copy` (`tests/capabilities.rs`) |
| `tools/ci/` | CI helpers: Parakeet DLL bundle overlay, installer collection (`.github/workflows/windows-build.yml`) |
| `ui/` | Plain TypeScript → `ui/dist/*.js` (committed). No bundler, no CDN, no web fonts; the CSP is `'self'` only. Settings tabs: General, AI cleanup, History, Insights, Dictionary, Developer; `scratchpad.html` (`?note=<id>`, default `scratchpad`; `?view=notes` = Notes list) |
| `tauri.llm.conf.json` | Config overlay that bundles `flow_llm.dll` next to the exe (`cargo tauri build --config tauri.llm.conf.json`) |
| `tools/probes/settings_mock/` | Serves `ui/dist` with a fake `__TAURI__` for checking the settings page (and `scratchpad.html`: `check_scratchpad.py`, `check_notes.py`) in a plain/headless browser |

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
# with cloud STT and the model downloader (reqwest + rustls/ring), with
# cloud STT only, hybrid with and without TLS,
# client-only and server-only.
for f in app,local-engine,remote,cloud,download app,local-engine,remote,cloud app,local-engine,remote,tls app,local-engine,remote app,local-engine app,remote; do
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
   (or the run page → Artifacts), unzip, run `Murmur_<version>_x64-setup_hybrid.exe` (e.g. `Murmur_0.8.0_x64-setup_hybrid.exe`)
   (per-user, no admin) or the `.msi`. Install only one flavour at a time
   (same app id). The tray icon appears; Settings opens on first start.
2. **Model** (only for "this PC"): copy `ggml-base.bin` (or `ggml-tiny.en.bin`
   + set the path in Settings → Speech model) to
   `%APPDATA%\FlowClone\models\`, or the Parakeet folder
   `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` there and pick Engine =
   Parakeet. The tray's first line turns into "Speech model: …".
3. **Server** (hybrid build): on the server, `flow-server gen-token --out
   <file>` and start it bound to its VPN/LAN address (see
   `crates/flow-server/dist/config.example.toml`). On Windows: Settings →
   Developer → Host = the server's VPN/LAN IP or name (no `ws://`, no
   port), Port = 7313, Token = the file's contents (Paste), TLS only if the
   server has `[tls]` (then paste the certificate pin), Save. **Test
   connection** must say `OK · median N ms · flow-server … · parakeet`;
   **Test session** must say `EMPTY (no_speech) via server`.

### 10-minute smoke checklist

- [ ] (1 min) Hold Ctrl+Win in Notepad, say "testing one two three", release → the text is pasted; the line above the pill said "Transcribing via server…" (hybrid, server up) or "Transcribing via this PC…".
- [ ] (1 min) Stop the server (or disconnect the VPN), dictate again → still pasted; the line says "Transcribing via this PC… (server unreachable)"; Developer → Active engine row = client, fallback `connect_failed`.
- [ ] (1 min) Start the server again; hold Ctrl+Win, stop the server while speaking, keep talking, release → the whole sentence pastes and the line switched to "this PC" mid-dictation.
- [ ] (1 min) Hands-free: Ctrl+Win+Space, speak, then stay silent → it stops and pastes by itself after ~1.5 s (Settings → General → "Hands-free: stop after … s of silence"; 0 = never). Push-to-talk never stops on silence.
- [ ] (1 min) Esc while the pill is processing → nothing is pasted, no error pill; the next dictation works at once (the server is not left busy).
- [ ] (1 min) Dictate, and while it is processing Alt+Tab to another window → nothing is typed there; the pill says "the focus moved…", Ctrl+V pastes the text. Dictate into a normal field and tab into a password box before it finishes → nothing is pasted or copied.
- [ ] (1 min) Copy something, dictate → afterwards Ctrl+V gives your original clipboard. Press Shift+Alt+Z twice quickly → still your original clipboard afterwards. Shift+Alt+X then Win+V → the transcript is **not** in clipboard history.
- [ ] (1 min) Run `cmd` as administrator, hold Ctrl+Win over it and release → the "runs as administrator" pill; then type `dle` in Notepad: plain letters appear (a stuck Win key would show the desktop / lock the PC), and Start did not open.
- [ ] (1 min) OpenRouter (optional, costs cents): OpenRouter tab → paste key → Save key → Check key (credit shown) → Speech-to-text Test `OK · "And so, my fellow Americans"…`; tick "Transcribe with OpenRouter" → accept the audio consent; dictate → the line says "Transcribing via OpenRouter (openai/whisper-large-v3-turbo)…", the pill shows **C**. Tick "Clean up text with OpenRouter" → a *second* consent (text) appears.
- [ ] (1 min) Client-only build: Developer tab hidden; dictation works with the model; `netstat -ano | findstr <pid>` shows no outbound connection.

Report failures with `%APPDATA%\FlowClone\prefs.json` (no secrets in it) and the
console output (`"C:\Program Files\Murmur\Murmur.exe"` started from a
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
- [ ] Push-to-talk `Right Alt` (Settings → Shortcuts), on a layout with AltGr (e.g. Polish or German) and on US: holding Right Alt dictates and releasing pastes into Notepad; afterwards typing `s` gives `s` (no stuck Ctrl or Alt: not Ctrl+S, not an AltGr letter), the menu bar of Notepad/Explorer is not focused, and Left Alt / Alt+Tab never start dictation.
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
- [ ] Command Mode with AI cleanup ready: select "hey cant make it tmrw" in Notepad, Word and a Chrome text box, hold Ctrl+Win+Alt, say "make this more formal", release → the orange pill spins, then the selection is replaced by a formal sentence (no "Sure, here…", no quotes); Ctrl+V afterwards gives your old clipboard; History shows the instruction and result.
- [ ] Command Mode with nothing selected: "write a two-line thank-you note" is typed at the cursor.
- [ ] Command Mode with AI cleanup off and no OpenRouter cleanup: error sound, pill "Command Mode needs AI…", the selection is untouched. Esc while the pill spins: nothing is pasted.
- [ ] Command Mode in an app without UI Automation text (e.g. a terminal or an Electron editor): the Ctrl+C fallback reads the selection and the clipboard is restored.
- [ ] AI cleanup off (default): tray "AI cleanup: off", results unchanged.
- [ ] AI cleanup on without `flow_llm.dll`: tray "⚠ AI cleanup engine (flow_llm.dll) missing", dictation still pastes.
- [ ] AI cleanup on with the DLL + Qwen 0.5B GGUF: tray goes "loading…" → "AI cleanup: qwen2.5-0.5b…"; dictation works during the load; afterwards "um so i think we should uh meet tomorrow" pastes cleaned up, and History shows "AI edited".
- [ ] The NSIS installer built with `--config tauri.llm.conf.json` puts `flow_llm.dll` beside `Murmur.exe`, and AI cleanup loads on a PC without Rust/VS (only the VC++ redistributable).
- [ ] Per-app styles: defaults → a WhatsApp / Slack dictation ends without a final period, Outlook keeps it; with Personal = Very casual and AI cleanup on, WhatsApp gives lowercase without a final period; Email stays formal.
- [ ] Style per app: dictate into Notepad, open AI cleanup → *Add* lists Notepad; add it as Personal → Save → a Notepad dictation ends without a final period. An old prefs.json with `app_buckets` opens as rows and saves back with the same keys; adding the same app twice says "Already in the list".
- [ ] Cleanup level: an old prefs.json with `"cleanup_level":"none"` opens as *Verbatim* with Auto-format unticked; choosing *No AI* ticks Auto-format again; "comma" then becomes ",".
- [ ] Debug log line has `"style":"casual"` for WhatsApp and `"llm"` (`llm_skipped_clean` with AI on and an already clean dictation).
- [ ] Insights tab: after a dictation the numbers and today's bar update without reopening; Delete all in History leaves them; Reset stats (confirm) zeroes them; a first start with existing History shows its totals.
- [ ] History tab: search filters, Copy, Re-paste (Settings hides, text lands in the previously focused app), Delete, Delete all (second click), retention change prunes.
- [ ] Undo AI edit: the diff shows removed/added words; "Re-paste original" inserts the raw ASR text.
- [ ] Learn from edits: dictate "I use whisper flow daily" into Notepad, fix it to "Wispr Flow" within ~45 s, click elsewhere → tray "1 dictionary suggestion…"; Dictionary tab lists it; nothing was added until "Add word". Same in Chrome (textarea), Word and Slack.
- [ ] Scratchpad: tray → Scratchpad… (and a set *Open Scratchpad* shortcut) opens it focused; Ctrl+Win dictation lands at the caret; editing it adds no dictionary suggestion and Settings → AI cleanup → *Add* doesn't list Murmur; Quit, start again, reopen → text is there (`%APPDATA%\FlowClone\notes\scratchpad.txt`; an old `FlowClone\scratchpad.txt` from before Notes was moved there); Copy all → Ctrl+V in Notepad gives the full text; Clear → Undo clear restores it.
- [ ] Notes: tray → Notes… lists the Quick note; New note ×3 with text, Quit, start again → all 4 there with content (`%APPDATA%\FlowClone\notes\*.txt`); search finds a word from a body line; Pin moves a note to the top and survives a restart; Export… .md lands in Downloads (a second export is `(2)`); Delete twice removes the `.txt`; a set *New note + dictate* shortcut from Notepad opens a new note in front and the hands-free dictation lands in it (not in Notepad); a `.txt` copied into `notes\` by hand is listed.
- [ ] Learn from edits ignores password fields, and a chat message sent with "press enter".
- [ ] Dictionary editor: add/edit/delete rows, snippet with a multi-line body, Save; export writes `Downloads\flow-dictionary.json`; importing that file (merge) changes nothing; importing the Android app's export works.
- [ ] Dictating into silence shows "No speech heard" above the pill for ~1.5 s, with no error sound.
- [ ] (2 min) Decode while speaking (D22): with the Developer toggle on, dictate ~30 s with pauses → text appears ≈ 0.5-1 s after release; the debug line shows `segments` ≥ 2 and `pre_asr_ms` > 0. Toggle off → `segments` 0 and release → text grows with length again.
- [ ] Settings → General: "Auto-format punctuation, fillers and corrections" off → "comma" stays a word; Engine = Parakeet with the model folder in `models\` (parakeet build) → tray shows the folder name and jfk-style speech pastes.

- [ ] Debug log: Developer → tick "Debug log (includes transcripts)", dictate into Notepad → the count says 1 entry, Open log folder shows `dictations.jsonl` with `"mode":"hold"`, `"insert":{"outcome":"inserted",…}`, `"target":"notepad.exe"`; stop the server and dictate (server-first) → `"fallback":"connect_failed"`; untick → the delete prompt appears, Delete → the file is gone; dictate again → no file.

#### Developer settings / hybrid mode (needs a `flow-server` on the LAN or a VPN)

- [ ] Hybrid build: Developer tab is visible; Build row says "client + server"; header line no longer says "nothing is sent over the network".
- [ ] Host field: `ws://1.2.3.4`, `1.2.3.4:7313`, `256.1.1.1`, `my box` each show an inline error and disable Save / Test connection; `100.64.0.10`, `[fd7a:115c:a1e0::1]`, `mybox.local` are accepted (brackets dropped on save). Port 0 / 70000 rejected.
- [ ] Host `example.com` ticks and locks "Use TLS"; a 100.x / 192.168.x host shows "Unencrypted: OK only on your LAN or over an encrypted VPN.".
- [ ] Token: Show / Paste / Clear work; a 10-char token is rejected; after Save, `prefs.json` contains `token_dpapi` and **not** the token text; copying `prefs.json` to another Windows account leaves the token empty there.
- [ ] Test connection with the server up: "OK · median N ms (a-b) · flow-server 0.2.0+1a2b3c4 · parakeet" (version + commit, D26); with a wrong token: "auth_failed … token rejected"; with the server stopped: "connect_failed … is flow-server running / LAN or VPN up?".
- [ ] Test session: "EMPTY (no_speech) via server"; with the server stopped: "via client (fallback: connect_failed)".
- [ ] Both toggles on, server up: dictate in Notepad → Active engine row = server, rtt shown; with the S/L badge on, the pill shows **S**.
- [ ] `systemctl stop flow-server` (or disconnect the VPN), dictate again → text still pastes (local), row = client, fallback `connect_failed`, badge **L** with a red dot; the next dictation within 5 s shows `circuit_open` and does not wait for the connect timeout; "Reset circuit" clears it.
- [ ] Kill the server mid-dictation (hold Ctrl+Win, stop it, keep speaking, release): the whole utterance pastes, nothing from before the kill is lost.
- [ ] Allow client off + server down → error pill "Server unreachable (…)"; Allow server off → dictation never opens a connection (check with `netstat -n` / Resource Monitor); both off → "Dictation disabled — enable server or client in Developer settings".
- [ ] Changing toggles / host and pressing Save takes effect on the next dictation without restarting; the local model is not reloaded (tray line unchanged).
- [ ] Route order "Move up" (client first) with both on → dictation runs on this PC even with the server up.
- [ ] Client-only build (`--no-default-features --features local,app`): Developer tab hidden; 7 taps on the version line unlock it ("not in this build" next to Allow server); Lock hides it; `flow-windows.exe` has no ws/tls code (`cargo tree -e normal --no-default-features --features local,app | findstr tungstenite` prints nothing).
- [ ] Server-only build (`--no-default-features --features remote,app`): starts without any model file or `flow_llm.dll`, tray says "Speech: your flow-server"; dictation works through the server; Allow client is disabled ("not in this build").

#### OpenRouter (needs internet and a real OpenRouter key)

- [ ] Default build: the OpenRouter tab is shown; the client-only build has none, and its exe has no reqwest/rustls (`cargo tree … --features local,app | findstr reqwest` prints nothing).
- [ ] Upgrade from a D18 build with a Groq provider active: the OpenRouter tab has a badge and the notice "Cloud speech-to-text now uses OpenRouter only. Groq was removed and its key deleted. …"; Credential Manager no longer lists `flow-clone/cloud/groq-1`; cloud STT is off. OK hides the notice for good.
- [ ] Paste a key, Save key: Credential Manager lists `flow-clone/cloud/openrouter`; `prefs.json` contains no key text; reopening Settings shows `Key set: ••••abcd` only. Check key shows the credit limit (or the "no credit limit" warning).
- [ ] Opening the tab fetches the model list once (header "Updated …"); with the network off it says "Cached …" or "Offline list from 2026-10-24". Search "whisper", toggle ZDR only / Fast / Chat audio; pick a chat-audio model → the paraphrase warning; type a custom id `elevenlabs/scribe-v2` → Use.
- [ ] Speech-to-text Test: `OK · "And so, my fellow Americans" · N ms · $…`; with a wrong key `auth_failed — key rejected`; with the network off `connect_failed — can't reach openrouter.ai`.
- [ ] Tick "Transcribe with OpenRouter" → consent dialog "Send audio to OpenRouter?"; Cancel unticks it; accept, dictate in Notepad → text pastes, pill shows **C** (tooltip "Sent to OpenRouter"), Active engine row `cloud · OpenRouter · openai/whisper-large-v3-turbo · $0.000…`, Usage "Speech $…".
- [ ] Tick "Clean up text with OpenRouter" → its own consent dialog (text, not voice); Test cleans the sample sentence; dictate a messy sentence → Active engine row `AI cleaned · OpenRouter · meta-llama/llama-3.3-70b-instruct · $…`, Usage "Cleanup $…"; a clean sentence gives "AI not needed (already clean)".
- [ ] Turn "Zero-retention providers only" off → confirm dialog; pick a model without ZDR with it on → "Cloud STT failed (no zero-retention provider serves this model); used this PC".
- [ ] Clear key → next dictation still pastes (local), bubble "Cloud STT failed (set a key and accept the consent); used this PC".
- [ ] Monthly budget below this month's cost → "Cloud STT failed (monthly budget reached); used this PC"; cleanup falls back to this PC / the formatted text.
- [ ] Quit and restart: usage counters and the key are unchanged.

## Known gaps / v1 limitations

- OpenRouter (D28): the **C** badge appears after the commit (like S/L); while recording the line above the pill names OpenRouter and the model. Cloud cleanup never runs on dictations served by your own server (it keeps its own AI). No "Connect with OpenRouter" (OAuth) yet: the key is pasted. HTTPS uses webpki roots, so a corporate TLS-inspection CA in the Windows store is not trusted.
- Hands-free auto-stop (D19) applies to dictations *started* hands-free (Ctrl+Win+Space). A push-to-talk dictation locked later (double tap, or Space added to the chord) has no endpoint, because the router fixes `endpoint_silence_ms` at session start; stop it with the key or ✓.
- Paste target (S13): the guard compares the foreground window (HWND + process) with the one at dictation start; switching tabs *inside* the same window (browser tabs) is not detected. The password-field re-check needs UI Automation to answer within 300 ms; otherwise only the window check applies.
- Command Mode: in controls without UI Automation TextPattern the selection is read with Ctrl+C, and editors that copy the whole line when nothing is selected (VS Code) make an empty selection look like that line: the result is then pasted at the cursor, not over the line. Commands never run on your own `flow-server` (the server route has no LLM endpoint for them); answers are capped by the cleanup output budget (about 230 tokens for a short selection, 430 when generating). No saved transform presets yet.
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
- Cue sounds are synthesised tones, not Wispr's WAVs (those are not redistributable); see [Custom sounds](#custom-sounds) to use your own.

## flow-core integration

`Cargo.toml` takes `flow-core = { path = "../../crates/flow-core" }`; the crate
stays a standalone workspace (Tauri/Win32 deps stay out of the core lockfile).
`src/core_adapter.rs` maps the shell's §9 types onto flow-core's:

* `SessionConfig`: the ordered `[{word, replacement, isSnippet}]` list, languages, styles (strings → `Style`), cleanup level, raw mode, `max_samples` from the 20-min policy, `platform: windows`, `partial_interval_ms` from the live-preview pref.
* `FieldContext`: exe stem as `app_id`, URL host, `AppKind`/`StyleBucket` → `AppCategory`, before/selected/after.
* `CommitResult`: `FORMATTED`/`RAW_TRANSCRIPT` paste the core's `segments` plan (text runs, Tab/Break, trailing Enter for spoken "press enter"; `press_enter` itself is not read), history keeps `plain_text`; `EMPTY` pastes nothing (`empty_reason: no_speech` shows "No speech heard"), `ERROR` shows the error pill; `route` (router) says which engine ran.

Requests for core changes go to `docs/core-change-requests.md` (the core is
developed on another branch).
