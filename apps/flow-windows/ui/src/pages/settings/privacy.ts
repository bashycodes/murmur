// Settings → Data and privacy (docs/design/app-design/SettingsPrivacy.dc.html):
// what leaves this PC (a summary of the
// engine and OpenRouter switches, changed on their own pages), history
// retention, and the opt-in debug log (README "Debug log").

import { $, askDialog, setStatus } from "../../lib/dom.js";
import { invoke, onEvent } from "../../lib/ipc.js";
import { ICON, iconSvg } from "../../lib/icons.js";
import { group, heading, row, toggle } from "../../lib/markup.js";
import { prefsStore } from "../../lib/prefs-store.js";
import type { Page } from "../../lib/router.js";
import { views } from "../../lib/views.js";
import { reloadHistory } from "../home.js";

interface DebugLogView {
  enabled: boolean;
  count: number | null;
  path: string;
  stats: { written: number; dropped: number; write_errors: number };
}

/** A "What leaves this PC" row: icon tile, title + desc, On/Off, Edit →. */
const leave = (id: string, icon: string, title: string, page: string): string =>
  `<div class="fl-row lw-leave" id="${id}">
    <span class="lw-tile">${iconSvg(icon)}</span>
    <div class="fl-row-text"><div class="fl-row-title">${title}</div><div class="fl-row-desc" id="${id}_desc"></div></div>
    <span id="${id}_badge" class="fl-badge"></span>
    <button type="button" class="lw-link lw-leave-edit" data-go="${page}">Edit →</button>
  </div>`;

const TEMPLATE = `
${heading("Data and privacy", "No account, no telemetry, nothing sent to us. This is everything that can leave this PC, in one place.")}
${group(
  "What leaves this PC",
  `<div class="fl-row lw-leave" id="leave_none">
    <span class="lw-tile">${iconSvg(ICON.chip)}</span>
    <div class="fl-row-text"><div class="fl-row-title">Nothing</div><div class="fl-row-desc">Everything runs on this PC. Nothing is sent over the network.</div></div>
  </div>` +
    leave("leave_server", ICON.server, "Audio to your server", "settings-server") +
    leave("leave_or_audio", ICON.cloud, "Audio to OpenRouter", "settings-speech") +
    leave("leave_or_text", ICON.cloud, "Text to OpenRouter", "settings-cleanup"),
)}
${group(
  "History",
  row(
    "Keep dictation history",
    "Shown on Home. Stored only on this PC.",
    `<select id="retention" class="fl-select lw-w160" aria-label="Keep dictation history">
      <option value="keep">Forever</option>
      <option value="days_30">30 days</option>
      <option value="days_7">7 days</option>
      <option value="never">Don't keep</option>
    </select>`,
  ) +
    row(
      "Keep recordings for retry",
      "The audio of your newest dictations, so Retry on Home can transcribe them again with your current settings. Never kept for password fields; deleted with its dictation.",
      `<select id="retry_audio_keep" class="fl-select lw-w160" aria-label="Keep recordings for retry">
        <option value="0">Don't keep</option>
        <option value="5">Newest 5</option>
        <option value="10">Newest 10</option>
        <option value="20">Newest 20</option>
        <option value="50">Newest 50</option>
        <option value="100">Newest 100</option>
      </select>`,
    ) +
    row(
      "Delete all history",
      "Erases every saved dictation from disk. This can't be undone.",
      `<button type="button" id="hist_clear" class="fl-btn fl-btn-destructive">Delete</button>`,
    ),
)}
${group(
  "Troubleshooting",
  row(
    "Debug log",
    `Saves each dictation's text and timings to a file here, for bug reports. <b>Includes what you said.</b> Never uploaded, and never holds your server token or API keys.`,
    toggle("dev_debug_log", "Debug log"),
  ) +
    row(
      "Log file",
      `<span id="dev_debug_count">Empty</span>`,
      `<button type="button" class="fl-btn fl-btn-ghost" id="dev_debug_clear">Clear</button>
       <button type="button" class="fl-btn fl-btn-secondary" id="dev_debug_open">Open folder</button>`,
      `id="dev_debug_file"`,
    ),
)}
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

function badge(id: string, on: boolean): void {
  const b = $<HTMLElement>(id);
  b.textContent = on ? "On" : "Off";
  b.className = on ? "fl-badge fl-badge-success" : "fl-badge fl-badge-light";
}

/** The summary rows: which switches can send audio or text off this PC. */
function showLeaves(): void {
  const dv = views.dev;
  const or = views.or;
  const remote = !!dv?.caps.remote;
  const cloud = !!or?.supported;
  $<HTMLElement>("leave_server").hidden = !remote;
  $<HTMLElement>("leave_or_audio").hidden = !cloud;
  $<HTMLElement>("leave_or_text").hidden = !cloud;
  $<HTMLElement>("leave_none").hidden = remote || cloud;
  if (remote && dv) {
    const f = dv.form;
    const on = f.allow_server;
    badge("leave_server_badge", on);
    $<HTMLElement>("leave_server_desc").textContent = on
      ? `${f.host || "No address set"}${f.tls ? ", encrypted" : ""}. Your own machine; audio goes nowhere else.`
      : "Off: speech is turned into text on this PC.";
  }
  if (cloud && or) {
    badge("leave_or_audio_badge", or.stt.allow);
    $<HTMLElement>("leave_or_audio_desc").textContent = or.stt.allow
      ? `For speech to text with ${or.stt.model}.`
      : "Off. Turning it on asks first.";
    badge("leave_or_text_badge", or.cleanup.allow);
    $<HTMLElement>("leave_or_text_desc").textContent = or.cleanup.allow
      ? `For AI cleanup with ${or.cleanup.model}.${or.privacy.zdr ? " Zero-retention providers only." : ""}`
      : "Off. Turning it on asks first.";
  }
}

/** Older files may hold `day` (24 hours, before the redesign): show it as is. */
function showRetention(r: string): void {
  const s = $<HTMLSelectElement>("retention");
  if (r === "day" && ![...s.options].some((o) => o.value === "day")) s.add(new Option("24 hours", "day"));
  s.value = r;
}

async function saveRetention(): Promise<void> {
  try {
    await prefsStore.save({ retention: $<HTMLSelectElement>("retention").value });
    setStatus("priv_status", "Saved", true);
    await reloadHistory();
  } catch (e) {
    setStatus("priv_status", String(e), false);
  }
}

/** A value saved outside the list (another count) is shown as is. */
function showRetryKeep(n: number | undefined): void {
  const s = $<HTMLSelectElement>("retry_audio_keep");
  const v = String(n ?? 20);
  if (![...s.options].some((o) => o.value === v)) s.add(new Option(`Newest ${v}`, v));
  s.value = v;
}

async function saveRetryKeep(): Promise<void> {
  try {
    await prefsStore.save({ retry_audio_keep: Number($<HTMLSelectElement>("retry_audio_keep").value) });
    setStatus("priv_status", "Saved", true);
    await reloadHistory();
  } catch (e) {
    setStatus("priv_status", String(e), false);
  }
}

async function clearHistory(): Promise<void> {
  if (!(await askDialog("hist_clear_dlg", "hist_clear_ok", "hist_clear_cancel"))) return;
  try {
    await invoke("history_clear");
    setStatus("priv_status", "History deleted", true);
  } catch (e) {
    setStatus("priv_status", String(e), false);
  }
  await reloadHistory();
}

// ------------------------------------------------------------- debug log

function showDebugLog(v: DebugLogView): void {
  $<HTMLInputElement>("dev_debug_log").checked = v.enabled;
  $<HTMLElement>("dev_debug_file").title = v.path;
  const n = v.count;
  let line = n === null ? "Not readable" : n === 0 ? "Empty" : n === 1 ? "1 entry" : `${n} entries`;
  if (v.stats.write_errors > 0) line += ` · ${v.stats.write_errors} write errors (ignored)`;
  if (v.stats.dropped > 0) line += ` · ${v.stats.dropped} dropped`;
  $<HTMLElement>("dev_debug_count").textContent = line;
}

async function debugCall(cmd: string, args: Record<string, unknown>, okText: string): Promise<DebugLogView | null> {
  try {
    const v = await invoke<DebugLogView>(cmd, args);
    setStatus("priv_status", okText, true);
    return v;
  } catch (e) {
    setStatus("priv_status", String(e), false);
    return null;
  }
}

async function loadDebugLog(): Promise<void> {
  try {
    showDebugLog(await invoke<DebugLogView>("debug_log_status"));
  } catch (e) {
    setStatus("priv_status", String(e), false);
  }
}

/** Turning the log off offers to delete what it already holds. */
function askClearDebugLog(count: number): Promise<boolean> {
  $<HTMLElement>("dev_debug_off_body").textContent =
    `The log still holds ${count === 1 ? "1 entry" : `${count} entries`} with transcripts of earlier dictations. Delete it now?`;
  return askDialog("dev_debug_off", "dev_debug_delete", "dev_debug_keep");
}

async function debugLogToggled(): Promise<void> {
  const on = $<HTMLInputElement>("dev_debug_log").checked;
  const v = await debugCall(
    "debug_log_set",
    { enabled: on },
    on ? "Debug log on: transcripts are being written" : "Debug log off",
  );
  if (!v) {
    $<HTMLInputElement>("dev_debug_log").checked = !on;
    return;
  }
  showDebugLog(v);
  if (!on && (v.count ?? 0) > 0 && (await askClearDebugLog(v.count ?? 0))) {
    const c = await debugCall("debug_log_clear", {}, "Debug log off and deleted");
    if (c) showDebugLog(c);
  }
}

export const settingsPrivacyPage: Page = {
  id: "settings-privacy",
  area: "settings",
  template: TEMPLATE,
  init(): void {
    $<HTMLSelectElement>("retention").addEventListener("change", () => void saveRetention());
    $<HTMLButtonElement>("hist_clear").addEventListener("click", () => void clearHistory());
    $<HTMLSelectElement>("retry_audio_keep").addEventListener("change", () => void saveRetryKeep());
    $<HTMLInputElement>("dev_debug_log").addEventListener("change", () => void debugLogToggled());
    $<HTMLButtonElement>("dev_debug_open").addEventListener("click", () =>
      void debugCall("debug_log_open_folder", {}, "Opened the log folder"),
    );
    $<HTMLButtonElement>("dev_debug_clear").addEventListener("click", () =>
      void debugCall("debug_log_clear", {}, "Debug log deleted").then((v) => v && showDebugLog(v)),
    );
    prefsStore.subscribe((p) => {
      showRetention(p.retention);
      showRetryKeep(p.retry_audio_keep);
    });
    views.onDev(showLeaves);
    views.onOr(showLeaves);
    void onEvent<null>("history:changed", () => {
      if ($<HTMLInputElement>("dev_debug_log").checked) void loadDebugLog();
    });
    void loadDebugLog();
  },
  show(): void {
    showLeaves();
    void loadDebugLog();
  },
};
