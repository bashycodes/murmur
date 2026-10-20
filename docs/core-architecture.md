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
 timings{audio_ms, asr_ms, llm_ms, postprocess_ms, total_ms, dropped_samples},
 press_enter, fallback_reason, truncated, error}
```
Status meanings:
* `FORMATTED`: the LLM cleanup ran and passed the guardrail.
* `RAW_TRANSCRIPT`: post-processed ASR text. This covers LLM off, LLM skipped (raw mode, level `none`, terminal) and LLM failed. `fallback_reason` says why it failed: `llm_timeout`, `llm_error: …`, `llm_diverged` or `llm_empty`.
* `EMPTY`: no audio, or blank ASR. Blank counts as empty, which is the recommended deviation from Wispr Android, where `"   "` is inserted.
* `ERROR`: ASR error, no session, or the session was cancelled or superseded.

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
* **Partials.** With `partial_interval_ms` set, the worker re-decodes the last 30 s window. These partials are for preview only. `commit` always decodes the whole utterance.
* `max_samples` is a hard safety bound only; the session-length policy lives in the shells (spec §9). Audio past the bound is discarded and `truncated` is set.

## Commit pipeline

1. **ASR** (`AsrEngine` trait). `WhisperEngine` (feature `whisper`, `whisper-rs` 0.16) keeps one warm context and state.
   * `initial_prompt` = dictionary vocabulary (replacement targets and plain words; snippets excluded), then screen words, deduped, capped at 200 words.
   * Language: exactly one configured language is forced; zero or several means auto-detect. `.en` models are forced to `en`.
   * Silence gate: RMS < 0.003 returns empty. Input shorter than 1.1 s is padded. `[BLANK_AUDIO]` / `(music)` annotations are removed.
   * Default threads = logical CPUs / 2. On this HT box, 8 threads was 14× slower than 4.
2. **LLM cleanup** (`Cleaner` trait, optional).
   * It is skipped for raw mode, level `none` and terminal apps.
   * It is also skipped when the whole utterance is a snippet trigger.
   * `cleanup::build_prompt` implements the windows-research 03 §7.2 template: Wispr's verbatim dictation framing, level/style descriptions and the backtrack list, plus a one-shot built from Wispr's UI example pair. Cursor before/after text (≤300/150 chars), the selection, the dictionary and screen words go in the user message.
   * HTML `<ol>`/`<ul>` lists in the LLM output become `1.` / `- ` lines.
   * Prompt layout is chosen for KV-cache reuse. The system prompt and six few-shot chat turns depend only on the level: Wispr's UI example pair plus one example each for backtracking, spoken punctuation with "no wait", an ordered list, a question that must not be answered, and email layout. Everything per dictation goes in the final user message: app, category, style bucket, non-empty context lines and the transcript.
   * The cleaner runs on its own thread with a hard timeout (`SessionConfig.llm_timeout_ms`, default 1.5 s). On timeout the core raises a cancel flag (`Cleaner::clean_cancellable`) so the backend frees its context within one decode step.
   * Output scrubbing: preambles ("Here is the cleaned text:"), trailing "Note:" commentary and one layer of quotes or guillemets are stripped.
   * Guardrail: word-level divergence between ASR and LLM output (edit distance / words) above a per-level threshold (light 0.6, medium/high 0.9; 03 §7.1) means the output is not used and the result falls back to `RAW_TRANSCRIPT`.

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
* **Android arm64:** see "Android: Parakeet" below.
* **flow-core lines touched** (all additive, for the merge with `core-p1`):
  * `crates/flow-core/Cargo.toml`: the `parakeet` feature, the optional `flow-asr-sherpa` dependency, and the `[[test]] parakeet_e2e` entry.
  * `src/lib.rs`: `#[cfg(feature = "parakeet")] pub mod parakeet;` after `pub mod llama;`.
  * `src/config.rs`: the `AsrBackend::Parakeet` variant (after `Whisper`), `#[serde(alias = "asr_engine")]` and its doc comment on `ModelConfig.asr_backend`, and the test `asr_engine_selector_defaults_to_whisper`.
  * `src/engine.rs` `load_models`: two match arms, `#[cfg(feature = "parakeet")] AsrBackend::Parakeet` and its `not(feature)` error twin, inserted after the `not(feature = "whisper")` arm. core-p1 rewrote the `Whisper` arm just above, so expect a trivial textual conflict there.
  * New files: `src/parakeet.rs`, `tests/parakeet_e2e.rs`. `AsrRequest` and `AsrOutput` are unchanged; the adapter builds `AsrOutput` with `..Default::default()`, so core-p1's `no_speech_prob` field merges cleanly.
  * Outside flow-core: the workspace `members`, `flow-ffi`/`flow-cli` `parakeet` features, `flow-cli --engine`, `flow-cli/build.rs`, and `tools/fetch-models.sh --parakeet`.

### LLM backend (feature `llm`, crate `flow-llm`)
`load_models` with `llm_path` set loads the GGUF model once and keeps it warm. `unload` (or drop) frees the model and its context. A bad path, a missing plugin or a build without `llm` fails `load_models`.

* **Packaging.** llama.cpp cannot be linked into the same binary as whisper.cpp: `whisper-rs-sys` and `llama-cpp-sys-2` each bundle a different ggml, and the link fails with duplicate `ggml_*` symbols. So the engine is the `flow-llm` cdylib, which exports only `flow_llm_*` (C ABI v1, documented in `crates/flow-llm/src/lib.rs`; chat messages travel as JSON). flow-core's `llm` feature is only a `libloading` loader (`flow_core::llama::LlamaCleaner`).
  * Plugin lookup: `ModelConfig.llm_lib`, else `FLOW_LLM_LIB`, else next to the executable (and its parent directory, for cargo test binaries), else the bare name through the system loader (Android: the packaged `libflow_llm.so`).
  * Shells ship `libflow_llm.so` / `flow_llm.dll` next to the app.
  * The library is never `dlclose`d. Closing a library that owns OpenMP threads and TLS destructors segfaulted at exit.
* **Engine.** One thread owns the model, one 4096-token context and its KV cache.
  * It renders the model's own chat template (Qwen2.5, Llama-3.2), with a ChatML fallback.
  * Prefix reuse: only the tokens after the first difference with the previous prompt are prefilled. `load` prefills the default (light, default-style) prompt, so a warm commit prefills only about 20 tokens of a ~1.2k-token prompt.
  * Decoding is greedy with at most 2 × transcript tokens + 32 new tokens (03 §7.5), stopping on EOG or a stop string (`<|im_end|>`, `<|eot_id|>`, `\nTRANSCRIPT:`, …). The deadline and cancel flag are checked between 128-token prefill chunks and before every token.
  * Exact-greedy prompt-lookup drafting is implemented, but off by default (`FLOW_LLM_DRAFT=n` enables it). On this CPU a batch of drafted tokens cost more than the forward passes it saved (`tools/probes/llm_ab.sh`).
  * llama.cpp logs are silenced unless `FLOW_LLM_LOG` is set.
* **Measured** (`tests/llm_e2e.rs`, 8 messy dictations; i7-6700, 4 threads, box shared with a concurrent llama-server at load 6-13, so treat the times as upper bounds):

  | model (Q4_K_M) | quality (generous budget) | llm per call | load + warm prefill |
  |---|---|---|---|
  | Qwen2.5-0.5B-Instruct | 4/8 (misses "scratch that", "no wait", filler "um", lists) | 0.75-1.1 s at load 6, up to 2.7 s at load 12 (then it falls back at 1.5 s) | 10-48 s |
  | Qwen2.5-1.5B-Instruct | 8/8 | 1.1-1.8 s at load 6, 2.6-5.3 s at load 12 | 23-38 s |

  Decode runs at 15-35 tok/s for 0.5B and 7-13 tok/s for 1.5B on this contended box. The first call after an app or style switch also prefills the user-message prefix (~100 tokens, +0.3-0.8 s).
3. **Deterministic post-processing** (`postprocess`). This is the client fallback path. The clone has no server, so it always runs the steps, in this order:
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
