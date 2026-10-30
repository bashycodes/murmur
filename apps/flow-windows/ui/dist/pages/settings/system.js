// Settings → System: app behaviour, sounds and pasting. Fields:
// features/general.ts (prefs) and features/engines.ts (engine badge).
import { group, heading, row, toggle } from "../../lib/markup.js";
export const settingsSystemPage = {
    id: "settings-system",
    area: "settings",
    template: `
${heading("System")}
${group("App settings", row("Launch app at login", "", toggle("launch_at_login", "Launch app at login", `data-save="prefs"`)) +
        row("Show Flow bar at all times", "Otherwise it only appears while you dictate.", toggle("show_flow_bar_always", "Show Flow bar at all times", `data-save="prefs"`)) +
        row("Show which engine was used", "A small S (server) or L (this PC) on the Flow bar after each dictation; a red dot means it fell back.", toggle("dev_show_engine_badge", "Show which engine was used", `data-save="dev"`), `data-needs="dev"`))}
${group("Sound", row("Dictation sounds", "A soft click when dictation starts and stops.", toggle("enable_sounds", "Dictation sounds", `data-save="prefs"`)) +
        row("Mute other audio while dictating", "", toggle("mute_audio_while_dictating", "Mute other audio while dictating", `data-save="prefs"`)))}
${group("Pasting", row("Paste with", "Change this if an app ignores Ctrl+V.", `<select id="paste_method" class="fl-select" data-save="prefs">
      <option value="ctrl_v">Ctrl+V</option>
      <option value="shift_insert">Shift+Insert</option>
      <option value="type">Typing the characters</option>
    </select>`) +
        row("Restore clipboard after pasting", "Puts back whatever you had copied before dictating.", toggle("restore_clipboard", "Restore clipboard after pasting", `data-save="prefs"`)) +
        row("Longest dictation", "Recording stops by itself after this many minutes.", `<input id="max_dictation_minutes" class="fl-input is-narrow" data-save="prefs" type="number" min="1" max="60" />`))}`,
};
