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
`ModelConfig.asr_backend` is either `"whisper"` or `{"mock":{"text":"…"}}`. The
mock lets shells be developed without a model.

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
cargo run --release -p flow-cli -- transcribe --model models/ggml-tiny.en.bin models/jfk.wav --json
cargo build --release -p flow-cli -p flow-llm   # plugin lands next to flow-cli
target/release/flow-cli clean --llm models/qwen2.5-0.5b-instruct-q4_k_m.gguf "um so scratch that hi"
cargo check -p flow-ffi --features jni --target aarch64-linux-android   # NDK needed only to link
```
* The model-dependent tests (`whisper_e2e`, `cli::real_model_on_jfk_if_present`) skip with a note when `models/` is missing.
* `whisper-rs-sys` is built at opt-level 3 even in dev builds. `llama-cpp-sys-2` builds llama.cpp as RelWithDebInfo (`-O2`) in dev builds anyway.
* `llama-cpp-sys-2` runs bindgen. On this box libclang 18 has no resource headers, so the build needs `BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/13/include"` (otherwise: `'stdbool.h' file not found`).
* The LLM tests (`llm_e2e`, which skips without a model or plugin) take ~1 min. `FLOW_LLM_MODEL`, `FLOW_LLM_BUDGET_MS`, `FLOW_LLM_QUALITY_BUDGET_MS` and `FLOW_LLM_MIN_GOOD` tune them.
