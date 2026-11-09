// Settings → System: app behaviour, sounds and pasting
// (docs/design/app-design/SettingsSystem.dc.html). Fields:
// features/general.ts (prefs) and features/engines.ts (engine badge).

import { group, heading, row, toggle } from "../../lib/markup.js";
import type { Page } from "../../lib/router.js";

export const settingsSystemPage: Page = {
  id: "settings-system",
  area: "settings",
  template: `
${heading("System")}
${group(
  "App settings",
  row("Start when you sign in to Windows", "", toggle("launch_at_login", "Start when you sign in to Windows", `data-save="prefs"`)) +
    row(
      "Keep the Murmur bar on screen",
      "Otherwise it only appears while you dictate.",
      toggle("show_murmur_bar_always", "Keep the Murmur bar on screen", `data-save="prefs"`),
    ) +
    row(
      "Show which engine was used",
      "A small <b>L</b>, <b>S</b> or <b>C</b> on the Murmur bar after each dictation: this PC, your server or the cloud.",
      toggle("dev_show_engine_badge", "Show which engine was used", `data-save="dev"`),
      `data-needs="dev"`,
    ),
)}
${group(
  "Sound",
  row("Dictation sounds", "A soft click when dictation starts and stops.", toggle("enable_sounds", "Dictation sounds", `data-save="prefs"`)) +
    row(
      "Mute other audio while dictating",
      "",
      toggle("mute_audio_while_dictating", "Mute other audio while dictating", `data-save="prefs"`),
    ),
)}
${group(
  "Pasting",
  row(
    "Paste with",
    "Change this if an app ignores Ctrl+V.",
    `<select id="paste_method" class="fl-select lw-w160" data-save="prefs" aria-label="Paste with">
      <option value="ctrl_v">Ctrl+V</option>
      <option value="shift_insert">Shift+Insert</option>
      <option value="type">Typing</option>
    </select>`,
  ) +
    row(
      "Restore clipboard after pasting",
      "Puts back whatever you had copied before dictating.",
      toggle("restore_clipboard", "Restore clipboard after pasting", `data-save="prefs"`),
    ) +
    row(
      "Longest dictation",
      "Recording stops by itself after this.",
      `<select id="max_dictation_minutes" class="fl-select lw-w160" data-save="prefs" aria-label="Longest dictation">
        <option value="2">2 minutes</option>
        <option value="6">6 minutes</option>
        <option value="15">15 minutes</option>
        <option value="20">20 minutes</option>
        <option value="30">30 minutes</option>
      </select>`,
    ),
)}`,
};
