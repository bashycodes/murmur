# 2026-10-23: phone local-route quality "gone down" since the debug-log build

Report: S25+ (Android 16), hybrid APK, local route, whisper `ggml-base-q5_1.bin` (multilingual),
language `auto`. Since the debug-log APK (`553612b7…`, integration c038c90) the user sees bad
transcripts ("Favorite worst nightmare by R2 Pointies?", a 2 s English utterance as Bulgarian
"И окей, това е си, Бъгънгъл."). Earlier builds (`728851be…` at 32982a1, `b23e8fd…` on
`fix/android-live`) were "working quite well". Every debug-log line has `endpoint_wait_ms=0`.

## Verdict

**No code or binary difference on the ASR path between the "good" 728851be build and the
debug-log build.** The regression is not in what we shipped; it is in what the phone is fed or
configured with, or in perception (the debug log now shows the raw recogniser output). One real
but cosmetic defect: `endpoint_wait_ms` is always 0 on real mics (debug metric only).

## Evidence

### 1. Native libs: bit-identical to the good build

| APK | libflow_ffi.so (hybrid, sha256) | libflow_llm.so | others |
|---|---|---|---|
| 728851be hybrid-debug (`integration/app/build/outputs/apk/hybrid/debug/app-hybrid-debug.apk`, 2026-10-22 22:18, still on disk) | `cf4586b73bf3…` | `f16fff37af46…` | same |
| f97ef200 hybrid-release (same dist run) | `cf4586b73bf3…` | `f16fff37af46…` | same |
| **553612b7 debug-log** (`integration/dist/`) | **`cf4586b73bf3…`** | **`f16fff37af46…`** | same |
| CI run 39837428823 (b9942f1, CI-built) | `5b15af84d2ff…` (rustc 58cf8f9c) | `845acb07…` | onnxruntime/sherpa/libc++ same |
| android-streaming (fresh, 18:29) | `fff89d424870…` | — | — |

The "copied" jniLibs (built 2026-10-22 21:57 after merge 49e1e54, `tools/build-native.sh
--flavor …`) are the exact libs the 728851be APK shipped. Provenance is therefore not a variable
between "good" and "bad". (CI vs local differ only by rustc build; same crate versions, same
whisper-rs 0.16.0.) Core is unchanged in the range (`git log 32982a1..c038c90 -- crates/` empty
for flow-core/flow-ffi).

### 2. Kotlin diff 32982a1..c038c90: nothing on the audio/ASR path

`git diff 32982a1 c038c90 -- apps/flow-android` (19 files): debug log (D21), a11y health
debounce/probe (9d81923), model import fix (a7bcfba). Checked one by one:

* `MicRecorder` (source `MIC`, 16 kHz mono PCM16): unchanged.
* `DictationController` frame callback: unchanged order `gate.push → meter → silence`; the
  trace only reads state after `endAudio()`; `onDictationTrace` is try/catch and enqueues.
* `SilenceTracker.update` now keeps running after first speech (trailing measure). It only
  feeds the "no speech yet" hint and the trace; it never touches PCM.
* `sessionConfig()` / `toSessionConfig` (language, dictionary → `initial_prompt`, preview,
  endpointing), `EngineHolder` load, route selection: unchanged (only `debugLog.event` calls
  added on failure paths).
* Android never sets `screen_words`, so `initial_prompt` = dictionary only.

### 3. Host repro: base-q5_1 + auto does not mis-detect clean English, even 2 s / 5 dB SNR

`tools/probes/prototypes/lang_auto_short_clips_2026_10_23.py` (flow-cli = same core, fit_ctx on,
`--platform android --threads 4`), bench core set (jfk + 12 LibriSpeech), full / first 3 s /
first 2 s, clean / white noise 15 dB and 5 dB SNR:

| config | full WER | non-English detections (2 s and 3 s crops, all noise levels) |
|---|---|---|
| base-q5_1, auto | 0.014 | 0 / 78 (clean, 15 dB, 5 dB) |
| base-q5_1, `en` | 0.014 | 0 / 78 |
| base.en (f16) | 0.024 | 0 / 52 (clean, 15 dB) |

Raw rows: `~/Claude/research/wispr/.cache/diag-2026-10-23/lang_auto*.jsonl`. So the device
behaviour (Bulgarian on 2 s, garbled words) is not reproduced on native read speech; it needs
the phone's actual audio (accent, spontaneous speech, distance, mic processing), which no build
records today.

### 4. `endpoint_wait_ms = 0`: real defect, debug metric only

`SilenceTracker` uses Wispr's fixed "can't hear you" bar (mean s² > 33856, RMS 184 ≈ −45 dBFS).
A phone mic's room-noise floor is above that, so frames after the last word still count as
signal and the trailing silence never accumulates. Exactly 0 on every line (not small values)
means the last read before every stop was above −45 dBFS, including hands-free stops that the
core's own adaptive VAD (vad.rs, 3× noise floor) had already judged silent. Regression test:
`DictationTraceTest.endpointWaitCountsRoomNoiseAfterSpeechAsSilence` (speech RMS 3000, then 300
ms of RMS-400 noise; expects ~300 ms, gets 0). Committed `@Ignore`d so the suite stays green;
un-ignore with the fix. It does **not** affect ASR: the core trims/endpoints with vad.rs.

Proposed fix: measure the trailing silence with a relative bar, e.g. track the running noise
floor (10th percentile frame RMS, as vad.rs does) and count a frame as silent when RMS <
max(3 × floor, −48 dBFS) and < 0.3 × the speech peak; keep the absolute bar for the leading
"no speech yet" hint. Or report the core's own endpoint/trim numbers through `CommitResult`.

## Ranked hypotheses for the quality drop (all outside the shipped bits)

1. **Different engine or route than in the "good" period.** In the live test (PROJECT_CONTEXT
   "Live test 2026-10-23") the user dictated on both the server route (Parakeet on this box,
   much better English) and the local route. If most "good" impressions came from the server
   route, or from local Parakeet, and now everything is local whisper base-q5_1, quality drops
   exactly like this. A Parakeet→whisper fallback (`EngineLogic`, "Parakeet failed: …") is
   shown on the main screen but is **not** written to the debug log.
2. **Multilingual base + `auto` on accented, spontaneous, short phone speech.** base is the
   weakest multilingual model; language ID on ~2 s is unreliable for non-native English (the
   Bulgarian line is a language-ID miss; "R2 Pointies"-style errors are base-model acoustic
   errors). Forcing `en` (or base.en-q5_1 per INSTALL.md's recommendation) removes the
   language-ID failure mode outright. Host bench shows `en` costs nothing on English.
3. **Settings drift since the good period:** language changed to `auto`, a model re-imported
   (base-q5_1 instead of base.en / small), dictionary grown (biases `initial_prompt`), AI cleanup
   turned off (raw errors no longer smoothed), or a different input (case, distance, Samsung mic
   mode / "Voice focus").
4. **Perception:** the debug log shows `raw` for every dictation; before, only the inserted text
   was seen, and only in a short, attentive test (13 inserts).

## Device experiments (orchestrator runs; no rebuild needed for 1-3)

1. Main screen / Settings: note the "Loaded: …" engine line (whisper file name or
   `client: Parakeet …`, any "(Parakeet failed: …)"), Language value, Speech model, AI cleanup
   on/off, dictionary entry count, Developer settings route order + allow server/client.
2. In each `files/debug/dictations.jsonl` line: `route`, `fallback`, `mode`, `audio_ms`.
   Compare with the always-on logcat `commit … route=…` lines in the history window.
3. A/B on the device without changing builds: History screen (or `history.db`, table
   `history`, `asr_text`, `audio_ms`, `created_at` via the app's `run-as dev.flowclone`) rows
   from **before** the debug-log install vs after: same model + settings? If old local rows are
   just as bad, nothing regressed.
4. Settings → Language `en` (no rebuild), dictate the same 5 sentences (one ~2 s, four 5-10 s)
   with `auto` and then `en`; compare `raw`. Then import `ggml-base.en-q5_1.bin` and repeat.
   If `en` / base.en fixes it, ship a default of `en` for English-locale phones or warn when a
   multilingual base/tiny model runs with `auto`.
5. To make future reports falsifiable: a Developer "save last dictation audio" option (WAV in
   app-private storage, off by default) so a bad line can be replayed on the host with flow-cli.
