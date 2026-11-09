# Command Mode turns Hebrew text into English (2026-11-01)

Branch: `feat/command-mode-hebrew`. Owner report: "Using the command mode
feature in Hebrew text changes it to English." The report doesn't say which
platform. Android and Windows share the code (below), so the fix covers both.
All text in this doc and in the tests is synthetic.

## Path

Every shell ends in `murmur_core::command::run`:

- Android: `DictationController.runCommand` → JNI `nativeCommand` →
  `murmur_ffi` → `Router::command` → `command::run`.
- Windows: `router_adapter.rs` / `core_api.rs` → `command::run`.
- Transforms (Windows) use `transform::run_transform` → `command::ask`.

The model that runs is the dictation cleanup route, narrowed for a
selection (`command::selection_route`). Cloud (OpenRouter) is used only
when "send surrounding text" is allowed. Otherwise the on-device model runs
(Qwen2.5-1.5B on the phone and the laptop). The server route never runs
commands. Command Mode passes no STT language or keyboard language to the
prompt: the instruction is transcribed as spoken (raw mode), then only the
instruction text and the selection go to the model. No guard,
`spoken_cleanup` or script check runs on the answer, only `command::sanitize`
(it strips preambles, quotes and fences, and doesn't touch the script). So
nothing after the model rewrote or rejected the Hebrew. The model itself
answered in English.

## Root cause

The prompt pushed the model toward English:

1. The system line said "Keep the meaning and the language of the text
   unless the instruction asks to change them". A 1.5B model doesn't hold
   that sentence against everything else in the prompt.
2. The only few-shot example was all English (instruction, text and answer).
3. The spoken instruction is usually English ("make it more formal"), so
   the user turn was English apart from the selection.

A small model copies the language of the example and the instruction. Point
2 also hit the Windows transforms (`transform.rs`, English example "Make
this clearer and more concise").

## Measurement (Qwen2.5-1.5B Q4_K_M, on-device engine, offline)

Bench: `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/runner/src/bin/command_bench.rs`,
14 synthetic cases in `command_he_cases.jsonl`: 10 Hebrew edits (formal,
shorter, polite, fix grammar, list, friendlier, expand, Hebrew with English
terms, an instruction spoken in Hebrew), 3 translations of Hebrew to English
(which must come out in Latin script), and 1 English control. Before = the
pre-fix prompt, one call. After = `command::run` as it ships now. Raw rows:
`results/command_he_{before,after}_q15.jsonl`.

| | Hebrew edits kept Hebrew | Translations in English | English control |
|---|---|---|---|
| before | 1/10 (only the list) | 3/3 | 1/1 |
| after | **10/10** | 3/3 | 1/1 |

Before, even a Hebrew instruction ("תנסח את זה יותר רשמי") came back in
English. After the fix, no answer was refused (`language_changed` 0/10).
The prompt alone kept the script; the retry and the refusal are a safety net.

**Quality caveat:** Qwen2.5-1.5B writes poor Hebrew. After the fix, several
rewrites barely change the text (the "formal" case returns the input). Two
answers contain stray Cyrillic or CJK fragments inside Hebrew words. The
script check lets those through, because the answer is still mostly Hebrew.
The language is now right, but the quality of a Hebrew rewrite needs a
bigger model (see the decision below).

## Fix

- `crates/murmur-core/src/script_lock.rs` (new) works out the dominant script
  of a text by Unicode block (Hebrew, Arabic, Cyrillic, Greek, Latin; at
  least 3 letters, a 60 % share). The lock applies when the selection is in
  a non-Latin script **and** the instruction doesn't ask for another
  language. "Asking" means a translate word ("translate", "transliterate",
  "תרגם", "ترجم", "перевед") or a language name other than the text's own
  ("in English", "באנגלית"; "keep it in Hebrew" keeps the lock). Latin text
  is never locked, because its letters can't tell English from Spanish.
- `command::build_prompt` / `transform::build_prompt`:
  - The system line now says to answer in the language of the selected
    text even when the instruction is in another language, and to change
    language only on an explicit request to translate.
  - With a lock, the user turn ends with "Write the result in Hebrew, the
    language of the selected text, …".
  - For Hebrew, the few-shot is Hebrew under an English instruction.
- `command::ask`: if an answer to a locked edit is less than 50 % the
  source script, it is retried once with a stronger line, provided at least
  `MIN_RETRY` (2 s) of the budget is left. If the retry also drifts (or
  there is no time for one), the result is the new
  `CommandError::LanguageChanged` (`language_changed`, "AI changed the
  text's language: text left unchanged"), and the selection stays as it
  was. An answer without letters (digits) never counts as drift.
- Wire vectors: `fixtures/command/command_results.json` adds the new kind,
  and `CommandOutcome.LANGUAGE_CHANGED` is added on Android (the decoder
  already passed unknown kinds through).

Speak-to-generate (nothing selected) is unchanged: it writes in the
language of the instruction.

## Tests

- `script_lock::tests` (5): script detection, which instructions unlock
  the language, Latin never locked, what counts as drift.
- `command::tests` (7 new): the prompt is locked and the few-shot is Hebrew;
  translating unlocks it; a Hebrew answer passes; an English answer is
  retried and then refused; no retry without time left; an explicit
  translation may change the language; Hebrew with English terms is kept.
- `transform::tests` (1 new): Polish on Hebrew is locked and refused on
  drift; "Translate to English." may change the language.
- `command_vectors.rs` and Kotlin `CommandVectorsTest`: 8 kinds.

## Owner decision

The language is now kept. Hebrew rewrite *quality* on the 1.5B on-device
model is still weak. Options:

1. Allow OpenRouter "send surrounding text" so Command Mode can use the
   cloud model on Hebrew selections.
2. Use a larger local model for commands.
3. Accept the current quality.

Not tested: the cloud model (costs money) and a real-device run.
