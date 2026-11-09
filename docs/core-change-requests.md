# flow-core change requests (from the shells)

The core is developed on its own branch. Shells do not edit `crates/` and
instead file requests here. Each entry says what the shell does in the meantime.

## CR-1: tell a missing model file apart from a bad one (Windows shell, 2026-10-20)

**Problem.** `FlowCore::load_models` reports every failure as `CoreError::Load(String)`. A shell cannot show "model not found, put a file at X" without parsing the message.

**Ask.** Add `CoreError::ModelNotFound { path }`, returned before whisper.cpp is invoked.

**Workaround.** `core_adapter::load_blocking` checks `path.is_file()` first and reports `ModelStatus::Missing`.

**Status: DONE (branch `core-p1`).** `CoreError::ModelNotFound{path}` (ASR and LLM model), plus `CoreError::LlmLibNotFound{tried}`, `CoreError::kind()` and C `flow_last_error_kind()`. See core-architecture "Changelog for shells".

## CR-2: structured paste segments instead of in-band markers (Windows shell)

**Problem.** `{{BREAK}}`/`{{TAB}}` markers travel inside `final_text` when the text carries them (e.g. a snippet body; flow-core no longer produces them since the IDE tagging was removed, D82). History and "paste last" then store the markers, and a shell that forgets to parse them pastes them literally.

**Ask.** Keep `final_text` marker-free and add `segments: [{text} | "tab" | "break"]` (or a `paste_plan` field) to `CommitResult`.

**Workaround.** `paste::segments` splits the markers, and marker-carrying text is typed segment by segment. History stores `final_text` as returned.

**Status: DONE (branch `core-p1`), additively.** `CommitResult.segments: [{text} | {key: "Enter"|"Tab"|"Break"}]` (Enter = spoken "press enter", last) and `plain_text` (marker-free). `final_text` keeps the markers so current shells keep working; the `@` → `{{BREAK}}` rule itself is unchanged.

## CR-3: whisper-rs-sys cannot be cross-checked for MSVC from Linux (info)

**Problem.** `cargo check --target x86_64-pc-windows-msvc --features whisper` fails in whisper-rs-sys' build script (cmake/C++, plus a `bindgen` that has no MSVC headers).

**Ask.** Nothing blocking. A future core option is a `whisper-prebuilt` feature that links a whisper.cpp built separately on Windows.

**Workaround.** The shell has a default-on `whisper` feature, and the Linux cross-check runs with `--no-default-features`.

## CR-4: typed LLM load errors (Windows shell, 2026-10-20)

**Problem.** `LlamaCleaner::load` returns `CleanError::Backend(String)` for every failure. The shell has to tell "flow_llm.dll not found" (a packaging problem: ship the DLL) from "model failed to load" (bad GGUF) by matching the prefix `"llm plugin not found"`.

**Ask.** `CleanError::PluginNotFound { tried: Vec<PathBuf> }` and `CleanError::ModelLoad(String)` (or a `kind` field), and `ModelNotFound` as in CR-1.

**Workaround.** `core_adapter::classify_llm_error` matches the message prefix; `llama_loader` checks `model_path.is_file()` first.

## CR-5: attach/detach the LLM on a running core (Windows shell, info)

**Problem.** `FlowCore::load_models` loads ASR and LLM together, so a 10-50 s LLM load would block dictation, and toggling cleanup would reload whisper.

**Ask.** Nothing blocking: the shell composes `WhisperEngine::load` + `LlamaCleaner::load` itself through `FlowCore::from_engines` and rebuilds the core when the cleaner changes. A `FlowCore::set_cleaner(Option<Arc<dyn Cleaner>>)` would avoid the rebuild (one extra worker thread per swap) and keep `load_models` the only entry point for other shells.

**Workaround.** Two-phase load in `core_adapter.rs` (module docs).

## CR-6: richer learn_edit result (Windows shell)

**Problem.** `learn_edit` returns `{original, corrected}` only. The shell bounds the edited region itself (`learn::bound_edit`, semi-global alignment) because a run that includes text typed after the dictation is otherwise dropped (> 4 words) or mislearned. Candidates that are only a case change of a screen word, or whose `corrected` appears in the field context, can't be told apart from real corrections (they come from the app context, not from the user).

**Ask.** Optionally accept the field context and return a `source` (`user_edits` / `app_context`) plus an `edited_bounded` string, so Android and Windows share the bounding rule.

**Workaround.** Bounding lives in `apps/flow-windows/src/learn.rs`; every candidate is a user-accepted suggestion, so a wrong one costs one "Dismiss".
