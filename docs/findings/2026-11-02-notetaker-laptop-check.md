# Notetaker on the laptop, 1.0.0+a24090a, 2026-11-02

> **Status:** device check of D45, D47 and D48 (tickets "Notetaker 1/2/3") on the laptop: Core Ultra 9 288V, Windows 11 Home, hybrid build, speech to text on Parakeet on this PC, then the server. CI run 41448883609.

## Setup
- **Install:** hybrid NSIS `Murmur_1.0.0_x64-setup_hybrid.exe` (sha256 `5e613048…794f`), `/S` over 1.0.0, exit 0. `%APPDATA%\FlowClone` (without `models`) was backed up first. Prefs and history were the same afterwards, apart from the new build's own keys `hebrew_model_dir` and `lid_model_path`.
- **Launch:** the app ran from an interactive scheduled task with stderr going to a temp log.
- **Driving the UI:** UI Automation on the settings window (Meetings → Start/Stop recording, the detection select, Delete).
- **Laptop state:** locked throughout (`LockApp` in the foreground). UI Automation, playback and WASAPI loopback all worked while locked.
- **"Others" audio:** a 44 s public-domain clip (JFK plus LibriSpeech test-clean 260-123440-0004, 2094-142345-0059 and 1995-1836-0008) played through the speakers at 35 % master volume. Nobody spoke, so the mic only caught bleed from the speakers.

## D45: record, transcribe, Me/Others: pass

| meeting | length | system track | mic track | transcription | turns |
|---|---|---|---|---|---|
| 1 (clip once) | 74.9 s | −18.8 dBFS, 53 % blocks non-silent | −40.0 dBFS | 15.4 s (4.9× real time) | 4 Others, 0 Me |
| 2 (clip twice) | 108.2 s | −17.4 dBFS | −38.5 dBFS | 18.2 s | 7 Others |
| 3 (clip twice, dictation during the job) | 105.3 s | −17.3 dBFS | −38.5 dBFS | 21.7 s | 7 Others |

- **Indicators:**
  - The Flow bar chip read "Recording meeting · 0:04".
  - The Meetings page read "Recording · your mic and the PC's sound", then "Transcribing a meeting. 0%. Dictation keeps priority.", then done.
- **Accuracy:** in meeting 1, 91 of the 92 Others words are in the reference. The only miss: "Missus Poyser" came out as "misses Poyser".
- **Gaps fill correctly:** loopback captured 44 s of the 75 s, and the system WAV was padded to the full length with silence.
- **Me not checked:** there were no Me turns, because no one spoke and the mic bleed was either below the speech level or dropped as echo.

## Dictation during a meeting job: pass

Push-to-talk (Right Alt, injected) for 7–8 s of JFK; times are from the debug log.

| | audio | speech to text | release → text |
|---|---|---|---|
| no meeting | 7.5 s | 1234 / 1196 ms | 1261 / 1247 ms |
| meeting 3 transcribing | 6.6 s | 1376 ms | **1449 ms** |

- The dictation never waited (`wait_ms` 0).
- The meeting job took about 3.5 s longer than meeting 2, which fits D45's pause while a dictation runs.

## D47: meeting route list: pass

- Settings › Speech to text › Meetings lists This PC (on), Your server (on) and OpenRouter (off), with "Same as the engines above until you change it here."
- `meeting_routes` is absent, so meetings follow dictation.
- Each meeting job loaded its own model on this PC at its start: a fresh model load in stderr just after "stopped".
- **Gap:** the engine that served a meeting is not logged.

## D48: detection: idle part only

- **CPU with "Ask me" on and no client running:** 0.57 % of one core over 60 s, against 0.29 % with detection off. That is about 9 ms per look, with no prompt.
- **One signal line with detection off:** a single `meeting-detect: signals: no meeting app running` appears at startup even with detection off. Only one line appeared in about 17 min, so detection does not poll while off.
- **Zoom call not run:** the test meeting needs browser clicks and the camera, and the laptop was locked. The window class and mic-session open questions in `2026-11-02-meeting-detection-signals.md` are still open.

## Other things noticed
- The CI build reports itself as `a24090a-dirty` in debug events.
- `taskkill` without `/F` cannot close the app from another session, so an install over SSH has to force-kill it.
