# 2026-11-27: spoken language ID on the phone (Automatic on Android, phase 1)

**Question.** Can the phone run D34's language ID (`murmur_core::lid::WhisperLid`,
whisper tiny q5_1, 32 MB) for Automatic, as Windows and the server do, instead of
using the keyboard as a hint (D113)? Gate: **go** if language ID adds ≤ 600 ms to a
dictation and the one-big-model rule (D33 decision 4) holds, or a change to it is
justified.

**Answer: GO.** Language ID costs about **48 ms** per call on the phone's CPU and
**~70 MB** of RAM, about a tenth of the resident Parakeet model. It runs at the 3 s mark
while the user is still speaking, so a dictation longer than 3 s pays nothing at release
and a shorter one pays ≈ 50 ms. That is far under the 600 ms gate. The one-big-model rule
holds if it is amended to "one big ASR model plus the 32 MB language-ID model".

**The real cost is the model switch, not the detection.** A dictation detected as Hebrew
while the English model is resident (and "Keep both languages loaded", D54, is off) has to
load the Hebrew model: **≈ 4.7 s** for Parakeet-he on this phone. That is the same reload
the phone already does today when the keyboard switches language. Phase 2 must hide that
reload behind speech and only pay it when the language changes (sketch below).

## Setup

* Phone: an SM-S936B-class phone (arm64). The probe ran from the shell area
  (`/data/local/tmp`), outside the app, with the app's data and settings untouched. Files
  pushed: the probe (2 MB), `libc++_shared.so` and `ggml-tiny-q5_1.bin` (32 MB). All were
  removed afterwards.
* Probe: `tools/probes/prototypes/phone_lid_2026_11_27/`, a standalone crate around
  `WhisperLid::load(path, 4, fit_ctx = true)` and `LanguageId::scores` on the first 3 s
  (`LID_SAMPLES`), candidates `en` and `he`. It is built with the hybrid flavour's native
  settings (`GGML_CPU_ARM_ARCH=armv8.2-a+dotprod+fp16`, NDK 27.2, platform 33,
  16 KB pages; same toolchain file as `build-native.sh`). It runs on 4 threads, as the
  app does.
* Audio: 3 s of synthetic voiced signal. Language-ID time does not depend on the content,
  because the encoder always runs on the same shrunk window (D34). Accuracy is D34's
  (79/81 on the PC set) and was not re-measured here; the synthetic signal scores as
  English.
* "Alongside Parakeet": `--ballast-mb 900` allocates and touches 900 MB before loading.
  It stands in for the resident Parakeet-en (769 MB PSS after load,
  `2026-10-31-phone-on-device-bench.md`). The real model was not reachable from the shell,
  and pushing it would have blown the cellular data budget.

## Numbers

| | load (ms) | RSS after load (MB) | MemAvailable after load (MB) | LID first call (ms) | LID warm mean (ms) | LID warm p90 (ms) |
|---|---|---|---|---|---|---|
| LID alone (10 runs) | 117.7 | 74.6 | 3240 | 50.6 | **47.7** | 49.6 |
| LID + 900 MB resident | 101.2 | 974.7 | 2395 | 42.0 | **46.7** | 54.0 |
| *reference: i7-6700 PC, 4 threads* | 133.6 | +71 | | 60.9 | 62.1 | 68.1 |

* The load includes the D34 warm-up decode that shrinks the window. The model file was in
  the page cache (just pushed). A cold read of 32 MB from UFS flash adds a few tens of ms
  at most.
* whisper.cpp reports 31.6 MB of weights. It reserves compute buffers of 65 MB (encode) and
  96 MB (decode), but only the encoder path runs for language ID, so the RSS stays at
  ≈ 70 MB.
* With 900 MB already resident, the language-ID model adds the same ≈ 70 MB and runs
  equally fast. There is no interaction beyond the RAM sum.

### Switching to the Hebrew model (from existing phone measurements, not re-run)

| model on the phone | load | PSS after load | source |
|---|---|---|---|
| parakeet-en (default) | 3.7 s | 769 MB | `2026-10-31-phone-on-device-bench.md` |
| parakeet-he | 4.7 s | 797 MB | same |
| zipformer-he | 16.6 s measured, ≈ 5–6 s expected after the Load 3 fix | 311 MB | same, `2026-11-05-phone-model-load-time.md` |
| whisper tiny q5_1 (language ID) | 0.1 s | ≈ 70 MB | this probe |
| on-phone cleanup LLM (if used) | 38 s | 2078 MB | `2026-10-31-phone-on-device-bench.md` |

* **One model resident (D54 off).** Free the English model, then load the Hebrew one. Peak
  RAM is about 0.8 GB + 70 MB, which keeps the one-big-model rule. The dictation waits for
  the load: up to ≈ 4.7 s for Parakeet-he if nothing overlaps it. Started at the 3 s
  detection point, the load overlaps the rest of the speech. For example, a 6 s dictation
  waits ≈ 1.7 s extra plus the decode. Later dictations stay on the Hebrew model until
  language ID hears English again, so only a language *change* pays this, as a keyboard
  switch does today (D113, D58).
* **Both resident (D54 on).** 769 + 797 + 70 ≈ 1.64 GB fits in the 3.2 GB that was
  available at measurement time, and the switch costs nothing. It does *not* fit next to
  the on-phone cleanup LLM (2.1 GB). So D54 stays opt-in and is not the default.

## Verdict against the gate

| criterion | result | pass |
|---|---|---|
| LID adds ≤ 600 ms to a dictation | ≈ 0 ms at release for dictations > 3 s (runs during speech); ≈ 50 ms at commit for shorter ones | yes |
| one-big-model rule | +70 MB beside a ≈ 0.8 GB ASR model; amend D33 decision 4 to "one big ASR model + the language-ID model" | yes, with that amendment |
| switch cost | ≈ 4.7 s (Parakeet-he) per language change with D54 off; 0 with D54 on | not gated; phase 2 hides it behind speech |

**GO.**

## Phase 2 sketch

1. **Core: language ID with one model resident.**
   * `MurmurCore::load_models_for` keeps `AsrRoutes.lid` (today always `None`): it calls
     `load_lid(cfg)` when `cfg.lid_model()` is set. `lid_model()` needs `languages`
     non-empty, which the phone already fills with the Hebrew entry.
   * In a one-model core, when `lid::choose_whole` routes to a language that is not loaded,
     the session must not decode on the wrong model. It reports the wanted route
     (e.g. `CommitResult.route_wanted = Some("he")`, plus a session event at the 3 s point)
     and holds the PCM, as the D34 hold already does.
   * `Timings.lid_ms` reports the cost as on the other shells.
2. **FFI (`murmur-ffi`, JNI).** Expose the wanted route in the session's poll/commit JSON,
   so no new symbol is needed beyond the field. Add a way to replay held PCM into a freshly
   loaded core (`nativeReplay(h, pcm)`, or the shell keeps the PCM it already records).
3. **Android shell.**
   * `EngineHolder`: when the wanted route ≠ the current `LoadSet`, call `keep_llm`, free
     the core, load `LoadSet.Language("he")` (or `Default`) right away (at the 3 s event,
     not at the stop), and decode the dictation on it. This reuses the D58 re-run-at-stop
     path that already handles a keyboard switch mid-recording.
   * `LanguageDefault.resolve("auto", offPhone = false)` stops forcing the keyboard
     language once the language-ID model is installed. The keyboard can still choose which
     model to load *first* (a good prior), and language ID corrects it.
   * With D54 on, nothing changes: `load_models` already loads `lid` with both models.
4. **Model import.** Ship `ggml-tiny-q5_1.bin` as an extra file of the Hebrew model, as the
   Windows downloader does (D31). It comes from the library download (hybrid) or inside
   the imported Hebrew model folder (local). Set `ModelConfig.lid_path` only when that
   file exists, a Hebrew model is set and the language is Automatic. Without the file,
   D113's keyboard hint stays.
5. **Tests.**
   * Core: a one-model core with `MockLid` routes to an unloaded language and reports
     `route_wanted` without decoding on the default model.
   * Android: an `EngineHolder` unit test for reload-and-replay.
   * Device check: `engine_load_timings` shows `kind: lid` at ≈ 0.1 s, and the commit
     shows `lid_ms` ≈ 50.

## Limits

* The probe ran in a shell process, not inside the app. The app's foreground-service
  cpuset may keep it off the big cores (an open suspect in
  `2026-11-05-phone-model-load-time.md`). Even at 4× slower, language ID stays at ≈ 200 ms.
* The Parakeet RAM row uses a 900 MB stand-in, not the real model. The Hebrew-switch load
  times are earlier phone measurements, not re-run here.
* Phone accuracy is assumed to match the PC (the same whisper.cpp model and window).
