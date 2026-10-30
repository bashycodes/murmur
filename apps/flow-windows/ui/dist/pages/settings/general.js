// Settings → General: shortcuts and voice. Fields: features/general.ts.
import { group, heading, row, toggle } from "../../lib/markup.js";
const key = (id, ph) => `<input id="${id}" class="fl-input" data-save="prefs" spellcheck="false" placeholder="${ph}" />`;
export const settingsGeneralPage = {
    id: "settings-general",
    area: "settings",
    template: `
${heading("General")}
${group("Shortcuts", row("Push to talk", "Hold, speak, let go to paste.", key("ptt", "Ctrl+Win")) +
        row("Hands-free", "Press to start, press again to stop.", key("hands_free", "Ctrl+Win+Space")) +
        row("Command Mode", "Hold and say an instruction: rewrites the selected text (or writes new text at the cursor) with the AI cleanup model.", key("command", "Ctrl+Win+Alt")) +
        row("Cancel", "", key("cancel", "Esc")) +
        row("Paste last dictation", "", key("paste_last", "Shift+Alt+Z")) +
        row("Copy last dictation", "", key("copy_last", "Shift+Alt+X")) +
        row("Open Scratchpad", "", key("scratchpad", "none (e.g. Ctrl+Alt+N)")) +
        row("New note and dictate", "", key("new_note", "none (e.g. Ctrl+Alt+M)")))}
<p class="fl-help lw-group-help">Keys: Ctrl, Win, Alt, Shift, Space, Esc, A–Z, 0–9, F1–F24. Ctrl/Alt/Shift match either side; Left Alt, Right Alt (also Left/Right Ctrl, Shift) match one side only, e.g. push to talk = Right Alt. Max 3 keys, at least one modifier.</p>
${group("Voice", row("Microphone", "", `<select id="microphone" class="fl-select is-wide" data-save="prefs"></select>`) +
        row("Languages", "Comma-separated, e.g. <code>en, de</code>. Empty = detect automatically.", `<input id="languages" class="fl-input" data-save="prefs" spellcheck="false" placeholder="auto" />`) +
        row("Hands-free stops after", "Seconds of silence before hands-free dictation ends (0 = never).", `<input id="hands_free_silence_s" class="fl-input is-narrow" data-save="prefs" type="number" min="0" max="10" step="0.1" />`) +
        row("Live preview", "Show words in the Flow bar while you speak.", toggle("live_preview", "Live preview", `data-save="prefs"`)) +
        row("Learn from my corrections", "After a paste the field is re-read for about 45 s. Words you fix show up in Dictionary as suggestions; nothing is added without your OK.", toggle("learn_from_edits", "Learn from my corrections", `data-save="prefs"`)))}
${group("Setup", row("Run setup again", "Choose where speech runs, the speech model and AI cleanup step by step.", `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="run_setup" data-run-setup>Run setup</button>`))}`,
};
