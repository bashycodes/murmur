# Transforms (desktop) — spec

Source: design artboard `docs/design/app-design/PageTransforms.dc.html`. Built on Command Mode.

## Behaviour
- Master "Opt in" toggle (default off): when off, no transform chords are registered and auto-apply never runs.
- A transform = {name, description, instruction, shortcut}. Built-ins: **Polish** (improve clarity and conciseness) and
  **Prompt Engineer** (turns a rough ask into a clear prompt), in our own wording. Users can create/edit/delete;
  "Reset to defaults" restores built-ins and keeps custom ones.
- Shortcut press (no speech): wait for key release, capture the selection, run the instruction on it through the
  AI cleanup engines (OpenRouter, then this PC), paste the result over the selection. Pill shows processing.
  Nothing selected → notice "Select text to transform", nothing pasted.
- Auto-apply after dictation: the cleaned dictation goes through the chosen transform before paste. On failure or
  timeout (8 s) paste the untransformed text plus a "Transform skipped" notice — never lose a dictation.
- "View changes" chord (Win+Alt+O): opens Home with the last transform's history row expanded as before/after.
- Default chords Win+Alt+1/2… (they shadow taskbar jump lists only while Transforms is on; user-changeable).

## Reuse
Selection capture (`capture_selection`), `CoreApi::command` → `Router::command_cleaner` → `flow_core::command`
(consent, budget, OR→local order), `command::sanitize`, `Effect::Paste` with the S13 paste guard, command history rows.

## New
1. `crates/flow-core/src/transform.rs`: `Transform`, `builtins()`, `run_transform(cleaner, instruction, text,
   Source::{Selection, Dictation}, timeout)`. Selection keeps Command Mode's surrounding-text gate; Dictation doesn't
   (transcript is already in cleanup scope).
2. Prefs `transforms: {enabled, auto_apply: Option<id>, view_changes: Option<Chord>, items: [{transform, shortcut}]}`;
   migrate seeds built-ins; validate chords unique across all bindings; caps ~20 items, 2000 chars/instruction.
3. Hotkeys: `HotkeyAction::Transform(u8)`, `ViewChanges`; matcher registers transform chords only when enabled.
4. Dictation machine: idle + Transform(i) → Processing{command} + `Effect::Transform`; auto-apply via
   `Effect::AutoTransform` / `Input::AutoTransformFinished` with fallback to original text.
5. Controller: `run_transform`, `last_transform`; history `transform` column (name).
6. Tauri: `transforms_reset`, `transform_try(id, text)` (same consent gate); config rides on get/set_prefs.

## Privacy
Only OpenRouter (D28) and the on-device LLM receive text; never the server route. Selection without
`send_surrounding_text` → local model only. Auto-apply sends only the transcript under existing cleanup consent.
No new network path (D9). Debug log per D21.

## Chunks
T1 core; T2 prefs + hotkeys; T3 dictation/controller/history/commands; T4 UI page + editor + check_transforms.py.
Android parity later (core is shared).
