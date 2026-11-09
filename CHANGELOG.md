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

### Added

- **Windows: download models from your server or Hugging Face** (D79). Setup's speech-model and cleanup steps and Settings → Speech to text → Download models from offer "Your server" (the default once a server is set, works with no internet) or "Hugging Face"; picking Your server in setup without one asks for its address right there; models your server doesn't have say "Not on your server". Downloads from the server are checked against the same fingerprints and resume like Hugging Face ones. When speech runs on your server, setup's model step can be skipped ("Skip — use my server only").

- **AI cleanup spells your dictionary words right** (every cleanup path: phone, PC, flow-server and AI cleanup; D64). When a dictation holds something close to a dictionary entry ("quill mate" for Quillmate, "Marbelford" for Marbleford), the cleanup is told that entry and writes its spelling when the word is meant as that name; ordinary words that only look alike stay. Only the entries close to what you said are sent, at most 20, so a large dictionary no longer makes the AI cleanup fail or slow down (with about 200 entries the whole list used to overflow the 1.5B model's prompt). A repeated word that is an entry ("bop bop" for BopBop) is no longer shortened as a stutter. On the phone, a tidy dictation with such a word now runs the on-device model instead of skipping it. Known limits: the 1.5B model often keeps a sound-alike that is itself a real word; the 3B can respell a look-alike name inside an ordinary phrase.

- **Android: speech recognition listens for your dictionary words** (D68). With at least one dictionary word, the phone's on-device English recognizer now listens for those words, as the PC does (parity English set: 27 → 30 of 37 terms right). It costs about 0.15–0.2 s per dictation, and adding the first word (or removing the last) reloads the speech model once, never in the middle of a dictation. With an empty dictionary nothing changes. Unlike the PC, the phone does not keep a second recognizer to retry a dictation that comes back empty (it would take about 0.7 GB more memory). Hebrew dictation and call transcription are unchanged.

- **Android: correct a dictation in History, and corrected names are offered for the dictionary** (D68). A dictation's page has a new "Correct the text": your fix is saved as a new version, and a name you corrected ("gina" → "Jenna") is offered straight away (Add word, Add as fix, or Not now, which keeps it in the Dictionary's suggestions); other changed words land in the suggestions as before. If the speech engine isn't loaded yet, the fix is saved and you're told its words weren't learned.

- **Android (hybrid debug builds only): English speech probe folder batch.** "Run folder batch" runs every WAV in `nano-batch/<set>/` through Gemini Nano and/or the platform recognizer and writes one line per clip and engine to a new `nano-batch/results/batch-<UTC>.jsonl` each batch; `tools/probes/wispr_parity/nano_rows.py` turns it into parity scorecard rows. Batch runs feed the platform recognizer as one segmented session, so a pause in the clip no longer ends the transcript at the first endpoint. Dictation is unchanged.

### Changed

- **Our own list of special apps, one list for the phone and the PC** (D80). The apps where the phone's bubble stays hidden (banking and payment), the browsers, the chat apps that get extra insert checks and the email / work / personal style buckets on both apps now come from one file of naming rules plus lists of well-known apps. Any app whose package name says bank (or a big bank's name) stays hidden. Fewer niche apps get a style group by default; pick one per app in Style.
- **"OpenRouter" is now "Cloud", with OpenRouter or LiteLLM as its provider** (PC; D78). The Settings tab, engine rows and switches say Cloud. On the Cloud page you pick one provider: OpenRouter (key, privacy, budget, as before) or your LiteLLM proxy (address, optional key, Test connection with model count and latency; the AI cleanup page offers the proxy's model list). LiteLLM does AI cleanup only, so picking it turns cloud speech off and the Speech page says why. LiteLLM errors read in plain words ("the server rejected the key, or needs one", "the server has no model by that name"). Setup's AI cleanup step offers Cloud with the same OpenRouter | LiteLLM switch. LiteLLM's cost is read from its `-original` cost header when the main one is missing. Saved settings and keys carry over unchanged. Android is unchanged (its custom server and OpenRouter speech still work together); its Cloud page comes later.
- **Windows setup recommends your server for speech** (D73). On a clean install the "Where should your voice become text?" step now has Your server selected and marked Recommended, with the server address, access token and Test right below; without a server, pick This PC in one click (the help line under the server fields says so). If you already use a server or OpenRouter, or you finished setup with This PC before, setup keeps that choice. Builds without the server engine still recommend This PC.
- **Windows: the Flow bar is hidden when idle on a new install.** "Show Flow bar at all times" (Settings → System) is now Off by default, so the bar appears only while you dictate (or record a meeting). Existing installs keep their saved setting.
- **flow-server: faster AI cleanup on long dictations** (D69). The server's cleanup now copies the unchanged stretches of your dictation several words at a time instead of word by word, and gets a head start on what you said while you are still talking. On the parity set the cleanup's typical time drops from about 4 s to about 2.6 s; on that set the text is identical on 12 of 13 dictations and the quality scores are unchanged. Set `[model] llm_draft = 0` to turn it off. The phone and PC are unchanged.
- **flow-server: setup is now a Docker build and one `docker run`** (D77). From the source: `docker build -t flow-server -f deploy/docker/Dockerfile .`, then `docker run` with your models folder mounted read-only at `/models` (one folder per model with its `model.toml`, e.g. a bundle's unzipped `library/`) and one volume for keys. The config is derived from the models in the folder on every start; nothing is downloaded. The admin key is written to the volume and never logged: `docker exec flow-server cat /data/admin.token` shows it. The setup guide (`docs/server-setup.md`) is a short quick start; the install without Docker, TLS, GPU and tuning moved to `docs/server-advanced.md` and `docs/container.md`. CI builds the image and checks it starts (nothing is published). The colleague install note puts Docker first.
- **AI cleanup recognises dictionary words with a number in them when you say the number** (every cleanup path: phone, PC, flow-server and AI cleanup; D67). A dictionary entry such as "Velk42" is now passed to the cleanup when you say "velk forty two", so it is spelled as in your dictionary; a different number ("velk forty three") does not count. Known limit: an entry made of an ordinary word and a number can be passed for everyday speech that happens to contain both ("win ten dollars" next to an entry Win10), as it already was when the speech model wrote the digits; the cleanup still keeps words used in their ordinary sense.
- **On-device AI cleanup writes spoken numbers, times and dates as digits** (phone and PC local model, English only, D63; the changed prompt v2 is also what flow-server's cleanup and the Windows cloud-cleanup test send). A short, tidy dictation such as "Let's meet at eleven forty on the twelfth" used to skip the model as "already clean" and kept the words. Now the model runs and writes "11:40 on the 12th", and a code read digit by digit becomes one number. "One of those", "a second ago" and "the fourth floor" stay words. Such dictations take about a second longer on the phone. Hebrew dictations are still never woken up for numbers, but the shared v2 prompt prefix changed the wording of some cleaned Hebrew dictations (accepted; Hebrew cleanup quality is tracked separately). Known regressions on the on-device 1.5B: "Don't forget that we have dinner at eight" now loses "that", and the 113-case cleanup suite goes from 91 % to 90 %.
- **AI cleanup turns dictated lists into lists and keeps only the correction** (cloud and custom-server AI cleanup, prompt v5, D60). "A few things for the trip: tent, stove, maps" becomes a bulleted list, and counted items become a numbered one, with the lead-in kept. "At 5, no, 6" becomes "at 6". An ordinary "No problem" stays; the smaller models still sometimes drop a contrast such as "Friday, not Monday". This change left the on-device and flow-server cleanup prompt (v2) as it was (D63 later changed v2 for spoken numbers). Known issue: with v5, spoken "new line" / "new paragraph" and spoken punctuation ("comma", "period") come out worse on smaller models (on the 3B, a "new paragraph" followed by "A or B" can become a bulleted list).
- **A dictated "Here's a list...:" lead-in is kept** (every cleanup path: on-device on the phone and the PC, flow-server and AI cleanup). It used to be stripped as a model preamble.
- **The keyboard active when you stop dictating decides the language** (Android and Windows, with the language following the keyboard). Switch from English to Hebrew mid-dictation and the whole recording is transcribed in Hebrew; on the phone the other speech model loads first if it isn't loaded (the bubble says so), and with *Keep both languages loaded* nothing loads. The same keyboard throughout works as before. The debug log line gains `start_language` and `stop_language` (D58).
- **Windows: hold Right Alt to dictate.** Right Alt is the new push-to-talk key instead of Ctrl+Win, and it starts at once like any push-to-talk key; if you never changed Ctrl+Win, it moves to Right Alt on the next start, so Ctrl+Win alone no longer dictates (set it back in Settings → General → Shortcuts if you prefer it; a shortcut you picked stays). While Right Alt is your push-to-talk key, no app sees it: it never opens a menu bar, and on keyboards where Right Alt is AltGr (Hebrew, many European layouts) it no longer types AltGr letters; type them with Left Ctrl+Left Alt instead. The tray tooltip names your push-to-talk key. Hands-free (Ctrl+Win+Space) and Command Mode (Ctrl+Win+Alt) are unchanged (D57).
- **AI cleanup keeps what you dictated in parentheses, quotes and formulas.** The safety check that threw away the AI's cleaned text, and pasted the uncleaned text instead, whenever the result had words or marks it could not trace back to your speech is gone on every path (phone, PC, flow-server, cloud). It rejected correct parentheses, quotes and math. The uncleaned text is still used when the AI is too slow, fails or answers with nothing. The debug log no longer has a `rejected_text` field. Upgrade flow-server too: dictations it cleans (server route or server cleanup) run its own core, and an older server still applies the check (D59).
- **AI cleanup: the cloud section is titled "Cloud server".** On Windows its title no longer switches between "OpenRouter" and "Custom server" as you pick the provider: the "Send text to" list names it. On Android the cloud page is titled "Cloud server" and its cleanup switch reads "Clean up text with the cloud server" whichever provider is picked.

### Fixed

- **Setup no longer stops on `bad_config: invalid custom cleanup model id ""`** (PC and phone; D76). Settings kept across an uninstall with "Cloud server" picked for AI cleanup and no model named yet made 1.1.2 refuse every save, so setup's "Where should your voice become text?" step could not continue. An empty model means the cloud server is not set up yet, and nothing is refused. Every refused setting now reads as a sentence without the `bad_config:` code, for example "The AI cleanup server's model name "two words" is not valid: …" or "The server port must be 1-65535.".
- **Cleanup no longer repeats text that is already in the field** (every cleanup path: phone, PC, flow-server and AI cleanup; D71). The cleanup sees the text around the cursor and sometimes copied it into its answer. With "Gate code: " already typed and only the number dictated, the field read "Gate code: Gate code: 4719". Words at the start of the answer that repeat the text before the cursor, or at the end that repeat the text after it, are now dropped, unless you plausibly said them yourself: a spoken number the cleanup wrote as digits, a name it corrected, or words with an "um" between them all stay.
- **No more stray "Yeah." at the end of a dictation** (PC, phone and flow-server, with "Decode while speaking"; D72). In a quiet room, the click of the key that stops a dictation could end up alone in the last piece. That piece was then made up to 100 times louder on its own, and the speech model wrote a word for it, usually "Yeah.". A piece is now only made louder when the dictation's typical speech is quiet, so a click after normal speech stays quiet; a dictation that is quiet all the way through is still raised, even with one cough or desk bump in it. A recording with nothing in it but the taps that start and stop it is now empty instead of "Yeah.".
- **No more stray capitals mid-sentence after a pause** (PC, phone and flow-server, any speech model, with "Decode while speaking" on; D70). Pausing in the middle of a sentence could leave the next word capitalised ("we should look at Options for the room", "it went fine, The next part"), mostly when AI cleanup did not run. The words around each pause now take their case and punctuation from the decode that heard both sides; names stay capitalised.
- **Windows setup: "Now try it" says why it can't work instead of doing nothing** (D75). On a clean install with no speech model on this PC and no server address, holding Right Alt in the last setup step showed nothing in the step (the reason only reached the pill). The step now checks before you talk and says what is missing in plain words for the engine you chose (no speech model yet, no server address, your server can't be reached), clears that once it is ready, and while you hold the shortcut it shows listening, working, and any error from the pill.

- **Whisper no longer repeats a stretch of text when "Decode while speaking" is on** (PC and phone whisper models, flow-server whisper; D66). A short piece decoded with the trimmed encoder window could loop a long phrase with a few words between the copies; that now counts as degenerate and is decoded again with the full window. Parity set, whisper small: segmented WER 20.9% → 18.3% (unsegmented 16.8%).
- **Windows: with cloud cleanup switched off in Cleanup order, the "Try it" row says so.** With a key saved but OpenRouter (or your custom server) switched off in Cleanup order, it showed "cloud_cleanup_not_configured — … save a key". It now says cloud cleanup is off in Cleanup order and has a "Turn on in Cleanup order" button.
- **Windows: the "Try it" row words every other cause too, without backend codes.** It tells apart a missing key, missing consent and a custom server without its address or model, and a failed Test names the cause in words.
- **Hebrew dictations stay Hebrew after AI cleanup** (phone and PC local model, and flow-server cleanup). The on-device model used to turn long Hebrew dictations into English. Its prompt now shows it a Hebrew example when you dictate in Hebrew (by the speech model's language, or by the letters when it gives none), and an AI answer that comes back in another alphabet than your dictation (Hebrew, Arabic, Cyrillic, Greek) is not used: the text is pasted without AI cleanup instead (`llm_language_changed` in the debug log; every cleanup path, D65). English dictations are unchanged. A long Hebrew dictation can take the local model longer than its time limit, and then it is also pasted without AI cleanup. Upgrade flow-server too: it builds the prompt for the dictations it cleans. Measured with the 1.5B local model and a 20 s time limit on a PC; not yet measured on the phone (3 s limit), with flow-server's model, or for cloud AI cleanup.
- **Windows: picking "Custom server" is saved at once.** It used to be saved only once both the server address and the model were filled in, so saving the key in between reloaded the settings, set the list back to OpenRouter and emptied the address (dictation kept using OpenRouter). Now the list always shows what dictation uses; until the address and model are set, cleanup skips your server and the next engine in the cleanup order cleans, and while cloud cleanup is on, its row in the cleanup order shows "Enter the address" or "Pick a model".
- **Windows: "Load models" / "Test connection" with a saved custom-server key no longer fail with a bare 401.** The saved key only goes to a server you agreed to send text to; for a server not agreed to yet the page now asks first and remembers the answer, instead of calling it without the key.
- **`flow-cli transcribe` keeps the whole clip.** A WAV longer than 30 s lost everything past the first 30 s (counted as `dropped_samples`), because the replay fills the session's audio buffer faster than the core empties it. The buffer is now sized to the clip. Live dictation is unchanged.

## [1.1.2] - 2026-11-05

### Added

- **Android: record while the speech model loads.** Right after the app starts, or after switching the keyboard between English and Hebrew, the bubble starts recording at once instead of saying the model is still loading. It shows which model is loading, and your words are transcribed on the phone as soon as it is ready, never sent to your server for this (X still discards it). If the model takes more than 2.5 minutes or fails to load, the recording stops, the bubble shows why, and the recording waits in History for "Retry with current settings".
- **Android: "Keep both languages loaded" (Settings → Speech to text, off by default).** With the Hebrew model and a default model installed, both stay in memory (up to about 1.1 GB, depending on the models) so switching the keyboard between English and Hebrew never waits for a model to load; takes effect at the next dictation.
- **Windows: AI summary and action items for a meeting (Notetaker, still hidden).** Made after the transcript on your AI cleanup order (this PC, your server, OpenRouter with consent), in the meeting's language, long meetings in chunks; a Regenerate button on the meeting's page. flow-server advertises the `summary` feature; older servers are skipped (D53).
- **AI cleanup through your own server (LiteLLM / OpenAI-compatible).** Windows and Android (hybrid) can send AI cleanup to a server you run instead of OpenRouter: base URL (http or https), optional key, model from its list or typed; cloud speech-to-text stays on OpenRouter; its cleanups and any reported cost show on their own "Custom server" usage line, outside OpenRouter's spend and budget (D52).

### Changed

- **Android: APKs from CI and from GitHub releases install over your own build.** They are now signed with the same debug key as the owner's local builds, so updating between them keeps your settings, dictionary, history and models instead of needing an uninstall (D56).
- **Faster model loads.** The Hebrew Zipformer model loads about 3× faster on a laptop (6.7 s → 2.2 s; the phone, where it took 16.6 s, is not measured yet), and on Android a keyboard switch between English and Hebrew no longer reloads the AI cleanup model (~38 s on the phone). With the debug log on, each load's per-model times go to it.

### Fixed

- **Side panel names the engines that will clean up your text.** Android's drawer said "Off" when only your server (or only a custom server) cleaned; it now lists the engines in the order cleanup tries them, skipping a server with no AI model, and Settings → AI cleanup no longer names a custom server that is not set up. Windows' Engines card names a custom server instead of "OpenRouter".

## [1.1.1] - 2026-11-04

### Changed

- **Windows: Notetaker hidden again behind its coming-soon page; recordings kept.** No tray line, shortcut, call detection or background meeting job; meetings already on disk are left untouched (D51).

## [1.1.0] - 2026-11-04

### Added

- **Docs: moving to a network with no internet.** `docs/air-gapped.md` gains a transfer checklist, every model to download (pinned, checked; list in `docs/air-gapped-models.txt`) and the server config to use them; review in `docs/findings/2026-11-04-air-gap-readiness.md`.
- **Server: every connection's end is logged.** Each WebSocket connection
  now leaves one `disconnected` line when it ends, however it ends, with the
  key id, client version, duration, control messages, audio bytes received,
  sessions, results, errors, cancels and cleanups, the session still open
  (if any), why it ended (`client_closed`, `idle`, `dead_peer`, `no_hello`,
  `proto_mismatch`, `socket_error`, `not_reading`, `shutdown`, `panic`) and
  an outcome (`errors`, `result`, `cleaned`, `no_audio`, `no_result`; the
  worst wins). Each session that ends without a result also gets one
  `session ended` line with its reason (`cancelled`, `replaced`,
  `too_large`, `wall_clock`, `busy`, `worker_failed`, `disconnect`),
  duration and audio bytes. Counts and timings only, never text; the client
  string is quoted and cut to 64 characters. These replace the "session
  ended by disconnect", "cancelled" and wall-clock lines.
- **Model catalogue: Parakeet unified and the recommended whisper files.**
  The Windows and Android catalogues gain Parakeet-unified-en 0.6B int8 (the
  phone's default English model) and whisper base.en / base / tiny.en /
  small.en q5_1 (the files Android recommends), each pinned to its SHA-256,
  so a server library copy tagged with these ids is checked against the
  built-in fingerprints. One shared list of catalogue ids
  (`flow_protocol::library::CATALOGUE`) is tested against both apps.

- **Android (hybrid debug builds only): English speech probe.** Developer
  settings → Diagnostics → **English speech probe** compares Google's
  Gemini Nano speech (ML Kit GenAI, advanced or basic mode) with the
  platform on-device recognizer (biasing terms, formatting), from the mic
  or a synthetic TTS / picked WAV clip, and shows status, download,
  partials, latency and every error code. An overlay runs the same
  engines over another app, to learn whether Gemini Nano refuses when
  Flow Clone is not on top (`BACKGROUND_USE_BLOCKED`). Each run is one
  line in `files/probe-english.jsonl`. Dictation is unchanged
  (`docs/findings/2026-11-02-pixel-offline-dictation.md`, PQ-a / PQ-d).

- **Hebrew models scored on the owner's own dictations (H7).** On 16
  clips zipformer-he scores 13.6–14.4 % WER against parakeet-he's 18.9 %,
  answers in about 50 ms after the last audio instead of 420 ms, and the
  phone matches the host, so zipformer-he stays the Hebrew default and
  parakeet-he stays selectable
  (`docs/findings/2026-11-02-h7-hebrew-stt-scores.md`).

- **Android: transcribe new call recordings automatically.** The Calls
  page has a switch, **Transcribe new call recordings**. Once it is on (it
  asks for access to audio files), every call recording Samsung saves
  afterwards is transcribed on the phone in the background, one at a time,
  and never while you dictate; a recording you pick yourself goes first. A
  notification says when a transcript is ready. Recordings from before the
  switch was turned on are left alone, a recording is never transcribed
  twice, and the recordings themselves are only read. Failures are listed on
  the Calls page, where you can try them again. The setting stays on after a
  restart (D49).
- **Windows: a meeting shows where it was transcribed.** An open meeting's
  details line now ends with "Transcribed on: This PC" (or Your server /
  OpenRouter; with several, how many parts each one did), stored with the
  meeting. The debug log has one line per transcribed part with the route
  and engine (never the text) (D47).
- **Windows: choose where meetings are transcribed.** Settings › Speech to
  text has a third ordered list, **Meetings**, built like Engines and the
  AI cleanup order: this PC, your server and OpenRouter, each moved up or
  down and switched on or off. Recorded meetings are transcribed by the
  first one that is on and answers, and fall back down the list when it
  is unreachable; dictation keeps its own order. Until you change it, the
  list is the same as your dictation engines. This PC's model is loaded
  for a meeting only when this PC is first or the engines above it fail.
  OpenRouter needs the same audio consent as for dictation (D47).
- **Windows: Notetaker notices Zoom, Webex and Jabber calls.** On the
  Meetings page, choose what happens when a call starts: **Ask me** (a
  small "Record this meeting?" box in the corner) or **Start recording
  automatically** (the recording chip shows on the Flow bar). The
  recording stops by itself about 12 s after the call ends; a recording
  you started yourself is never stopped by it. Off until you turn it on;
  while on it looks every 3 s at which of these apps shows a meeting
  window or uses the microphone, and does nothing more while none of them
  runs (D48).
- **Windows: Notetaker records meetings (Meetings page).** Start and stop
  a recording from the tray, the Meetings page or an optional shortcut
  (Settings › General). Your microphone and what the PC plays are
  recorded as two separate tracks; a blinking chip on the Flow bar shows
  the recording. When you stop, the meeting is transcribed into turns
  labelled **Me** (mic) and **Others** (PC sound) in time order, on its
  own engine and only between dictations, so dictating during a long
  transcription is never held up. The Meetings page lists meetings (date,
  length, app) and opens, copies, deletes or retries them. The audio of
  the last 3 meetings is kept; older audio is deleted, transcripts stay
  (D45).
- **Meeting transcription (minutes to hours).** A recording of any length
  is cut at pauses into chunks of 8-30 s, and each chunk runs as one normal
  dictation on the route order with its fallbacks. The result is a
  transcript of timestamped segments (start, end, text). Only the open
  chunk is held in memory, silence is skipped without being transcribed,
  and progress and cancellation are exposed. Hebrew and English follow the
  dictation language setting. This is the shared engine for the coming
  Windows Notetaker and Android call transcription: `flow_core::meeting`,
  with `flow_router::meeting` for the router (D44). `flow-cli meeting
  file.wav` prints the timestamped transcript of a long WAV and streams it
  from disk (`--json`, `--progress`; `--router` with the `remote` feature
  adds a server route).
- `flow_core::audio::WavBlocks` / `Resampler`: read a WAV of any length
  block by block as 16 kHz mono.
- **Android: Calls.** A new page in the drawer (on-device builds: local
  and hybrid) transcribes a call recording you pick: Samsung's m4a call
  recordings, WAV and anything else the phone can decode. It runs on the
  phone only, in Hebrew or English, with progress and Cancel in a
  notification, and keeps going with the screen off. The recording is only
  read, never changed or deleted; the transcript is kept on the phone and
  can be opened, copied or deleted. Dictation always comes first: the call
  job uses its own copy of the speech model and pauses between chunks
  while you dictate (D46).

### Changed

- **Keys: "Last seen" is now "Last used" and "Last connected".** Last used
  counts only dictations, text-only cleanups and model downloads; last
  connected is any request, so opening Users & keys or a connection check
  no longer looks like use. Shown in the Windows and Android Users & keys
  pages and `flow-server keys list`. An older `key-usage.json` loads as is: its last seen becomes last
  connected, and last used shows "unknown" (app) / "-" (CLI) until the key
  is next used. The admin API still sends `last_seen` for older apps (D50).
- **AI cleanup always runs on your server or OpenRouter.** The "already
  clean" skip ("AI not needed") now applies only when the phone's or PC's
  own small model would clean first, since that model damages tidy text.
  A tidy dictation whose first cleaner is flow-server or the cloud is
  cleaned, including the first one after the server drops (the cloud next
  in line takes it); it never falls through to the on-device model. The
  server cleaning its own sessions no longer skips either. Phone AI stays
  off by default (D43).
- **Asides, degrees and times read like Wispr's.** "my bike in parentheses
  which is the green one is locked outside" → `My bike (which is the green
  one) is locked outside`: an "in parentheses which/who …" aside spoken
  without pauses now closes right before the sentence goes on ("write the
  date in parentheses which …" and other literal uses stay words).
  "20 degrees" → `20°` ("-3°C", "350°F"; "degrees of freedom", "two
  degrees in history" stay words). Times use lowercase am/pm (`7 am`,
  `6:30 pm`, was `7 AM`), also when the speech model spells the letters
  ("7 A M"). The AI cleanup prompts aim at the same text and add a comma
  before an "and" that opens a clause with a time or place ("…, and at
  7 am the heater …").

### Fixed

- Windows: the AI cleanup order (D36) now applies to dictations
  transcribed on this PC. With this PC's AI model off, cleanup went no
  further than this PC, so your server and OpenRouter never cleaned a local
  dictation: the model slot ignored the router's cleanup plan. It now keeps
  the plan and puts it back on every model it loads (speech model change,
  on-demand load, AI model on or off). The speech route is unchanged.
  What you will notice: with server cleanup on, this PC's dictations are
  now cleaned by the server even when server speech is off (one server
  connection per dictation, text only). With this PC's AI model on and
  first, a dictation transcribed on the server is now cleaned on this PC
  (the server skips its LLM), as the order says.

- Windows: a meeting recording started while the previous meeting's
  transcription was still tidying up old audio could lose its WAVs (they
  were taken for orphans, because the new meeting file appeared after the
  clean-up listed the meetings). A WAV is now an orphan only if its meeting
  file is missing when the clean-up looks at it, and a transcription is
  reported finished only after its clean-up.

- **Server library: a `catalogue_id` alone is enough.** `/v1/models` fills a
  missing `engine` from the catalogue entry, so the apps no longer mark such
  a model "Not usable". The "no engine" message now lists every engine,
  `sherpa_online` included. A test now covers tag edits being picked up by
  the rescan without a restart.

- **`tools/phone/call-decode.sh` works on Android 17.** `exec-in`/`exec-out`
  skip the device shell there, so the push silently did nothing (exit 0) and
  the hash check failed with "got sha256sum:". The file now goes through
  `/data/local/tmp` and `shell` + `run-as`, and a failed hash prints the
  phone's own output.
- Windows: CI builds no longer call themselves "-dirty" (1.0.0's installers
  reported e.g. `a24090a-dirty`). The installer step's `tauri build`
  rewrote `apps/flow-windows/Cargo.toml` (it adds `features = []` to a
  `tauri-build` entry without one) just before building the shipped exe;
  the entry now has it. CI also checks the tree after the installers.
- Windows: with meeting detection off, the app now does no detection work
  at all. Before, it still ran one empty look at startup (a
  `meeting-detect: signals: no meeting app running` line in the debug log)
  and checked the recorder every 3 s; now the detector only takes down a
  "Record this meeting?" box when you turn it off mid-call (D48).
- Android: the floating bubble no longer gets stuck on a red error while the speech model loads. A tap during the load now shows a pulsing "…" (still loading, with a toast naming the model) instead of the error "!", and the bubble goes back to normal by itself once the model is ready, or shows the real error if the load fails. Each tap checks again: ready starts dictating, still loading says so again, and only a real engine error opens the app. Before, the error stayed for good and every tap opened the app.
- Android: switching the keyboard language while the Hebrew model loads (D37) now loads the model the new language needs. The running load can't be interrupted, so its model is dropped when it finishes, freed before the next load (still one model in memory), and never becomes the active engine. A dictation meanwhile says which model it waits for ("Loading the default speech model…"), not the Hebrew one. Switching back from a loaded Hebrew model now names the default model too, instead of a generic "still loading".
- Android: Settings ▸ Accessibility now lists the service as "Murmur" (it showed "Effortless on-device voice dictation", so it was hard to find). The hybrid and remote builds no longer claim "no internet permission" in the accessibility description.
- flow-server `install.sh`: an upgrade with `--sherpa-lib` pointing at the install's own lib dir no longer stops halfway with "are the same file" (libs that are already the installed files are kept), and without `--sherpa-lib` an upgrade now uses the installed libs. The binary and libraries are staged first and renamed into place together, the binary last, so a failed copy leaves the previous binary and libraries in place instead of a new binary next to old libs.
- Quiet speech (around -60 to -50 dBFS, e.g. a soft voice or a low-gain mic) is no longer dropped as "no speech" or returned blank, on every platform (shared `flow-core`). The speech detector now measures speech against the recording's own noise floor when that floor is very low, and quiet speech it accepts is raised in level before the speech model, whose own silence check used to blank it. Silence and noise alone are still skipped. On the 49 Hebrew FLEURS bench clips, the 15 that came back empty on the phone now reach the model (21 % WER on them in a host-side check).
- Command Mode and transforms (Android and Windows) keep Hebrew text in Hebrew. Before, an instruction such as "make it more formal" on selected Hebrew text usually came back as an English rewrite on the on-device model (9 of 10 synthetic cases on Qwen2.5-1.5B; now 0 of 10). The prompt now tells the model to answer in Hebrew and shows a Hebrew example. If the answer still comes back in another script, the app asks once more, and if that answer is also in another script it leaves the selection unchanged with "AI changed the text's language". Asking to translate ("translate this to English", "תתרגם לאנגלית") still changes the language. The same lock applies to Arabic, Cyrillic and Greek text.
- OpenRouter cleanup is no longer skipped as "AI not needed (already clean)" when your server comes before it in the AI cleanup order but cannot clean (unreachable, or no AI model). Since 0.11.0 the default order (phone → server → cloud) with the phone AI off made the server the first engine. A tidy-looking dictation then hit the phone's "already clean" check, and nothing cleaned it. OpenRouter also got the formatted text with the older prompt instead of the raw transcript. When the server's circuit is open or it has no AI model, OpenRouter now counts as first: it gets the raw transcript with no skip, as when it is first in the order. (Shared `flow-core`/`flow-router`, so Windows too.) Finding: `docs/findings/2026-11-02-local-cleanup-skipped.md`.
- Windows: whisper on the CPU is built optimised again. Through 1.0.0 the CI built whisper.cpp without optimisation (MSVC `/Od`), so on the CPU it was 6–18× slower than whisper.cpp's own build: about 4.5 min for a 10 s Hebrew clip with ivrit turbo, and 10.6 s instead of 1.6 s with the base model. This hit the app's built-in whisper (English on every PC without a GPU, and Hebrew whisper without a Vulkan driver) and the GPU plugin's CPU fallback. The cause was the Rust `cmake` crate, which drops CMake's Release flags with the Visual Studio generator. CI now passes `/O2 /Ob2 /DNDEBUG` itself and fails the build if a whisper.cpp project is not compiled with `/O2`. On the test laptop, a 10 s Hebrew clip on the CPU now takes 18 s instead of 267 s (whisper-cli: 15 s), and with the base model 1.1 s instead of 10.6 s. The portable AVX2 baseline is unchanged. Finding: `docs/findings/2026-11-02-win-cpu-whisper-speed.md`.
- Android debug log: a dictation that got no AI cleanup at all now says why in a new `llm_off` field (`ai_cleanup_off`, `no_phone_model`, `phone_model_failed` or `no_engine_available`), shown in the in-app viewer as "no AI cleanup (…)". Before, the line only had `llm: null`.
- Windows: Settings → Speech to text says why a whisper Hebrew model runs on the CPU, instead of "no graphics chip found" on PCs that have one. The reasons are: the CPU is forced (`FLOW_WHISPER_DEVICE=cpu`), the graphics chip plugin (`flow_whisper_vulkan.dll`) is missing, no Vulkan graphics chip was found, or the graphics chip failed (with the short error). The not-yet-downloaded hint uses the same reasons. The CPU line now says "expect slow dictations" instead of "several seconds". Finding: `docs/findings/2026-11-02-vulkan-he-laptop-check.md` (bug 2).

## [1.0.0] - 2026-11-01

### Added

- **Windows: Hebrew on the graphics chip with ivrit.ai Whisper turbo.**
  Settings → Speech to text → Hebrew model offers ivrit.ai Whisper turbo
  (574 MB, Apache-2.0), the most accurate Hebrew model (about 20 % word
  errors against Zipformer's 27 %). Whisper now runs on the GPU through
  Vulkan (new `flow_whisper_vulkan.dll` in the installers, D42): about 1 s
  per dictation on a laptop's integrated GPU. Without a usable GPU or
  Vulkan driver the app falls back to its CPU whisper, where this model is
  slow (about 13 s), so Settings recommends it only when it finds a GPU
  (Zipformer otherwise) and says where Hebrew runs: the graphics chip's
  name, or the CPU and why. It downloads from Hugging Face, from your
  server's library, or imports from a file like the other models; the
  default whisper model also uses the GPU when there is one.
  `FLOW_WHISPER_DEVICE=cpu` forces the CPU.

- **Guide: running Flow Clone with no internet (`docs/air-gapped.md`).**
  One short owner page for a closed network: install the server (host
  build or the Docker image moved with `docker save` / `docker load`),
  copy and tag models into its library, create the admin key on the
  command line and the other keys from the app's Users & keys, set up the
  Windows and Android apps so they download their models from the server,
  copy models by hand when there is no server, and the network-free local
  Android app. Linked from the README, the Android install guide and the
  server setup guide.
- **Air-gapped end-to-end check (`tools/e2e/airgap/run.sh`).** Builds
  flow-server from the checkout and runs it on loopback with a temp state
  dir and model library, then drives it with the apps' own client code: an
  admin key from the CLI, a user key from the admin API, a library model
  downloaded and its SHA-256 checked, one dictation (public clip), the
  user's dictations and downloads in the admin's key list, then revoke and
  the key refused on every route. Both processes run under strace and the
  check fails on any connection to a non-loopback address (in a loopback-only
  network namespace too, where the system allows unprivileged ones).
- **Server: Docker image, Compose file and Helm chart.** Run flow-server
  in a container with your own models: a locally built CPU image (with
  Parakeet, whisper, TLS and LLM cleanup) and an NVIDIA CUDA variant that
  runs LLM cleanup on the GPU. Models, keys and config are volumes, so
  nothing is baked in and keys survive restarts and rebuilds. On first
  start the server waits until you add the admin key (`docker compose exec
  flow-server flow-server keys add owner --admin`, or `kubectl exec` with
  the chart). It runs as a non-root user on a read-only filesystem and
  keeps the server's LAN/VPN-only rules: it binds the container's own
  address instead of `0.0.0.0`. The image can be built offline from host
  binaries and moved to air-gapped hosts with `docker save` / `docker
  load`. Nothing is published to a registry. See `docs/container.md`.
- **Windows: with a server set, models download from its library instead
  of Hugging Face (D41).** Settings → Speech to text → From your server
  lists every model your server serves (speech to text, Hebrew, AI cleanup,
  language detection) with its size and what it is for; Download (after
  the usual confirm dialog) installs and selects it, Use switches to one
  already downloaded. Built-in models are checked against the fingerprints
  in the app, models you added against the fingerprint your server lists.
  An interrupted download resumes where it stopped. While a server is set
  the app never contacts Hugging Face; models the app can't load (a
  voice-activity model, an unknown engine) are listed with the reason.
- **Android (hybrid): download models from your own server (D41).** Each
  model screen (Speech model, Parakeet, Hebrew model, AI cleanup) gains
  "From your server": tap Show models on your server to list what the
  server's library has for that slot, then Download (the confirm dialog
  shows the size, since you may be on mobile data). The model lands in the
  same place an import would and is selected; Use switches to one already
  downloaded. Built-in models are checked against the fingerprints in the
  app, models you added against the ones your server lists. An interrupted
  download keeps what arrived and Resume continues from there; Cancel keeps
  nothing. Hebrew models you add yourself (streaming or Parakeet) now work
  too. Import .zip / Import from file stay. Nothing ever comes from the
  internet, and the local build has no download code or network permission
  at all (D9 amended).
- **flow-remote: model library client (D41)** shared by the apps: list the
  server's models and download a file resumably (HTTP Range) with its
  SHA-256 checked, over the same TLS pin and key as dictation.

- **Server: model library, so the apps can get models from your own server
  (D40).** Copy a model folder into the server's library folder (default
  `<state dir>/library`, or `[library] dir`) and tag it with a small
  `model.toml` (kind, title, optional language, catalogue id and engine);
  the server lists tagged models with each file's size and SHA-256 at
  `GET /v1/models` and serves the files, with resume (HTTP Range), on the
  same port and TLS as dictation, to any valid key. New and changed folders
  are picked up without a restart; `flow-server library list` shows what is
  served and why a folder was skipped. Downloads count toward the key
  (`keys list` → DOWNLOADS). The Windows and Android apps download from it (D41).
- **Android (debug builds): on-device benchmark.** A shell-only
  `BenchActivity` measures STT (WER/CER, latency, RTF) and AI cleanup
  (88-suite score, LLM latency) on the phone. It reads the models the app
  already has, in place, and runs them through the app's own engine code.
  Fixtures are public clips; the results are scored on Linux
  (`tools/probes/android_bench/`). Release builds do not contain it.
  (`keys list` → DOWNLOADS). App-side download comes in later releases.
- **Windows: manage the server's keys from the app (D39).** With an admin
  key saved in Settings → Your server, a new **Users & keys** page lists
  every key with its last use, app, dictation count and audio minutes,
  creates a key (name, admin switch; the secret is shown once in a copy box
  and kept nowhere) and revokes one after a confirm, with a warning when it
  is the key this app uses. The page is hidden for non-admin keys.
- **Android: manage the server's keys from the phone (D39).** In the
  hybrid and remote apps, with an admin key saved, Settings → Your server
  shows **Users & keys**: every key with its app, last use, dictations,
  audio minutes and model downloads; create a key (name, admin switch; the
  secret is shown once with a Copy button that marks the clip sensitive,
  and is never stored or logged) and revoke one after a confirm. The last
  admin key can't be revoked. Hidden for non-admin keys and in the
  on-device-only app.
- **Server: admin API (D39).** `GET /v1/whoami`, `GET`/`POST
  /v1/admin/keys` and `DELETE /v1/admin/keys/{id}` on the usual port, with
  the usual key; non-admin keys get 403. The server refuses to revoke the
  last admin key, from the API and from `flow-server keys revoke` (which
  gets `--force` to override). The CLI and the running server can now both
  change keys at once safely (a lock file). The client is in the shared
  `flow-remote` crate, ready for Android.
- **Server: one key per device instead of one shared token (D38).** On the
  server, `flow-server keys add <name> [--admin]` creates a key and prints
  its secret once, `keys list` shows each key's last use, app version,
  dictation count and audio minutes, and `keys revoke <name>` locks out one
  device at its next connection without touching the others or open
  sessions. Changes apply without restarting the server. Only a hash of each
  secret is stored. Upgrading needs nothing: the existing token becomes the
  admin key `owner`, and the apps keep working with it unchanged.
- **Windows: import a model from a folder, .zip or file (no network).**
  "Import from file…" next to each downloadable model in Settings → Speech to
  text (default and Hebrew models) and AI cleanup installs that catalogue
  model from files copied to the PC, for air-gapped PCs and client-only
  builds without the downloader. The dialog lists the files the model needs;
  each is checked against the same pinned size and SHA-256 as a download and
  goes through a `.part` file, and only when all of them match is anything
  moved into place (a mismatch installs nothing). The imported model is then
  selected exactly as after a download. The Hebrew models' language-ID file
  is optional and can be imported later on its own. Other or custom model
  files are refused.

### Changed

- **"quote unquote" at the start of a sentence quotes the whole sentence.**
  Mid-sentence it still quotes one word ("she called it quote unquote cozy
  and left" → `she called it "cozy" and left`); at the start of a
  dictation or right after a sentence end or line break it now quotes up to
  the sentence end ("quote unquote the new menu is great" → `"The new menu
  is great."`). Same scope in the fallback rules, both AI cleanup prompts
  (local, server and cloud models) and the guard, which now rejects an AI
  answer that quotes the wrong words (the rest of the sentence mid-sentence,
  or one word at a sentence start).

### Fixed

- **Hebrew (zipformer) no longer suspected of dropping first words.** The
  13 s FLEURS clip that seemed to lose its first three words on the server
  has every word; the speaker reads them last, and the output has them
  there. Leading padding, VAD trimming and endpointing were checked and
  are fine; a regression test now covers that clip and a soft lead-in
  before speech (`docs/findings/2026-10-31-zipformer-first-words.md`).

- **Windows: the Users & keys page may call the server.** Its four
  commands were missing from the settings window's permissions, so Tauri
  refused them; the capabilities test caught it.

- **Server: upgrading with `install.sh` keeps language detection working.**
  The installer now builds with whisper (language ID, Silero VAD), reuses
  the installed model when no `--model` is given, and `--start` restarts a
  running server, so a re-install no longer stops a server whose config
  sets `lid_path`. `--no-whisper` opts out unless the config needs it.
- **Server: AI cleanup no longer times out on the first or a long
  dictation.** The LLM is warmed at startup, and its time budget grows
  with the transcript (`llm_timeout_per_char_ms`, default 15 ms per
  character, capped by `llm_timeout_max_ms`, default 8 s).
- **AI cleanup notices when your server gains an AI model, without a
  restart.** After a server upgrade that added AI cleanup, Settings → AI
  cleanup kept saying the server "has no AI cleanup model: skipped" and the
  server was never asked, until the app restarted or "Test connection" ran.
  A "no AI model" answer is now believed for the health interval only
  (`server.health_ttl_s`, 30 s): after it the app checks the server again
  once in the background, and the next dictation tries it. A server that
  has AI cleanup costs no extra connections. The Windows AI cleanup page
  re-reads the status every 10 seconds while it is open.

## [0.12.0] - 2026-10-31

### Added

- **Dictation language follows the keyboard (D37).** New language option
  "Keyboard language", now the default: each dictation uses the language of
  the keyboard you type with. A Hebrew keyboard (Gboard on Android, a Hebrew
  layout in the app you dictate into on Windows) means Hebrew, so the Hebrew
  model is used when it is installed, without changing the app's language.
  An unknown keyboard language falls back to the old behaviour: the phone
  language on Android, auto-detect on Windows. Users on Phone language
  (Android) or Detect automatically (Windows) move to it once; a language
  you picked stays. On Android a switch to or from Hebrew reloads the speech
  model ("Loading the Hebrew speech model…"). Settings → Language shows
  "Keyboard language · now: Hebrew"; the debug log line gains
  `language_source` (keyboard / phone / fixed; Windows: keyboard / fixed /
  auto, plus `language`).
- **Android: "Hebrew model" on Settings → Speech to text.** The Hebrew model
  import was only reachable through the model row; it now has its own row
  on that page.
- **Windows: AI cleanup on the graphics chip.** The installers now ship the
  AI cleanup engine twice: `flow_llm_vulkan.dll` (GPU through Vulkan) and
  `flow_llm.dll` (CPU). The app loads the Vulkan one first. On a PC without
  a Vulkan driver it loads the CPU one instead. With no usable GPU, or a GPU
  that fails to load the model, the Vulkan one runs on the CPU.
  `FLOW_LLM_DEVICE=cpu` forces the CPU. Settings and the tray say where
  cleanup runs. Both builds keep the portable AVX2 baseline (`GGML_NATIVE=OFF`).
- **Windows: Llama 3.2 3B Instruct for AI cleanup.** Settings → AI cleanup →
  On this PC can download Llama 3.2 3B Instruct Q4_K_M (2.0 GB, pinned
  revision and SHA-256). The row recommends it when cleanup runs on a GPU,
  and Qwen2.5 1.5B on a CPU-only PC. On the laptop's Arc 140V iGPU it scored
  80 % on the 88-suite at p50 0.62 s
  (`docs/findings/2026-10-30-laptop-model-benchmark.md`).

### Changed

- **Windows: the default AI cleanup model is now Qwen2.5 1.5B.** With no
  model file set, local cleanup loads `qwen2.5-1.5b-instruct-q4_k_m.gguf`
  instead of the 0.5B. On the 88-suite the 1.5B scores 74 % and the 0.5B
  43 %; the 1.5B takes about 1 s (p50) on a laptop CPU. A PC that has only
  the old 0.5B file keeps using it. Nothing is migrated or deleted, and a
  model chosen in Settings stays as it is.

## [0.11.1] - 2026-10-30

### Fixed

- **Windows 0.11.0 installers were missing:** CI's newer clippy rejected two
  audio-byte conversions in history. 0.11.1 is 0.11.0 with that fixed.

## [0.11.0] - 2026-10-30

### Added

- **AI cleanup over the server (all platforms).** Settings → AI cleanup has a
  "Cleanup order" list like the speech route order: On this device, Server and
  Cloud, each with a switch, default device → server → cloud. Each dictation
  is cleaned by the first one that is on and ready, falling back down the
  list, whichever route transcribed it; the server cleans text sent to it
  with a new `clean` request (protocol feature `clean`). The Developer
  cleanup order and "Allow cloud cleanup" settings move into this list
  (decision D36). If AI cleanup was off before the update, the Server entry
  starts off, so nothing changes until you turn it on.
- **Windows: press a shortcut to set it.** Change on a shortcut (Settings →
  General, setup's push-to-talk, and a transform's new "Press keys") now
  records: hold the keys, let go, and that chord is saved. The keyboard hook
  records it, so Right Alt is told apart from AltGr and the Win key works.
  Shortcuts don't fire while recording. Esc, 10 s without keys, or leaving
  the window cancels. "Type instead" opens the old text field for keys you
  can't press.
- **Windows: download AI cleanup models.** Settings → AI cleanup → On this PC
  has a "Download a model" row like the speech one: Qwen2.5 0.5B Instruct
  (Q4_K_M, 491 MB, the default) or Qwen2.5 1.5B Instruct (Q4_K_M, 1.1 GB),
  each pinned to a Hugging Face commit and SHA-256 checked. A finished
  download turns cleanup on this PC on with that file and loads it; Use
  switches between downloaded models and Delete removes one (turning local
  cleanup off when it was in use). Pins: `docs/findings/2026-10-30-cleanup-model-pins.md`.

- **Retry with current settings (Windows, Android).** History can transcribe a
  dictation's exact recording again with the settings active now (speech
  route, engine, model, language, dictionary, style, AI cleanup), styled for
  the original app and never pasted. Windows: a Retry icon beside Copy and
  Paste again; Android: "Retry with current settings" in the card ⋮ menu and
  on the details screen. Each retry is a new version: the card shows the
  newest, and numbered pills (0 = original) show earlier ones. Copy, Paste
  again and Insights use the newest. The recordings of the newest 20 History
  rows are kept for this ("Keep recordings for retry" in Data and privacy /
  Settings > History; 0 turns it off), never from password or protected
  fields, and deleted with their row (D35).
- **Core: Hebrew zipformer engine.** A new speech engine, `sherpa_online`
  (also accepted as `zipformer`), runs streaming sherpa-onnx models such as
  ivrit.ai's 69 MB Hebrew zipformer. Use it as the Hebrew model
  (`languages.he = {asr_backend: "sherpa_online", asr_path: <dir>}`) or as the
  default one. Dictionary words are passed as hotwords, which fix the spelling
  of words the model already hears. Builds without the `parakeet` feature
  report a load error for it, as they do for Parakeet.
- **Server: Hebrew.** flow-server can load a Hebrew model next to English
  Parakeet: `[model.languages.he]` in its config (example in
  `crates/flow-server/dist/config.example.toml`, guide in
  `docs/server-setup.md`). Sessions whose language is set to Hebrew decode on
  it and return `"language":"he"`; everything else, auto-detect included,
  stays on the default model. `hello_ok` / `/v1/health` list the server's
  languages (`"languages":["he"]`, a new optional field), and the `result`
  log line and debug capture record the language and the model that decoded.
- **`tools/fetch-models.sh --hebrew`** downloads the 69 MiB Hebrew zipformer
  (pinned revision, SHA-256 checked, already-verified files skipped);
  `--hebrew-parakeet` the 642 MiB Parakeet-he alternative. `FLOW_MODELS_DIR`
  picks the destination.
- **Core: Hebrew auto-detect (H6, D34).** Set `ModelConfig.lid_path` to a
  multilingual whisper model (`ggml-tiny-q5_1.bin`) next to a `languages`
  map. An auto-detect dictation then has its first 3 s identified (~70 ms
  on an i7-6700) and decodes Hebrew on the Hebrew model. Without
  `lid_path`, nothing changes.
- **Android: Hebrew model.** *Settings → Speech to text → Model on this
  phone → Hebrew model* imports ivrit.ai's zipformer-he (69 MiB) or
  parakeet-v3-he as a folder or a .zip; each file's size and SHA-256 are
  checked against the pinned release. With the dictation language set to
  Hebrew (or a Hebrew phone language), the phone loads only that model, so
  one model stays in RAM; switching language reloads the engine (a few
  seconds, "Loading the Hebrew speech model…"). If it fails to load,
  whisper is used. flow-ffi accepts `load_language` in the model config for
  this (`FlowCore::load_models_for`).
- **Windows: Hebrew dictation.** Settings → General → Languages is now a
  picker (Detect automatically, English, Hebrew, or Other… for typed codes).
  Settings → Speech to text has a **Hebrew model** choice: Zipformer
  (recommended, 72 MB), Parakeet Hebrew (673 MB) or None. With the language
  set to Hebrew, dictations decode on that model; every other language keeps
  the default model, and both stay loaded, so switching language reloads
  nothing. Both Hebrew models are in the download catalogue (pinned commit,
  SHA-256 per file, CC-BY-4.0 credit shown in the row). Choosing Hebrew with
  no Hebrew model downloaded shows a hint that links to the download. A
  missing Hebrew model never blocks the default one.
- **Windows: auto-detect hears Hebrew.** Each Hebrew model download now
  also fetches Whisper tiny (32 MB, `ggml-tiny-q5_1.bin`) as a language-ID
  model. With it present and a Hebrew model chosen, "Detect automatically"
  sends Hebrew speech to the Hebrew model and everything else to the default
  one. Without it, auto-detect stays English as before; a Hebrew model
  downloaded earlier shows "Download 32 MB…" to fetch just that file.

### Fixed

- **Windows: Launch at login after a reinstall.** With Launch at login on,
  the app now re-points its startup entry at its own exe each time it starts.
  Before, the entry was written only when settings were saved, so after
  reinstalling to a new folder (the "Flow Clone" → "Murmur" rename) it
  pointed at a deleted exe and the app no longer started at login.
- **Windows: opening the app twice.** Starting Murmur while it already
  runs no longer adds a second tray icon (and a second keyboard hook): the
  new launch brings the running app's Settings window to the front and
  exits. A dictation in progress is not interrupted.

## [0.9.2] - 2026-10-30

### Fixed

- **Windows: crash a few seconds into a dictation with local Whisper** on
  CPUs without AVX-512 (e.g. Intel Core Ultra / Lunar Lake, most AMD and
  consumer Intel chips). whisper.cpp was compiled for the CI machine's CPU;
  it is now built for an AVX2 baseline that every supported PC has.

## [0.9.1] - 2026-10-30

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

[Unreleased]: https://github.com/bashycodes/murmur/compare/v1.1.2...HEAD
[1.1.2]: https://github.com/bashycodes/murmur/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/bashycodes/murmur/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/bashycodes/murmur/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/bashycodes/murmur/compare/v0.12.0...v1.0.0
[0.12.0]: https://github.com/bashycodes/murmur/compare/v0.11.1...v0.12.0
[0.11.1]: https://github.com/bashycodes/murmur/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/bashycodes/murmur/compare/v0.9.2...v0.11.0
[0.9.2]: https://github.com/bashycodes/murmur/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/bashycodes/murmur/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/bashycodes/murmur/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/bashycodes/murmur/compare/v0.7.1...v0.8.0
[0.7.1]: https://github.com/bashycodes/murmur/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/bashycodes/murmur/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/bashycodes/murmur/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/bashycodes/murmur/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/bashycodes/murmur/compare/v0.3.0...v0.4.0
