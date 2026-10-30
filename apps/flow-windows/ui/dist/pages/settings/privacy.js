// Settings → Data and privacy: what leaves this PC (a summary of the
// engine and OpenRouter switches, changed on their own pages), history
// retention, and the opt-in debug log (README "Debug log").
import { $, askDialog, setStatus } from "../../lib/dom.js";
import { invoke, onEvent } from "../../lib/ipc.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import { prefsStore } from "../../lib/prefs-store.js";
import { views } from "../../lib/views.js";
import { reloadHistory } from "../home.js";
const link = (page, label) => `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" data-go="${page}">${label}</button>`;
const TEMPLATE = `
${heading("Data and privacy")}
${group("What leaves this PC", row("Nothing", "Everything runs on this PC. Nothing is sent over the network.", "", `id="leave_none"`) +
    row("Audio to your server", `<span id="leave_server_desc"></span>`, `<span id="leave_server_badge" class="fl-badge"></span>${link("settings-speech", "Change")}`, `id="leave_server"`) +
    row("Audio to OpenRouter", `<span id="leave_or_audio_desc"></span>`, `<span id="leave_or_audio_badge" class="fl-badge"></span>${link("settings-speech", "Change")}`, `id="leave_or_audio"`) +
    row("Text to OpenRouter", `<span id="leave_or_text_desc"></span>`, `<span id="leave_or_text_badge" class="fl-badge"></span>${link("settings-cleanup", "Change")}`, `id="leave_or_text"`))}
${group("History", row("Keep dictation history", "Shown on Home. Text only, in a local file on this PC; audio is never stored.", `<select id="retention" class="fl-select">
      <option value="keep">Forever</option>
      <option value="day">24 hours</option>
      <option value="never">Don't keep</option>
    </select>`) +
    row("Delete all history", "Erases every saved dictation from disk. This can't be undone. Insights keep their counts.", `<button type="button" id="hist_clear" class="fl-btn fl-btn-destructive fl-btn-sm">Delete all</button>`))}
${group("Troubleshooting", row("Debug log", `Saves each dictation's raw and final text, timings, engine and target app to a file on this PC (last 500), for bug reports. <b>Includes what you said.</b> Never contains your server token, API keys or clipboard. Turn it off when you are done.`, toggle("dev_debug_log", "Debug log")) +
    row("Log file", `<span id="dev_debug_count">No entries</span> · <code id="dev_debug_path" class="lw-mono"></code>`, `<button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="dev_debug_open">Open folder</button>
       <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm lw-danger" id="dev_debug_clear">Delete</button>`))}
<p class="lw-status-line"><span id="priv_status"></span></p>

<dialog id="hist_clear_dlg" class="fl-dialog">
  <h3>Delete all history?</h3>
  <p>Every saved dictation is erased from this PC. This can't be undone.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="hist_clear_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-destructive" id="hist_clear_ok">Delete all</button>
  </div>
</dialog>

<dialog id="dev_debug_off" class="fl-dialog">
  <h3>Debug log turned off</h3>
  <p id="dev_debug_off_body">The log still holds transcripts of earlier dictations. Delete it now?</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="dev_debug_keep">Keep the file</button>
    <button type="button" class="fl-btn fl-btn-destructive" id="dev_debug_delete">Delete the log</button>
  </div>
</dialog>`;
function badge(id, on) {
    const b = $(id);
    b.textContent = on ? "On" : "Off";
    b.className = on ? "fl-badge fl-badge-success" : "fl-badge fl-badge-light";
}
/** The summary rows: which switches can send audio or text off this PC. */
function showLeaves() {
    const dv = views.dev;
    const or = views.or;
    const remote = !!dv?.caps.remote;
    const cloud = !!or?.supported;
    $("leave_server").hidden = !remote;
    $("leave_or_audio").hidden = !cloud;
    $("leave_or_text").hidden = !cloud;
    $("leave_none").hidden = remote || cloud;
    if (remote && dv) {
        const f = dv.form;
        const on = f.allow_server;
        badge("leave_server_badge", on);
        $("leave_server_desc").textContent = on
            ? `${f.host || "No address set"}${f.tls ? ", encrypted" : ""}. Your own machine; audio goes nowhere else.`
            : "Off: speech is turned into text on this PC.";
    }
    if (cloud && or) {
        badge("leave_or_audio_badge", or.stt.allow);
        $("leave_or_audio_desc").textContent = or.stt.allow
            ? `For speech to text with ${or.stt.model}.`
            : "Off. Turning it on asks first.";
        badge("leave_or_text_badge", or.cleanup.allow);
        $("leave_or_text_desc").textContent = or.cleanup.allow
            ? `For AI cleanup with ${or.cleanup.model}.${or.privacy.zdr ? " Zero-retention providers only." : ""}`
            : "Off. Turning it on asks first.";
    }
}
async function saveRetention() {
    try {
        await prefsStore.save({ retention: $("retention").value });
        setStatus("priv_status", "Saved", true);
        await reloadHistory();
    }
    catch (e) {
        setStatus("priv_status", String(e), false);
    }
}
async function clearHistory() {
    if (!(await askDialog("hist_clear_dlg", "hist_clear_ok", "hist_clear_cancel")))
        return;
    try {
        await invoke("history_clear");
        setStatus("priv_status", "History deleted", true);
    }
    catch (e) {
        setStatus("priv_status", String(e), false);
    }
    await reloadHistory();
}
// ------------------------------------------------------------- debug log
function showDebugLog(v) {
    $("dev_debug_log").checked = v.enabled;
    $("dev_debug_path").textContent = v.path;
    const n = v.count;
    let line = n === null ? "Not readable" : n === 0 ? "Empty" : n === 1 ? "1 entry" : `${n} entries`;
    if (v.stats.write_errors > 0)
        line += ` · ${v.stats.write_errors} write errors (ignored)`;
    if (v.stats.dropped > 0)
        line += ` · ${v.stats.dropped} dropped`;
    $("dev_debug_count").textContent = line;
}
async function debugCall(cmd, args, okText) {
    try {
        const v = await invoke(cmd, args);
        setStatus("priv_status", okText, true);
        return v;
    }
    catch (e) {
        setStatus("priv_status", String(e), false);
        return null;
    }
}
async function loadDebugLog() {
    try {
        showDebugLog(await invoke("debug_log_status"));
    }
    catch (e) {
        setStatus("priv_status", String(e), false);
    }
}
/** Turning the log off offers to delete what it already holds. */
function askClearDebugLog(count) {
    $("dev_debug_off_body").textContent =
        `The log still holds ${count === 1 ? "1 entry" : `${count} entries`} with transcripts of earlier dictations. Delete it now?`;
    return askDialog("dev_debug_off", "dev_debug_delete", "dev_debug_keep");
}
async function debugLogToggled() {
    const on = $("dev_debug_log").checked;
    const v = await debugCall("debug_log_set", { enabled: on }, on ? "Debug log on: transcripts are being written" : "Debug log off");
    if (!v) {
        $("dev_debug_log").checked = !on;
        return;
    }
    showDebugLog(v);
    if (!on && (v.count ?? 0) > 0 && (await askClearDebugLog(v.count ?? 0))) {
        const c = await debugCall("debug_log_clear", {}, "Debug log off and deleted");
        if (c)
            showDebugLog(c);
    }
}
export const settingsPrivacyPage = {
    id: "settings-privacy",
    area: "settings",
    template: TEMPLATE,
    init() {
        $("retention").addEventListener("change", () => void saveRetention());
        $("hist_clear").addEventListener("click", () => void clearHistory());
        $("dev_debug_log").addEventListener("change", () => void debugLogToggled());
        $("dev_debug_open").addEventListener("click", () => void debugCall("debug_log_open_folder", {}, "Opened the log folder"));
        $("dev_debug_clear").addEventListener("click", () => void debugCall("debug_log_clear", {}, "Debug log deleted").then((v) => v && showDebugLog(v)));
        prefsStore.subscribe((p) => ($("retention").value = p.retention));
        views.onDev(showLeaves);
        views.onOr(showLeaves);
        void onEvent("history:changed", () => {
            if ($("dev_debug_log").checked)
                void loadDebugLog();
        });
        void loadDebugLog();
    },
    show() {
        showLeaves();
        void loadDebugLog();
    },
};
