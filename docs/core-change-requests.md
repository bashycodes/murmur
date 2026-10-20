# flow-core change requests (from the shells)

The core is developed on its own branch. Shells do not edit `crates/` and
instead file requests here. Each entry says what the shell does in the meantime.

## CR-1: tell a missing model file apart from a bad one (Windows shell, 2026-10-20)

**Problem.** `FlowCore::load_models` reports every failure as `CoreError::Load(String)`. A shell cannot show "model not found, put a file at X" without parsing the message.

**Ask.** Add `CoreError::ModelNotFound { path }`, returned before whisper.cpp is invoked.

**Workaround.** `core_adapter::load_blocking` checks `path.is_file()` first and reports `ModelStatus::Missing`.

## CR-2: structured paste segments instead of in-band markers (Windows shell)

**Problem.** `cursor_tag` puts `{{BREAK}}`/`{{TAB}}` inside `final_text`. Every `@` gets a `{{BREAK}}`, even inside an email address. History and "paste last" then store the markers, and a shell that forgets to parse them pastes them literally.

**Ask.** Keep `final_text` marker-free and add `segments: [{text} | "tab" | "break"]` (or a `paste_plan` field) to `CommitResult`.

**Workaround.** `paste::segments` splits the markers, and marker-carrying text is typed segment by segment. History stores `final_text` as returned.

## CR-3: whisper-rs-sys cannot be cross-checked for MSVC from Linux (info)

**Problem.** `cargo check --target x86_64-pc-windows-msvc --features whisper` fails in whisper-rs-sys' build script (cmake/C++, plus a `bindgen` that has no MSVC headers).

**Ask.** Nothing blocking. A future core option is a `whisper-prebuilt` feature that links a whisper.cpp built separately on Windows.

**Workaround.** The shell has a default-on `whisper` feature, and the Linux cross-check runs with `--no-default-features`.
