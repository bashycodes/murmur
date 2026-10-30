// Settings → General: shortcuts, voice and theme
// (docs/design/app-design/SettingsGeneral.dc.html). Fields and the
// keycap / Change editors: features/general.ts.

import { group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";

/** Keycaps of the saved shortcut + Change; Change records the keys the
 *  user presses ("Type instead" swaps in the text field). */
const key = (id: string, label: string): string =>
  `<div class="lw-edit" data-edit="${id}">` +
  `<span class="fl-kbd-group lw-edit-view" id="${id}_view"></span>` +
  `<input id="${id}" class="fl-input lw-edit-input" data-save="prefs" spellcheck="false" aria-label="${label} shortcut" hidden />` +
  `<button type="button" class="lw-link lw-edit-type" hidden>Type instead</button>` +
  `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm lw-edit-btn">Change</button></div>`;

const select = (id: string, options: [string, string][], cls = "lw-w160"): string =>
  `<select id="${id}" class="fl-select ${cls}" data-save="prefs">${options
    .map(([v, l]) => `<option value="${v}">${l}</option>`)
    .join("")}</select>`;

export const settingsGeneralPage: Page = {
  id: "settings-general",
  area: "settings",
  template: `
${heading("General")}
${group(
  "Shortcuts",
  row("Push to talk", "Hold, speak, let go to paste.", key("ptt", "Push to talk")) +
    row("Hands-free", "Or double-tap push to talk. Stops by itself when you go quiet.", key("hands_free", "Hands-free")) +
    row(
      "Command Mode",
      "Hold and say an instruction: rewrites the selected text (or writes new text at the cursor) with the AI cleanup model.",
      key("command", "Command Mode"),
    ) +
    row("Cancel", "", key("cancel", "Cancel")) +
    row("Paste last dictation", "", key("paste_last", "Paste last dictation")) +
    row("Copy last dictation", "", key("copy_last", "Copy last dictation")) +
    row("Open Scratchpad", "", key("scratchpad", "Open Scratchpad")) +
    row("New note and dictate", "", key("new_note", "New note and dictate")),
)}
<p class="fl-help lw-group-help" id="keys_help" hidden>Type the keys joined by +, e.g. <code>Ctrl+Win</code>. Keys: Ctrl, Win, Alt, Shift, Space, Esc, A–Z, 0–9, F1–F24. Ctrl/Alt/Shift match either side; Left Alt, Right Alt (also Left/Right Ctrl, Shift) match one side only, e.g. push to talk = Right Alt. Max 3 keys, at least one modifier. Empty = off (Scratchpad, New note). Enter saves, Esc cancels.</p>
${group(
  "Voice",
  row("Microphone", "", `<select id="microphone" class="fl-select is-wide" data-save="prefs" aria-label="Microphone"></select>`) +
    row(
      "Languages",
      `<span id="languages_desc"></span><span id="languages_parakeet" hidden> Parakeet only understands English; <button type="button" class="lw-link" data-go="settings-speech">switch to Whisper →</button> for other languages.</span>` +
        `<span id="languages_hebrew" class="lw-status-sub hint err" hidden>No Hebrew model is downloaded, so Hebrew uses your default speech model. <button type="button" class="lw-link" data-go="settings-speech">Download the Hebrew model →</button></span>`,
      `<select id="language_pick" class="fl-select lw-w160" aria-label="Language">` +
        `<option value="">Detect automatically</option><option value="en">English</option><option value="he">Hebrew</option><option value="other">Other…</option></select>` +
        `<div class="lw-edit" data-edit="languages" id="languages_other" hidden>` +
        `<input id="languages" class="fl-input lw-edit-input" data-save="prefs" spellcheck="false" placeholder="e.g. en, de (empty = detect)" aria-label="Languages" hidden />` +
        `<button type="button" class="fl-btn fl-btn-secondary lw-edit-btn">Change</button></div>`,
    ) +
    row(
      "Hands-free stops after",
      "How long you can pause before dictation ends.",
      select("hands_free_silence_s", [
        ["1", "1 second"],
        ["1.5", "1.5 seconds"],
        ["2", "2 seconds"],
        ["3", "3 seconds"],
        ["5", "5 seconds"],
        ["0", "Never"],
      ]),
    ) +
    row("Live preview", "Show words in the Flow bar while you speak.", toggle("live_preview", "Live preview", `data-save="prefs"`)) +
    row(
      "Learn from my corrections",
      "After a paste the field is re-read for about 45 s. Words you fix show up in Dictionary as suggestions; nothing is added without your OK.",
      toggle("learn_from_edits", "Learn from my corrections", `data-save="prefs"`),
    ),
)}
${group(
  "",
  row(
    "Theme",
    "",
    select("theme", [
      ["light", "Light"],
      ["dark", "Dark"],
      ["system", "System"],
    ]),
  ),
)}
${group(
  "Setup",
  row(
    "Run setup again",
    "Choose where speech runs, the speech model and AI cleanup step by step.",
    `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="run_setup" data-run-setup>Run setup</button>`,
  ),
)}`,
};
