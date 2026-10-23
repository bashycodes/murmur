# flow-core architecture

Shared Rust core of the local Wispr Flow clone. It implements the §9 v2 API from
`windows-research/findings/by-topic/10-clone-spec.md` (Android alignment in
`10-clone-spec-android.md` §9). All speech recognition and text shaping run
on-device; the core makes no network calls.

```
crates/
  flow-core/   lib: session engine, ASR/LLM seams, deterministic post-processing
  flow-ffi/    C ABI (cdylib/staticlib, include/flow_core.h) + JNI (feature `jni`)
  flow-cli/    dev harness: `flow-cli transcribe` = full pipeline on a WAV; `flow-cli clean` = LLM cleanup on text
  flow-llm/    llama.cpp cleanup engine as a cdylib plugin (libflow_llm.so / flow_llm.dll), loaded at runtime
  flow-asr-sherpa/  Parakeet-TDT ASR via the official sherpa-onnx crate (flow-core feature `parakeet`)
```

## API (§9 v2)

| §9 call | Rust (`FlowCore`) | C ABI | JNI (`com.flowclone.core.NativeFlowCore`) |
|---|---|---|---|
| `load_models(ModelConfig)` | `FlowCore::load_models(&ModelConfig)` | `flow_load_models(json) -> FlowHandle*` | `loadModels(json): Long` |
| `unload` | `unload(self)` / drop | `flow_unload` | `unload(h)` |
| `start_session(SessionConfig)` | `start_session(cfg)` | `flow_start_session(h, json)` | `startSession(h, json)` |
| `push_audio(pcm16)` | `push_audio(&[i16]) -> bool` | `flow_push_audio(h, ptr, len)` | `pushAudio(h, short[], len)` |
| `set_context(FieldContext)` | `set_context(ctx)` | `flow_set_context(h, json)` | `setContext(h, json)` |
| `partial()` | `partial() -> Option<String>` | `flow_partial(h)` | `partial(h): String?` |
| *(clone)* poll | `poll() -> SessionPoll`, `endpoint() -> Option<EndpointState>`, `endpoint_reached() -> bool` | not yet exported (see changelog) | not yet exported |
| `commit()` | `commit() -> CommitResult` | `flow_commit(h) -> json` | `commit(h): String` |
| `cancel()` | `cancel()` | `flow_cancel(h)` | `cancel(h)` |
| `learn_edit(pasted, edited)` | `learn_edit() -> Vec<Candidate>` | `flow_learn_edit(h,p,e) -> json` | `learnEdit(h,p,e)` |

The JNI class/package name is provisional until `apps/flow-android/JNI_CONTRACT.md`
lands. Renaming it only changes the `Java_…` symbol prefix in
`crates/flow-ffi/src/jni_api.rs`.

All config types are serde structs in which every field has a default, so a
shell can send partial JSON (`{}` is a valid `SessionConfig`).
`ModelConfig.asr_backend` (alias `asr_engine`) is `"whisper"` (default),
`"parakeet"` or `{"mock":{"text":"…"}}`. The mock lets shells be developed
without a model.

`CommitResult` JSON:
```
{status: FORMATTED|RAW_TRANSCRIPT|EMPTY|ERROR, asr_text, final_text, language,
 timings{audio_ms, asr_ms, llm_ms, postprocess_ms, total_ms, dropped_samples, asr_audio_ms},
 press_enter, fallback_reason, truncated, error,
 segments: [{"text": "…"} | {"key": "Enter"|"Tab"|"Break"}], plain_text, empty_reason}
```
Status meanings:
* `FORMATTED`: the LLM cleanup ran and passed the guardrail.
* `RAW_TRANSCRIPT`: post-processed ASR text. This covers LLM off, LLM skipped (raw mode, level `none`, terminal) and LLM failed. `fallback_reason` says why it failed: `llm_timeout`, `llm_error: …`, `llm_unsupported` (an output sentence is not backed by the input), `llm_dropped` (input content missing), `llm_empty`, or `llm_diverged` (only with an explicit `divergence_threshold`).
* `EMPTY`: nothing to paste. `empty_reason`: `no_audio`; `no_speech` (the VAD found no speech, ASR was skipped); `blank_asr`; `hallucination` (a Whisper silence phrase was suppressed; `asr_text` holds it); `only_filler` (the spoken-cleanup pass removed everything). Blank counts as empty, which is the recommended deviation from Wispr Android, where `"   "` is inserted.
* `ERROR`: ASR error, no session, or the session was cancelled or superseded.

Paste fields:
* `final_text` keeps the in-band `{{BREAK}}`/`{{TAB}}` markers (compatibility).
* `segments` is the same text as a paste plan: text runs, `Tab` (press Tab), `Break` (finish inserting, pause ~150 ms, continue; no key), and a trailing `Enter` when `press_enter` is set. A shell that pastes `segments` must not also act on `press_enter`. Empty for EMPTY/ERROR.
* `plain_text` is `final_text` without markers (history, "paste last").

Load errors (`CoreError`, `#[non_exhaustive]`): `ModelNotFound{path}` (ASR or LLM model file missing, checked before whisper.cpp/llama.cpp run), `LlmLibNotFound{tried}` (no `flow-llm` plugin at any lookup location), `Load(String)` (anything else), `WorkerGone`. `CoreError::kind()` gives a stable snake_case name; C callers read it with `flow_last_error_kind()` (`""`, `bad_config`, `model_not_found`, `llm_lib_not_found`, `load`, `error`). Messages keep stable prefixes for JNI (`nativeLastError`): `model not found: <path>`, `llm plugin not found (…)`.

## Threads and buffering

```
shell audio thread ──push_audio──▶ [SPSC ring (rtrb), per session] ──▶ worker thread (core-owned)
                                                                        │ drain every 10 ms
shell UI thread ──start/commit/cancel──▶ control channel (mpsc) ──────▶│ partial ASR every N ms
                ──set_context──▶ Mutex<FieldContext> (read at commit)   │ commit: ASR → LLM → post
```
* **Non-blocking push.** `push_audio` copies into the ring under a mutex that the worker never takes. Only start, commit and cancel take it, so the audio thread never waits on inference. Overflow is counted in `dropped_samples` and never waited on. The default ring holds 30 s, which absorbs a slow partial decode.
* **Session isolation by construction.** Every `start_session` creates a new ring. `commit` and `cancel` take the producer away synchronously, so audio pushed after that (or before any start) is rejected (`false`, or C `1`). The worker drains a ring to its end after its producer is dropped.
* **Late context.** `set_context` may arrive at any time before `commit`. Context is read when commit runs, so screen words still reach the Whisper `initial_prompt` and the post-processing.
* **Partials.** With `partial_interval_ms` set, the worker re-decodes the last 12 s (2026-10-23; was 30 s) of the open audio, shown with a leading `"… "` when the window does not reach back. A preview is skipped once ≥ 400 ms of silence follows speech the last preview already covered. These partials are for preview only. `commit` waits for a decode in flight (`timings.wait_ms`): neither whisper's encoder nor a sherpa decode can be interrupted.
* **Pause segmentation** (D22, `SessionConfig.segment_pause_ms`, default off; recommended 800). `segment::Segmenter` runs the streaming VAD over the drained samples and cuts in the middle of each pause ≥ `segment_pause_ms` once the open piece is ≥ `segment_min_ms` (4 s), or at its longest ≥ 200 ms gap after `segment_soft_max_ms` (20 s). The cuts depend only on the audio (identical under any push chunking, on the server and in router fallback replay). The worker decodes each closed piece (VAD-trimmed, `AsrPurpose::Segment`, whisper `prior_text` = the last 30 words) before any preview, one decode per tick. At commit it reuses pieces whose request is unchanged, re-decodes a piece a late `set_context` changed, decodes the tail, and joins with one space. A per-piece guard drops stock-phrase-only pieces and strips a trailing "Thank you." from non-final ones. Then the usual pipeline runs on the joined text. `timings.asr_ms` = ASR inside commit; `pre_asr_ms` = before it; `segments` = pieces with speech. Engines with `segmentable() == false` (cloud) ignore it. `cancel()` stops a piece in flight through a per-session abort flag.
* **Hands-free endpointing** (D19, `SessionConfig.endpoint_silence_ms`, default off). `push_audio` also feeds a streaming copy of the VAD (`endpoint::Endpointer`: same 20 ms frames, speech bars and 60 ms runs as D16; noise floor from the last 30 s). After ≥ 300 ms of speech and `endpoint_silence_ms` (min 300; recommended 2000) of trailing silence, `poll().endpoint_reached` latches true for the rest of the session. It runs on the pushing thread (a few µs per 20 ms frame), so a slow partial decode on the worker cannot delay it. The core never commits by itself. `flow_router::Router` runs the same detector on its own pushes, so the signal is identical on every route.
* `max_samples` is a hard safety bound only; the session-length policy lives in the shells (spec §9). Audio past the bound is discarded and `truncated` is set.

## Commit pipeline

0. **VAD** (`vad`, `SessionConfig.vad`, default on). Energy + zero-crossing per 20 ms frame, noise floor = 10th-percentile frame RMS, speech = RMS ≥ clamp(3 × floor, 0.004, 0.015), or ≥ half that with ZCR ≥ 0.3 (fricatives), in runs of ≥ 60 ms. No speech → EMPTY `no_speech` without calling ASR. Otherwise the span from 300 ms before the first speech to 400 ms after the last goes to ASR (`timings.asr_audio_ms`). It fails open: a loud stationary signal counts as speech everywhere. Partials are not decoded until there is speech. Optional Silero: `ModelConfig.vad_model_path` (a whisper.cpp Silero ggml model such as `ggml-silero-v5.1.2.bin`, user-supplied, never downloaded; missing file = `ModelNotFound`) turns on whisper.cpp's built-in VAD in addition (`WhisperEngine::with_silero_vad`); untested here because no Silero model is on this box.
1. **ASR** (`AsrEngine` trait). `WhisperEngine` (feature `whisper`, `whisper-rs` 0.16) keeps one warm context and state.
   * `initial_prompt` = dictionary vocabulary (replacement targets and plain words; snippets excluded), then screen words, deduped, capped at 200 words.
   * Language: exactly one configured language is forced; zero or several means auto-detect. `.en` models are forced to `en`.
   * **Encoder window** (D20, `ModelConfig.whisper_fit_ctx`, default on): `audio_ctx` = clip + 1 s (50 positions/s, full window from 29 s up), so a 5 s dictation encodes 6 s instead of 30 s. A repetition/length guard (`whisper::looks_degenerate`) re-decodes with the full window; `WhisperEngine::redecodes()` counts it. 40 clips: tiny.en 2.6×, base.en 1.8×, small 3.4× faster; WER −0.4 / +0.3 / +0.1 points.
   * Silence gate: RMS < 0.003 returns empty. Input shorter than 1.1 s is padded. `[BLANK_AUDIO]` / `(music)` annotations are removed.
   * Default threads = logical CPUs / 2. On this HT box, 8 threads was 14× slower than 4.
   * `AsrOutput.no_speech_prob`: the highest per-segment Whisper no-speech probability.
   * **Hallucination guard** (`hallucination`, `SessionConfig.hallucination_guard`, default on). Whisper's stock silence phrases ("Thank you.", "you", "Thanks for watching!", "Bye.", repeated forms) are dropped (EMPTY `hallucination`) only with evidence of little speech: VAD speech < 400 ms or `no_speech_prob` ≥ 0.6. Subtitle credits ("Subtitles by the Amara.org community") are always dropped. Table: `tests/hallucination_vectors.rs`.
1b. **Spoken cleanup** (`postprocess::spoken`, `SessionConfig.spoken_cleanup`, default on, off at level `none`, skipped for whole-utterance snippets). Deterministic: spoken punctuation ("comma", "period", "question mark", "new line", "new paragraph", …, not after "the/a/trial/Oxford…"), fillers (um/uh/er/hmm; ", like," and ", you know," between commas; "was like $40"; a leading "so"/"okay so"), stutters ("I I" → "I"; not "that that"/"had had"), backtracking ("scratch that", "never mind", "forget that", "no wait", "actually I mean", "or rather"; the correction replaces from its first word's last match in the sentence, else the retracted clause back to its comma, else the last word; literal uses such as "don't forget that", "there's no wait", sentence-initial "Never mind the …" are left alone) and ITN for money ("$120", "$25.50"), percent, clock times ("3:30 PM", "4pm" → "4 PM") and dates ("March 5th"). Its output feeds the LLM and the Wispr port; text that becomes empty is EMPTY `only_filler`. On the benchmark's 20 cases (`tests/spoken_bench.rs`) it lifts mean similarity from 0.710 (raw) to 0.920 without an LLM, never worse than raw (the few-shot Qwen2.5-1.5B LLM scored 0.876 on the same set). The Wispr vectors call `postprocess::process`, which does not include this pass.
   **Layout** (`spoken::layout`, same gate, 2026-10-22), the deterministic list and email layout rules: ≥ 3 enumerated items become a numbered list in the format `cleanup::lists_to_plain` gives the LLM's `<ol>` (`Intro:\n\n1. …\n2. …`). Markers are ordinals 1, 2, 3, … in order ("first … second … and third/finally"; not after a determiner: "the first time", "wait a second"), "number/step one …", or a bare "one," / "1." at a clause start; 2 items stay prose; the last item ends at its sentence end and the rest becomes a paragraph; not in terminals. In email apps a spoken greeting ("Hi Tom,", ≤ 3 words) gets its own line and a spoken sign-off ("Best regards, Anna." / "Thanks, Raj.") its own paragraph with the name on the next line; nothing is invented ("Thanks." without a name stays). Bench 20 cases: 0.920 → 0.957 (c09, c11, c12 now exact; c08's unordered list is left to the LLM).
2. **LLM cleanup** (`Cleaner` trait, optional).
   * It is skipped for raw mode, level `none` and terminal apps.
   * It is also skipped when the whole utterance is a snippet trigger.
   * `cleanup::build_prompt` uses a short system prompt (one rule paragraph from the benchmark's few-shot variant plus one line per level) and 7 worked chat turns (fillers, "no wait" + time, spoken punctuation + money, ordered list, email layout, a question that must not be answered, filler-only → empty). The long 03 §7.2 rule list made ≤3B models echo or answer the input (docs/model-benchmarks.md on the bench branch). Cursor before/after text (≤300/150 chars), the selection, the dictionary and screen words go in the user message; a `STYLE:` line appears only for non-default styles.
   * HTML `<ol>`/`<ul>` lists in the LLM output become `1.` / `- ` lines.
   * Prompt layout is chosen for KV-cache reuse. The system prompt and the few-shot turns depend only on the level. Everything per dictation goes in the final user message: app, category, style, non-empty context lines and the transcript.
   * **Prefill-ahead** (`SessionConfig.llm_prefill`, default on, only when the LLM would run). While the user speaks, the worker sends `Cleaner::prefill` the prompt the commit will probably use: at session start (empty transcript), after every `set_context`, and after every new partial. The plugin coalesces stale requests, prefills in 32-token chunks and is preempted by a commit (`Cleaner::cancel_prefill` is called before the final ASR so the two do not fight for the CPU; what was decoded stays cached). Commit then decodes only from the first token that differs.
   * The cleaner runs on its own thread with a hard timeout (`SessionConfig.llm_timeout_ms`, default 1.5 s). On timeout the core raises a cancel flag (`Cleaner::clean_cancellable`) so the backend frees its context within one decode step.
   * Output scrubbing: preambles ("Here is the cleaned text:"), trailing "Note:" commentary and one layer of quotes or guillemets are stripped.
   * Guardrail (`guard::check`, replaces the 03 §7.1 edit-distance ratio). *Support*: every output sentence's content words must mostly occur in the input (≥ 75 %; 1-2-word sentences fully), and no sentence may repeat → else `llm_unsupported`. *Coverage*: every clause of the spoken-cleaned input must keep ≥ 50 % of its content words and must not lose two in a row, except the clause a backtracking phrase retracts → else `llm_dropped`. Number words match their digits. On the benchmark's 120 judged few-shot outputs (`tests/guard_bench.rs`) it accepts 79/79 paste-ready and rejects 14/14 harmful outputs; the old ratio > 0.6 rejected 4/79 paste-ready and passed 10/14 harmful (20 cases, one rater: treat as a regression fixture, not a precision estimate). `divergence_threshold` set explicitly re-enables the old check in addition.

### ASR backend: Parakeet (feature `parakeet`, crate `flow-asr-sherpa`)
D4 makes Parakeet-TDT-0.6B-v2 int8 the English default on desktops with ≥ 4 GB RAM (`docs/model-benchmarks.md`: WER 1.2% vs 3.7% for whisper base, RTF 0.14).

* **Selecting it.** `ModelConfig` JSON `{"asr_engine":"parakeet","asr_path":"<model dir>","threads":4}` (`asr_backend` works too). `asr_path` is the sherpa-onnx model *directory* (`encoder/decoder/joiner*.onnx`, `tokens.txt`; `*.int8.onnx` preferred). The build must enable `parakeet` (flow-core, flow-ffi or flow-cli); otherwise `load_models` fails with "built without the `parakeet` feature". CLI: `flow-cli transcribe --engine parakeet --model <dir> file.wav`. Whisper stays the default, and the only choice for non-English or auto-detect; the shell picks the engine (English-only user + model present → parakeet).
* **Packaging.** It uses the official `sherpa-onnx` crate (=1.13.8, k2-fsa, safe wrapper over the C API) with `features = ["shared"]`. It links `libsherpa-onnx-c-api.so` (3.6 MB) + `libonnxruntime.so` (27 MB, x86_64). No plugin is needed, unlike llama.cpp (D10): onnxruntime does not contain ggml, so whisper.cpp and Parakeet share one binary. Proof: `flow-cli` with whisper + llm + parakeet runs both engines, and the `--all-features` test suite (whisper_e2e, parakeet_e2e, llm_e2e, JNI) passes in one build.
  * Build: `SHERPA_ONNX_LIB_DIR=<dir with the two .so>`. Without it, `sherpa-onnx-sys` downloads the v1.13.8 prebuilt archive from GitHub at **build** time. The runtime never touches the network.
  * The build copies the .so files next to the outputs in `target/<profile>/`. `flow-cli/build.rs` and `flow-asr-sherpa/build.rs` add `-rpath,$ORIGIN`, because sherpa-onnx-sys's own rpath arg only applies to its own crate. Shells ship both .so files next to the exe (Windows: `sherpa-onnx-c-api.dll` + `onnxruntime.dll`) or in `jniLibs/arm64-v8a` (Android, see below).
  * Cost when the feature is off: none. The feature is not in any default feature set.
* **Engine.** One `OfflineRecognizer` stays warm (the three onnxruntime sessions), and each decode uses a fresh stream. The silence gate (RMS < 0.003) returns empty, and input shorter than 1 s is padded. Language is always `en`. A sherpa decode cannot be interrupted, so the abort flag is only checked before the decode. That is acceptable at RTF ≈ 0.1-0.15, but a live partial cannot be cut short.
* **Contextual biasing (hotwords): supported.** sherpa-onnx's NeMo transducer runs `modified_beam_search` with per-stream hotwords (an Aho-Corasick context graph), TDT included.
  * The adapter turns the whisper `initial_prompt` vocabulary (dictionary words and replacement targets, then screen words, ≤ 200 words) back into phrases (`parakeet::hotwords_from_prompt`, round-trip unit-tested against `build_initial_prompt`). It passes them `/`-separated, deduped and capped at 100 phrases.
  * The NeMo export ships no `bpe.vocab`, so `flow-asr-sherpa` derives one from `tokens.txt` with score = -id, which is sentencepiece BPE merge order. It writes the vocab to the temp dir (or `bpe.vocab` in the model dir if one is present) and deletes it once loaded. If it cannot be written, the engine falls back to greedy search without hotwords. **Android: the shell should set `TMPDIR` to the app cache dir, or ship `bpe.vocab` in the model dir.**
  * A flat vocab (all scores 0) was harmful: words split into single letters, every letter earned the bonus, and an unrelated hotword was inserted everywhere.
  * Measured (`tools/probes/parakeet_wer.py`, results in `tools/probes/results/`). Extended 40-clip set, 22 hotwords: 3 relevant (Battleax, Meadowcroft, "Vive le roi") and 19 unrelated tech names:

    | decode | WER | surface-only word changes vs greedy (case/punct) |
    |---|---|---|
    | greedy, no hotwords | 1.2% (11/955) | 0 |
    | beam 4, no hotwords | 1.2% | 4 |
    | beam 4, hotwords score 0.5 / 1.0 / 1.5 | 0.9% (9/955) each | 12 / 17 / 22 |
    | beam 2, hotwords 1.5 | 2.1% (split apostrophes, "Persocratic") | — |
    | 3 clips, one nonsense hotword, score 3.5 / 5 | nonsense hotword inserted / garbage | — |

    Defaults: beam 4, score 1.0. Higher scores fix more names but re-case ordinary words ("the Captain", "Fir Tree"). `FLOW_PARAKEET_HOTWORDS_SCORE` overrides the score; `0` means greedy with no hotwords.
  * Proof: `tests/parakeet_e2e.rs`. LibriSpeech 5142-36377-0008 gives "Meadocroft" unbiased and "Meadowcroft" with the dictionary word.
* **Measured** (i7-6700, 4 threads, load 6-25 from other jobs, release):
  * WER/RTF spot check on the 40-clip extended set: WER **1.2%** (11/955), the same errors as the bench, with greedy RTF 0.16. Core 13-clip set: WER 0.6%, RTF 0.10-0.12 greedy and 0.14-0.20 beam 4. On a quiet box, beam 4 costs about +20-40% over greedy, though the noise is as large as the effect.
  * jfk.wav (11 s) through `flow-cli --engine parakeet`: ASR 0.9-1.6 s.
  * **Preload / cold start:** `load_models` takes 2.9-4.0 s with the model in the page cache and 10.3-11.2 s cold (page cache evicted, HDD). Beam vs greedy makes no difference to load time. The bench saw 12-47 s cold under heavier load. RSS is about 880 MB. Shells must therefore load at app start or on the first hotkey press, never per dictation, and keep the core resident.
* **Android arm64:** built, see "Android: Parakeet" below.
* **flow-core lines touched** (all additive, for the merge with `core-p1`):
  * `crates/flow-core/Cargo.toml`: the `parakeet` feature, the optional `flow-asr-sherpa` dependency, and the `[[test]] parakeet_e2e` entry.
  * `src/lib.rs`: `#[cfg(feature = "parakeet")] pub mod parakeet;` after `pub mod llama;`.
  * `src/config.rs`: the `AsrBackend::Parakeet` variant (after `Whisper`), `#[serde(alias = "asr_engine")]` and its doc comment on `ModelConfig.asr_backend`, and the test `asr_engine_selector_defaults_to_whisper`.
  * `src/engine.rs` `load_models`: two match arms, `#[cfg(feature = "parakeet")] AsrBackend::Parakeet` and its `not(feature)` error twin, inserted after the `not(feature = "whisper")` arm. core-p1 rewrote the `Whisper` arm just above, so expect a trivial textual conflict there.
  * New files: `src/parakeet.rs`, `tests/parakeet_e2e.rs`. `AsrRequest` and `AsrOutput` are unchanged; the adapter builds `AsrOutput` with `..Default::default()`, so core-p1's `no_speech_prob` field merges cleanly.
  * Outside flow-core: the workspace `members`, `flow-ffi`/`flow-cli` `parakeet` features, `flow-cli --engine`, `flow-cli/build.rs`, and `tools/fetch-models.sh --parakeet`.

### Android: Parakeet (arm64-v8a)
**Feasible, and built; not yet run on a device.** `tools/build-android-parakeet.sh` (NDK 27.2.12479018, cargo-ndk, API 33) cross-builds `libflow_ffi.so` with `jni,whisper,parakeet` into `target/android-parakeet/jniLibs/arm64-v8a/`. It links against the official prebuilt `sherpa-onnx-v1.13.8-android.tar.bz2` (`jniLibs/arm64-v8a`; pass it as `SHERPA_ANDROID_LIBS`, otherwise sherpa-onnx-sys downloads it at build time). `apps/` is not touched. `tools/probes/check_parakeet_so.sh` checks the output:

| file | size | notes |
|---|---|---|
| `libflow_ffi.so` | 5.4 MB (was 3.9 MB with whisper only) | all 11 JNI contract symbols; NEEDED: `libc++_shared.so`, `libsherpa-onnx-c-api.so`, bionic |
| `libsherpa-onnx-c-api.so` | 4.5 MB | prebuilt; NEEDED `libonnxruntime.so`, `libandroid`, `liblog`; its libc++ is static |
| `libonnxruntime.so` | 22.2 MB | prebuilt 1.28 (CPU EP; NNAPI/XNNPACK not enabled) |
| `libc++_shared.so` | 1.8 MB | already shipped for whisper |

Every LOAD segment of all four is 16 KB aligned: k2-fsa's prebuilts already are, and flow-ffi gets the same `-z max-page-size=16384` flags as `build-native.sh`. The sherpa `libsherpa-onnx-jni.so` / `-cxx-api.so` are not needed, because flow-ffi uses the C API.

APK packaging, for the Android shell owner:
1. Copy the four .so files into `apps/flow-android/app/src/main/jniLibs/arm64-v8a/`, or point `tools/build-native.sh` at the same features. bionic resolves `libsherpa-onnx-c-api.so` and `libonnxruntime.so` from the app's lib dir when `System.loadLibrary("flow_ffi")` runs; no extra `loadLibrary` calls are needed.
2. Keep `jniLibs.useLegacyPackaging = false` (stored + 16 KB zip-aligned, as today). The APK grows by about 28 MB uncompressed. If that matters, a "parakeet" product flavor can ship the libs only there; flow-ffi without the feature does not reference them.
3. The model (661 MB: `encoder.int8.onnx` 652 MB, `decoder` 7 MB, `joiner` 2 MB, `tokens.txt`) is user-imported like the whisper model (D9, SAF) into a directory under `filesDir`. `loadModels` takes `{"asr_engine":"parakeet","asr_path":"<that dir>","threads":4}`.
4. Before `loadModels`, call `Os.setenv("TMPDIR", cacheDir.path, true)`. The derived `bpe.vocab` for hotwords is written there; otherwise hotwords are silently disabled. Alternatively, write `bpe.vocab` into the model dir at import (`flow_asr_sherpa::bpe_vocab_from_tokens`).
5. Expect about 0.9 GB RSS (desktop measurement) and several seconds of load. Offer the engine only on phones with ≥ 6-8 GB RAM, preload it in the keep-alive service, and fall back to whisper on load failure. Phone RTF is unmeasured. The bench estimate is ≈ 1-2.5 s for a 10 s dictation on a flagship (`docs/model-benchmarks.md`).

Not yet verified: loading on bionic (first device run), and whether whisper's libc++_shared and sherpa's static libc++ coexist at runtime. They coexist on desktop, and the C API boundary passes no C++ objects.

### LLM backend (feature `llm`, crate `flow-llm`)
`load_models` with `llm_path` set loads the GGUF model once and keeps it warm. `unload` (or drop) frees the model and its context. A bad path, a missing plugin or a build without `llm` fails `load_models`.

* **Packaging.** llama.cpp cannot be linked into the same binary as whisper.cpp: `whisper-rs-sys` and `llama-cpp-sys-2` each bundle a different ggml, and the link fails with duplicate `ggml_*` symbols. So the engine is the `flow-llm` cdylib, which exports only `flow_llm_*` (C ABI v1, documented in `crates/flow-llm/src/lib.rs`; chat messages travel as JSON). flow-core's `llm` feature is only a `libloading` loader (`flow_core::llama::LlamaCleaner`).
  * Plugin lookup: `ModelConfig.llm_lib`, else `FLOW_LLM_LIB`, else next to the executable (and its parent directory, for cargo test binaries), else the bare name through the system loader (Android: the packaged `libflow_llm.so`).
  * Shells ship `libflow_llm.so` / `flow_llm.dll` next to the app.
  * The library is never `dlclose`d. Closing a library that owns OpenMP threads and TLS destructors segfaulted at exit.
* **Engine.** One thread owns the model, one 4096-token context and its KV cache.
  * It renders the model's own chat template (Qwen2.5, Llama-3.2), with a ChatML fallback.
  * Prefix reuse: only the tokens after the first difference with the previous prompt are prefilled. Optional ABI-1 symbols `flow_llm_prefill(h, messages)` (non-blocking) and `flow_llm_prefill_cancel(h)` drive prefill-ahead; flow-core looks them up lazily, so an older plugin still loads. `flow_llm_info` adds `ahead{requests, completed, preempted, tokens, ms}`. `load` prefills the default (light, default-style) prompt, so a warm commit prefills only about 20 tokens of a ~1.2k-token prompt.
  * Decoding is greedy with at most 2 × transcript tokens + 32 new tokens (03 §7.5), stopping on EOG or a stop string (`<|im_end|>`, `<|eot_id|>`, `\nTRANSCRIPT:`, …). The deadline and cancel flag are checked between 128-token prefill chunks and before every token.
  * Exact-greedy prompt-lookup drafting is implemented, but off by default (`FLOW_LLM_DRAFT=n` enables it). On this CPU a batch of drafted tokens cost more than the forward passes it saved (`tools/probes/llm_ab.sh`).
  * llama.cpp logs are silenced unless `FLOW_LLM_LOG` is set.
* **Measured** (`tests/llm_e2e.rs`, 8 messy dictations; i7-6700, 4 threads, box shared with a concurrent llama-server at load 6-13, so treat the times as upper bounds):

  | model (Q4_K_M) | quality (generous budget) | llm per call | load + warm prefill |
  |---|---|---|---|
  | Qwen2.5-0.5B-Instruct | 4/8 (misses "scratch that", "no wait", filler "um", lists) | 0.75-1.1 s at load 6, up to 2.7 s at load 12 (then it falls back at 1.5 s) | 10-48 s |
  | Qwen2.5-1.5B-Instruct | 8/8 | 1.1-1.8 s at load 6, 2.6-5.3 s at load 12 | 23-38 s |

  Decode runs at 15-35 tok/s for 0.5B and 7-13 tok/s for 1.5B on this contended box. The first call after an app or style switch also prefills the user-message prefix (~100 tokens, +0.3-0.8 s).
3. **Deterministic post-processing** (`postprocess`). The dictionary matcher (look-around regex) is compiled once per distinct dictionary and shared by later sessions, instead of twice per commit (`FlowCore::dict_matcher_builds`, `postprocess::process_with`/`apply_with`). This is the client fallback path. The clone has no server, so it always runs the steps, in this order:
   1. all-lowercase mode;
   2. dictionary regex. Uses `fancy-regex` look-around, JS first-alternative order, longest key first, Unicode boundaries and the `'t` guard. Personal entries override team entries;
   3. cursor context: mid-sentence lowercasing, leading and trailing spaces, stripping `.!?` before a continuation, CJK handling, the selection path, the style-dependent trailing-period drop, and the Notion and Flow-app skips;
   4. Slack tagging (gated on the macOS bundle IDs, as in Wispr, so it never fires);
   5. the whole-utterance snippet trigger on raw ASR;
   6. email signature (off unless `email_signature` is set);
   7. "press enter" (sets `press_enter`; the shell sends the key);
   8. Cursor/Windsurf `@file` tagging. Emits `{{BREAK}}`/`{{TAB}}` markers for the paste layer.

   `postprocess::apps::classify` maps an app id or URL to its type, messaging flag, style bucket and category. It uses Wispr's Windows process-name lists and Android's package/host lists (both in `crates/flow-core/data/`). A shell-supplied `app_category` takes precedence.

### Platform differences (`SessionConfig.platform`)
Wispr Android shapes no text on the client; its server does it all (android 06). The clone runs the Windows order on Android too, with these gates:
* No whole-utterance snippet override.
* No signature.
* No press-enter.
* No IDE tagging.
* When `FieldContext.context_known == false` (the field could not be read), no trailing space is added.

`postprocess::android` ports the Android shell rules (post-commit verification, the node-tier compose decision, placeholder detection, the bubble policy, excluded fields) as pure functions. `bubble_policy(.., clone_safe=true)` applies the recommended deviation: a browser + GO action does not un-hide a password field.

### Intentional divergences from Wispr
* `starts_with_proper_noun` was computed by Wispr's server. The core derives it locally. It is true when the first word is a user name, a screen word or a dictionary word, or is the pronoun "I" or "I'…". This fixes the `ctx_pronoun_I_quirk` lowercasing. The vector is kept and still passes against the raw port (`context::match_context`). Only the engine path diverges.
* The press-enter first-use prompt is treated as already shown, so the command works the first time. The `enter_first_use_prompt` vector still passes with `prompt_shown=false`.

## Ground-truth tests
`crates/flow-core/tests/vectors.rs` loads every JSON file in `tests/vectors/`. Those files are copied verbatim from the RE repos; see the README there. The suite has:
* 108 Windows vectors, produced by executing Wispr's own functions.
* 58 Android vectors.
* A cross-check: Android inputs run through the clone rules must equal Wispr Windows' output for the same input.

To refresh the vectors, re-copy the files. A mutation check (sentence cap 2→3) confirms that the harness fails when the semantics drift.

## learn_edit
`align::align_words` does word-level Levenshtein with backtrace, labelling each step C/S/I/D as Wispr's `parseAlignmentSteps` does. `learn::learn_edit` then:
* takes substitution runs of ≤ 4 words;
* drops stopwords, contractions, existing dictionary words and sentence-initial capitalisation;
* caps the result at 4 candidates.

The cloud `/llm/extract_edited_words` step is left to the shell or to a future local LLM.

## Build / test
```
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
cargo test --all-features                 # + JNI round trip (needs `java`) + flow-llm plugin + llm_e2e
tools/fetch-models.sh [ggml-base.en.bin]  # explicit, user-run model download into models/
tools/fetch-models.sh --llm [0.5b 1.5b]   # Qwen2.5-Instruct Q4_K_M GGUF for LLM cleanup
tools/fetch-models.sh --parakeet          # Parakeet-TDT-0.6B-v2 int8 model dir (sherpa-onnx)
export SHERPA_ONNX_LIB_DIR=…/sherpa-onnx/lib   # optional; else sherpa-onnx-sys downloads its prebuilt libs at build time
cargo run --release -p flow-cli --features parakeet -- transcribe --engine parakeet --model models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 models/jfk.wav
cargo run --release -p flow-asr-sherpa --example spot_check -- <model dir> tools/probes/bench/asr_manifest_extended.json <audio dir> > out.jsonl
python3 tools/probes/parakeet_wer.py out.jsonl  # WER/RTF with the bench normaliser
cargo run --release -p flow-cli -- transcribe --model models/ggml-tiny.en.bin models/jfk.wav --json
cargo build --release -p flow-cli -p flow-llm   # plugin lands next to flow-cli
target/release/flow-cli clean --llm models/qwen2.5-0.5b-instruct-q4_k_m.gguf "um so scratch that hi"
cargo check -p flow-ffi --features jni --target aarch64-linux-android   # NDK needed only to link
```
* The model-dependent tests (`whisper_e2e`, `cli::real_model_on_jfk_if_present`) skip with a note when `models/` is missing.
* `whisper-rs-sys` is built at opt-level 3 even in dev builds. `llama-cpp-sys-2` builds llama.cpp as RelWithDebInfo (`-O2`) in dev builds anyway.
* `llama-cpp-sys-2` runs bindgen. On this box libclang 18 has no resource headers, so the build needs `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"` (otherwise: `'stdbool.h' file not found`).
* The LLM tests (`llm_e2e`, which skips without a model or plugin) take ~1 min. `FLOW_LLM_MODEL`, `FLOW_LLM_BUDGET_MS`, `FLOW_LLM_QUALITY_BUDGET_MS` and `FLOW_LLM_MIN_GOOD` tune them.

## Measurements (2026-10-20, i7-6700, 4 threads, box shared with other agents' builds, load 7-24)

**LLM prefill-ahead**, `flow-cli transcribe --realtime` on jfk.wav (11 s), tiny.en + Qwen2.5-1.5B Q4_K_M, context `{app_id: slack.exe, before: "Hey team,"}`, budget 10 s so every run finishes; counters are the commit's own prefill. Raw log: `docs/bench/llm-prefill-2026-10-20/` (old prompt, 1223 prompt tokens).

| partials | prefill-ahead | tokens prefilled at commit | commit prefill ms | llm ms |
|---|---|---|---|---|
| off | off | 81, 81 | 2834, 1255 | 8287, 2951 |
| off | on (context only) | 29 | 1591 | 7725 |
| every 1 s | off | 81, 81 | 2379, 1936 | 5351, 4046 |
| every 1 s | on | 21, 10 | 1249, 514 | 4865, 2699 |

Prefill-ahead removes 75-90 % of the commit's prefill (≈ 0.7-1.4 s here). Generation (27 tokens at 5-16 tok/s on this contended box: 1.7-6 s) still dominates, so the 1.5 s budget falls back to RAW_TRANSCRIPT on this box with 1.5B either way. One np_true rep (ASR 9.9 s, LLM timeout) was a load spike and is excluded.

## Changelog for shells

All additions are backward compatible: new JSON fields have serde defaults (old JSON still parses, extra fields are ignored by org.json/serde), new `SessionConfig` fields default to the recommended behaviour, and new C/plugin symbols are optional.

2026-10-23 (branch `feat/streaming-asr`, D22; shell steps in `docs/proposals/streaming-asr-shell-api.md`):
* `SessionConfig.segment_pause_ms` / `segment_min_ms` / `segment_soft_max_ms` (all `Option<u32>`, omitted when unset): pause-segmented incremental decoding. Adopt: send `segment_pause_ms: 800` behind a "Decode while speaking" toggle (default on); leave the other two unset.
* `endpoint::RECOMMENDED_SILENCE_MS` 2000 → **1500**. Adopt: hands-free default 1500 (explicit user values kept).
* `Timings.wait_ms`, `segments`, `pre_asr_ms` (omitted when 0); `asr_ms` = ASR time inside commit. `RouteInfo.client_ms` = router-side release → result wall time (includes the network). Adopt: show/log them in history.
* `ModelConfig.parakeet_decoding` (`auto` default | `greedy` | `beam`). Parakeet decodes greedy without dictionary hotwords, retries an empty beam result greedy, and decodes clips over 20 s in pieces at pauses (fixes EMPTY results and sentence drops on long dictations). `greedy` saves the second recognizer's RAM.
* Behaviour: the live-preview window is 12 s (leading `"… "` when it does not reach back), and previews pause once ≥ 400 ms of silence follows speech already previewed. No shell change.

2026-10-22 (branch `core-endpoint`):
* Hands-free endpointing (D19). `SessionConfig.endpoint_silence_ms: Option<u32>` (omitted from JSON when unset, so old servers see the same config). New `FlowCore::poll() -> SessionPoll` / `Router::poll()`: JSON `{"partial": str|null, "endpoint_reached": bool, "speech_ms": u64, "trailing_silence_ms": u64}`; plus `endpoint()`, `endpoint_reached()`. Adopt (shells; flow-ffi owner): export `flow_poll(h) -> json` (C) / `poll(h): String` (JNI) wrapping `Router::poll`; in hands-free mode start sessions with `endpoint_silence_ms: 2000` (a user pref, 300-10000), poll it on the existing partial timer, and on `endpoint_reached` run the normal finish path (stop sound + `commit` + paste), exactly as if the user had pressed the hands-free key. Push-to-talk should leave it unset.
  * Update (branch `router-route`): done — C `flow_poll` / `flow_start_session_info`, JNI `nativePoll` / `nativeStartSessionInfo`; `Router::poll()` now returns `RouterPoll` (`SessionPoll` + live `route`, derefs to `SessionPoll`). Step-by-step adoption for W and A: `docs/hybrid-architecture.md` "Changelog for shells", entry "live route + hands-free endpointing".
* `ModelConfig.whisper_fit_ctx` (D20, default on): Whisper encodes only the clip + 1 s, with a full-window re-decode on repetition. No shell change; a shell may expose it as a developer toggle ("Fast Whisper encoder") and send `false` to get the old behaviour.
* Behaviour: spoken cleanup now lays out numbered lists ("first … second … third" → `1.` lines) and, in email apps, greeting / sign-off lines without the LLM. `POSTPROCESS_REV` → `2026-10-22.1`. No shell change; the "Auto-format" pref (`spoken_cleanup`) turns it off too.

2026-10-20 (branch `core-p1`):
* `CommitResult.segments` / `plain_text` (CR-2). Adopt: paste `segments` in order (`Break` = pause, `Tab`/`Enter` = key) and stop acting on `press_enter` separately; store `plain_text` in history.
* `CommitResult.empty_reason`, `timings.asr_audio_ms`. Optional: show "no speech heard" for `no_speech`.
* `fallback_reason` gains `llm_unsupported`, `llm_dropped` (`llm_diverged` only with an explicit `divergence_threshold`).
* `CoreError::ModelNotFound{path}`, `CoreError::LlmLibNotFound{tried}`, `CoreError::kind()`; C `flow_last_error_kind()` (in `flow_core.h`) (CR-1). Adopt: Windows can drop its `is_file()` pre-check; Android can map `nativeLastError` prefixes `model not found:` / `llm plugin not found` (no JNI symbol was added).
* `SessionConfig` (all default on): `vad`, `hallucination_guard`, `spoken_cleanup`, `llm_prefill`. Shells may expose `spoken_cleanup` as a pref ("Auto-format punctuation, fillers and corrections"). `divergence_threshold` now defaults to off.
* Behaviour: room noise without speech is EMPTY (no ASR call); Whisper "Thank you." on a breath is EMPTY; fillers/spoken punctuation/backtracking/ITN are applied without the LLM; the LLM prompt is the short few-shot one.
* `FlowCore::llm_info()` (backend counters JSON); `Cleaner::prefill`/`cancel_prefill`/`info_json` default methods; `postprocess::spoken`, `guard`, `vad`, `hallucination`, `paste` modules are public.
* Breaking only for code that builds these structs by hand: `AsrOutput` gained `no_speech_prob` (and lost `Eq`); `LlamaCleaner::load` returns `LlamaLoadError`. No shell does either.

**VAD** (`tests/vad_bench.rs`, release, min of 2; synthetic room noise RMS ≈ 0.0023 padding / ≈ 0.005 noise-only):

| model | case | VAD off: ASR ms | VAD on: ASR ms | audio to ASR |
|---|---|---|---|---|
| tiny.en | jfk 11 s | 896 | 847 | 11.0 → 11.0 s |
| tiny.en | 3 s noise + jfk + 3 s noise | 879 | 803 | 17.0 → 11.6 s |
| tiny.en | 3 s noise only | 733 (blank) | **0** (EMPTY `no_speech`) | 3.0 → 0 s |
| base.en | jfk 11 s | 1809 | 1698 | |
| base.en | 3 s noise + jfk + 3 s noise | 1760 | 1780 | 17.0 → 11.6 s |
| base.en | 3 s noise only | 3915 (blank) | **0** | |

Trimming barely changes Whisper's time below 30 s, because its encoder always runs a full 30 s window (as the benchmark found); the win is skipping ASR entirely on presses without speech (0.7-3.9 s saved, and no chance of a silence hallucination). Trimming matters past 30 s (one fewer window per 30 s of silence) and for `audio_ctx` sizing later.

**Cleanup quality with the new pipeline** (spoken cleanup → few-shot LLM → guard → post), Qwen2.5-1.5B Q4_K_M, generous budget:
* benchmark 20 cases (`FLOW_LLM_BENCH=1 … llm_e2e bench_cases`): mean similarity **0.919** (raw 0.710; spoken cleanup alone 0.920; the benchmark's 1.5B few-shot alone 0.876); 19 FORMATTED, c17 EMPTY; no fallbacks; llm 0.9-3.9 s per case on this box. On CPU the LLM adds layout (email, lists, colons) but no net similarity over the rules.
* the 8 llm_e2e dictations: quality 6/8 (was 8/8 with the long prompt). The two misses: "milk comma eggs … period thanks", where the model dropped "thanks" and the guard fell back to the deterministic text (which is correct), and an unpunctuated "first … second … third" that the model wrote as three sentences instead of a numbered list. At the real 1.5 s budget, 5/8 FORMATTED (p50 1.0 s).
