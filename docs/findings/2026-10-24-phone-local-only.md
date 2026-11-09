# 2026-10-24: phone went local-only, a target_changed dictation, blank_asr, client latency gap

Evidence: the phone's D21 debug log (59 dictation lines, 2026-10-23 evening to
2026-10-24 13:03 UTC) and the murmur-server journal for the same times. The
phone ran the f2cea77a build until 02:48 UTC, then the hybrid build from
`fix/never-fail` + `feat/devsettings-ui` (merge 0548140 era). Transcript
content is not quoted here, only word counts.

## 1. The phone stopped using the server (from 02:46 UTC)

* Before 02:46: 22 dictations went `route=remote`, 20 `local`. From 02:46:
  15 of 15 went `local`, all with `fallback=None`. A `None` fallback means the
  phone was the first choice, not a failover: the server was never tried.
* Stored prefs: `route_order="client,server"`. The router's `prefer` is
  derived from the first of server/client in that list (`DevSettings.prefer`),
  so "client,server" means phone first.
* **Cause: the user's own change.** The user confirmed they set the order to
  "client,server" on purpose. This is not a bug, and routing behaviour and
  defaults are unchanged. For the record, the code agrees: the only writers of
  `route_order` are `DevStore.save` / `saveCloud`, called from the Developer
  pages' Save with the order the ↑/↓ rows show (`DeveloperActivity.routeOrder`,
  which is loaded from the stored order). The connect-timeout migration
  (800 → 2500) touches only `connect_timeout_ms`, and `keep_local_warm` and the
  D24 pre-warm do not touch the order.
* What changed so this is easier to see next time:
  * Developer hub "Route in use" and the Routing row subtitle now read
    "Phone first → Server", "Server first → Phone" or "Phone only" (was
    "On-device → Server"). The Routing page's numbered rows name the phone
    route "Phone (on-device)". Test: `DevPagesTest.routingSummary*` (the
    summary follows the stored order).
  * Debug log `route_order` (additive): the routes the router may take for the
    build, in try order, e.g. `client,server`. Test:
    `DevSettingsTest.routeOrderWireListsTheEnabledRoutesInOrder`,
    `DebugLogTest.routeOrderAndSpeechDiagnosticsAreAdditive`.

## 2. target_changed lost a dictation (02:10:30 UTC, 41 words)

Remote, 12.96 s of audio, LLM timed out at 3000 ms (`post_ms` 3000), 4.8 s
from stop to outcome. Outcome `target_changed`, no insert steps: at insert time
`insertTarget(explicit = false)` said the focused field was no longer the one
pinned at dictation start, so nothing was typed. (The field may have moved,
or the app may have rebuilt its editor node during the 4.8 s wait; the log
cannot tell which.)

What happened to the text before this fix (`DictationController`):

* History: `recordHistory(result, inserted = false)`. The row has the text
  (unless History retention is "never").
* Bubble: `Error(FailedToPaste(text))` with ✓ (Paste into the field focused
  now) and ✕ (dismiss). There was **no copy**. The bubble's COPY action copied
  `lastText`, which is set only by an insert attempt, so it would have copied
  the previous dictation.
* Clipboard: untouched. Notification: none. Toast: "Tap Paste to insert it
  here".
* So one tap on ✕ or on the bubble body (`MAIN` in an error dismisses it), or
  the next dictation started from the error state, left History as the only
  copy. That breaks "never lose a dictation" (D24 spirit).

Fix:

* Controller: a new `Host.onNotInserted(text, outcome)` runs for
  `target_changed`, `target_protected` and `paste_failed`, with the full text
  (all text runs).
* App: it posts a "Dictation not inserted" notification ("You switched
  fields while dictating (N words). Tap to copy it."). It never shows the text
  (lock screen), and it stays until tapped, which copies the text with the
  S16 one-minute clipboard clear.
* Bubble: in `FailedToPaste` it now shows ✓ insert here · ⧉ copy ·
  ✕ dismiss, and ⧉ copies the failed text itself.
* Tests: `DictationControllerTest.targetChangedKeepsTheTextRecoverable`,
  `failedPasteAndProtectedTargetAlsoKeepTheTextRecoverable` and
  `insertedOrEmptyDictationsAreNotOffered`, plus `NotInsertedNoticeTest`.

## 3. blank_asr on 3.8 s and 0.8 s of audio through the server (20:01 UTC)

Server journal for both: `asr_audio_ms == audio_ms` (3800 / 800),
`segments=1`, `status=Empty empty_reason="blank_asr"`. So the energy VAD did
not reject the audio (otherwise the reason would be `no_speech`), and it
trimmed nothing. The ASR got the whole clip and returned an empty string. That
fits two cases: real speech the model dropped, or noise or a quiet signal
where the VAD fails open (by design, `vad.rs`). Without the audio (rotated off
the phone) the log cannot tell them apart. Neither the VAD's speech time nor
the level was logged anywhere.

Added (additive, no threshold change):

* `timings.speech_ms`: the VAD's speech frames (summed over pieces when
  segmented; `0` = none; absent = VAD off).
* `timings.peak_dbfs`: the peak level of the session audio.

Both are in every commit result from murmur-core (local and server routes), in
the server's `result` log line, and in the Android debug log
(`speech_ms` / `peak_dbfs`, also in the in-app viewer). The next time a
blank_asr shows a low `speech_ms`, or `peak_dbfs` around -40, the cause is
the signal. A high `speech_ms` at a normal level points at the ASR. Tests:
`murmur-core/tests/api_additions.rs` (`blank_asr_reports_vad_speech_and_peak_level`
etc.), `CoreJsonTest.decodesSpeechAndPeakDiagnostics`,
`DictationControllerTest.traceCarriesSpeechAndPeakFromTheResult`.

## 4. Client-side latency gaps

**20:01:22 `wait_ms` 4426** (server `total_ms` 4671): the server core's
commit waited for a decode already in flight on that worker (D22
`wait_ms`). The box was busy at the time, so this is server-side and expected.
The phone's `client_ms` 5550 is that plus the network.

**02:45:39 `client_ms` 6241 vs `total_ms` 1703** (f2cea77a build, so no
`connect_ms` field). `client_ms` is `Router::commit` wall time. The remote
backend sends `commit` on the same ordered writer queue as the audio and then
waits for the result (`murmur-remote` `RemoteBackend::commit`). It does not
connect, probe or sleep there. A timeline built from the two clocks:

| event | time (UTC) | source |
|---|---|---|
| record start | 02:45:39.32 | phone `ts` |
| server: connected | 02:45:40.09 | journal (connect at session start, no reused connection) |
| release, `Router::commit` starts | ≈ 02:45:46.6 | 39.32 + 7.16 s audio + drain; consistent with 52.89 − 6.24 |
| server commit begins | ≈ 02:45:51.18 | result time − `total_ms` |
| server result | 02:45:52.89 | journal |

So the `commit` message reached the server about 4.5 s after the phone sent
it. Nothing on either side waits in that window. The message sat behind the
audio upload: the connection was opened 0.8 s into the recording, on a link
with 239 ms RTT (a VPN relay path, as in D24), and the uplink could not
keep up with real time during TCP slow start. The next dictation on the same
connection 18 s later had a 150 ms gap (`client_ms` 2860 vs `total_ms` 2708),
which fits a fresh-connection effect.

* The hybrid build already has the cheap fix: the D24 pre-warm opens the server
  connection when the field gets focus, off the dictation's critical path.
  It also logs `connect_ms`.
* Added to see the rest: murmur-server now logs `start_to_commit_ms` (`start` →
  `commit` arrival). Upload backlog at commit ≈ `start_to_commit_ms` − (phone
  recording length − `connect_ms`). No wire change.
* Not added: a phone-side "unsent audio" counter. Audio leaves the app's
  writer queue into the kernel socket buffer almost at once, so the app-level
  queue would read about 0 even with a multi-second backlog.
