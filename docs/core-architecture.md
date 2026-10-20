# flow-core architecture

Shared Rust core of the local Wispr Flow clone. It implements the §9 v2 API from
`windows-research/findings/by-topic/10-clone-spec.md` (Android alignment in
`10-clone-spec-android.md` §9). All speech recognition and text shaping run
on-device; the core makes no network calls.

```
crates/
  flow-core/   lib: session engine, ASR/LLM seams, deterministic post-processing
  flow-ffi/    C ABI (cdylib/staticlib, include/flow_core.h) + JNI (feature `jni`)
  flow-cli/    dev harness: `flow-cli transcribe` = full pipeline on a WAV
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
   * `cleanup::build_prompt` builds the prompt from the level, style, app category, cursor text and dictionary.
   * The cleaner runs on its own thread with a hard timeout (default 1.5 s).
   * Guardrail: word-level divergence between ASR and LLM output (edit distance / words) above a per-level threshold (light 0.5, medium 0.7, high 0.9) means the output is not used and the result falls back to `RAW_TRANSCRIPT`.
   * The llama.cpp backend (feature `llm`) is **not wired yet**. `load_models` ignores `llm_path`.
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
cargo test --all-features                 # + JNI round trip in a real JVM (needs `java`)
tools/fetch-models.sh [ggml-base.en.bin]  # explicit, user-run model download into models/
cargo run --release -p flow-cli -- transcribe --model models/ggml-tiny.en.bin models/jfk.wav --json
cargo check -p flow-ffi --features jni --target aarch64-linux-android   # NDK needed only to link
```
* The model-dependent tests (`whisper_e2e`, `cli::real_model_on_jfk_if_present`) skip with a note when `models/` is missing.
* `whisper-rs-sys` is built at opt-level 3 even in dev builds.
