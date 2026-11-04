// Meetings page (Notetaker, D45): record a meeting (mic = "Me", what the
// PC plays = "Others"), then read, copy or delete its transcript. The
// recording is transcribed after it stops, on its own engine, and never
// holds up a dictation. Audio of the last 3 meetings is kept (for Retry);
// transcripts stay until deleted. Backend: `meeting_*` in src/app.rs,
// events `meetings:changed` (src/meetings/mod.rs `View`). Meeting
// detection (D48): `prefs.meeting_detection` = off / ask / auto. AI
// summary + action items (D53): made after the transcript on the cleanup
// order, `meeting_summarise` = Regenerate.
import { $, askDialog, el } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { router } from "../lib/router.js";
const TEMPLATE = `
<div class="lw-page lw-mt">
  <div class="lw-page-head lw-mt-head">
    <h1 class="fl-page-title">Meetings</h1>
    <span class="fl-badge fl-badge-dark">Beta</span>
    <span class="lw-mt-grow"></span>
    <span class="fl-kbd-group" id="mt_keys" hidden title="Starts and stops a recording from any app"></span>
    <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="mt_shortcut" data-go="settings-general">Set a shortcut</button>
    <button type="button" class="fl-btn fl-btn-primary fl-btn-sm" id="mt_toggle">Start recording</button>
  </div>

  <div class="lw-mt-live" id="mt_live" role="status" hidden>
    <span class="lw-mt-dot" id="mt_dot"></span>
    <span id="mt_live_text"></span>
  </div>
  <p class="lw-mt-warn" id="mt_warn" role="alert" hidden></p>

  <div class="lw-mt-detect">
    <label for="mt_detect" class="lw-mt-detect-label">When a Zoom, Webex or Jabber call starts
      <span class="fl-help">Checks every few seconds which of these apps uses the microphone or shows a meeting window. A recording started this way stops when the call ends.</span>
    </label>
    <select id="mt_detect" class="fl-select lw-w220" aria-label="When a Zoom, Webex or Jabber call starts">
      <option value="off">Do nothing</option>
      <option value="ask">Ask me</option>
      <option value="auto">Start recording automatically</option>
    </select>
  </div>

  <section id="mt_list_view">
    <div class="fl-list" id="mt_list"></div>
    <div class="lw-nt-hero lw-mt-empty" id="mt_empty" hidden>
      ${iconSvg(ICON.notetaker, 24)}
      <h2 class="fl-serif lw-nt-title">Meeting notes, <em>without the bot</em></h2>
      <p class="lw-nt-body">Start a recording when your call begins. Your microphone becomes “Me”, everything your PC plays becomes “Others”. When you stop, the meeting is transcribed with the same engines you use for dictation.</p>
    </div>
    <p class="fl-help">Saved on this PC next to your history. The audio of the last 3 meetings is kept so you can transcribe them again; older audio is deleted, transcripts stay.</p>
  </section>

  <section id="mt_detail" hidden>
    <div class="lw-mt-detail-head">
      <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="mt_back">← All meetings</button>
      <span class="lw-mt-grow"></span>
      <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="mt_retry" hidden>Transcribe again</button>
      <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="mt_copy">Copy transcript</button>
      <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="mt_delete">Delete</button>
    </div>
    <h2 class="lw-mt-title" id="mt_title"></h2>
    <p class="lw-mt-sub" id="mt_sub"></p>
    <p class="lw-mt-note" id="mt_note" hidden></p>
    <div class="lw-mt-summary" id="mt_summary" hidden>
      <div class="lw-mt-summary-head">
        <h3 class="lw-mt-summary-title">Summary</h3>
        <span class="lw-mt-summary-by" id="mt_summary_by"></span>
        <span class="lw-mt-grow"></span>
        <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="mt_regen">Regenerate</button>
      </div>
      <p class="lw-mt-summary-state" id="mt_summary_state" hidden></p>
      <p class="lw-mt-summary-text" id="mt_summary_text" dir="auto"></p>
      <h4 class="lw-mt-summary-sub" id="mt_items_title" hidden>Action items</h4>
      <ul class="lw-mt-items" id="mt_items" dir="auto"></ul>
    </div>
    <div class="lw-mt-turns" id="mt_turns"></div>
  </section>

  <p class="fl-help"><span id="mt_status" role="status"></span></p>
</div>

<dialog id="mt_del_dlg" class="fl-dialog">
  <h3>Delete this meeting?</h3>
  <p>Its transcript and audio are erased from this PC. This can't be undone.</p>
  <div class="lw-dialog-actions">
    <button type="button" class="fl-btn fl-btn-secondary" id="mt_del_cancel">Cancel</button>
    <button type="button" class="fl-btn fl-btn-destructive" id="mt_del_ok">Delete</button>
  </div>
</dialog>`;
let view = { recording: null, transcribing: null, queued: 0, warning: "" };
let openId = null;
let liveTimer = 0;
let statusTimer = 0;
/** 65_000 → "1:05", 3_725_000 → "1:02:05". */
export function clock(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const ss = String(s % 60).padStart(2, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
/** 65_000 → "1 min", 30_000 → "30 s", 3_725_000 → "1 h 2 min". */
export function duration(ms) {
    const s = Math.round(ms / 1000);
    if (s < 60)
        return `${s} s`;
    const m = Math.round(s / 60);
    return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}
function startedText(ms) {
    return new Date(ms).toLocaleString([], {
        weekday: "short",
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
    });
}
function appName(app) {
    return (app ?? "").replace(/\.exe$/i, "");
}
function say(text, ok = true) {
    const s = $("mt_status");
    s.textContent = text;
    s.classList.toggle("ok", ok && text !== "");
    s.classList.toggle("err", !ok);
    clearTimeout(statusTimer);
    if (ok)
        statusTimer = setTimeout(() => (s.textContent = ""), 3000);
}
function statusText(m) {
    switch (m.status) {
        case "recording":
            return "Recording…";
        case "transcribing":
            return view.transcribing?.id === m.id
                ? `Transcribing… ${Math.round(view.transcribing.percent)}%`
                : "Waiting to be transcribed";
        case "failed":
            return `Failed: ${m.error ?? "unknown error"}`;
        default:
            return "";
    }
}
function showLive() {
    clearInterval(liveTimer);
    const live = $("mt_live");
    const text = $("mt_live_text");
    const toggle = $("mt_toggle");
    const rec = view.recording;
    toggle.textContent = rec ? "Stop recording" : "Start recording";
    toggle.className = rec ? "fl-btn fl-btn-destructive fl-btn-sm" : "fl-btn fl-btn-primary fl-btn-sm";
    $("mt_dot").classList.toggle("is-rec", !!rec);
    if (rec) {
        const tick = () => {
            text.textContent = `Recording · ${clock(Date.now() - rec.started_ms)} · your mic and the PC's sound`;
        };
        tick();
        liveTimer = setInterval(tick, 1000);
    }
    else if (view.transcribing) {
        const more = view.queued ? ` (${view.queued} more waiting)` : "";
        text.textContent = `Transcribing a meeting… ${Math.round(view.transcribing.percent)}%${more}. Dictation keeps priority.`;
    }
    live.hidden = !rec && !view.transcribing;
    const warn = $("mt_warn");
    warn.textContent = view.warning;
    warn.hidden = !view.warning;
}
function row(m) {
    const r = el("div", "fl-list-row lw-mt-row");
    r.dataset.id = m.id;
    const open = el("button", "lw-mt-open");
    open.type = "button";
    open.title = "Open the transcript";
    const text = el("span", "lw-mt-text");
    const facts = [duration(m.duration_ms), appName(m.app)].filter(Boolean).join(" · ");
    text.append(el("span", "lw-mt-when", startedText(m.started_ms)), el("span", "lw-mt-facts", facts));
    const st = statusText(m);
    const line = st || m.preview || (m.turns ? "" : "Nothing was said");
    if (line)
        text.append(el("span", `lw-mt-preview${m.status === "failed" ? " is-err" : ""}`, line));
    const ico = iconEl(ICON.notetaker, 20);
    ico.classList.add("lw-mt-ico");
    open.append(ico, text);
    open.addEventListener("click", () => void openMeeting(m.id));
    const actions = el("div", "lw-mt-actions");
    if (m.status === "done") {
        actions.append(iconButton("Copy transcript", ICON.copy, () => void copy(m.id)));
    }
    if (m.status === "failed" && m.has_audio) {
        actions.append(iconButton("Transcribe again", ICON.retry, () => void retry(m.id)));
    }
    if (m.status !== "recording") {
        actions.append(iconButton("Delete", ICON.trash, () => void remove(m.id)));
    }
    r.append(open, actions);
    return r;
}
function iconButton(label, icon, onClick) {
    const b = el("button", "fl-iconbtn is-toolbar");
    b.type = "button";
    b.title = label;
    b.setAttribute("aria-label", label);
    b.append(iconEl(icon, 16));
    b.addEventListener("click", onClick);
    return b;
}
async function refreshList() {
    let list;
    try {
        list = await invoke("meeting_list");
    }
    catch (e) {
        say(`Could not list the meetings: ${e}`, false);
        return;
    }
    $("mt_list").replaceChildren(...list.map(row));
    $("mt_empty").hidden = list.length > 0;
}
const SUMMARY_BY = {
    local: "this PC's AI model",
    server: "your server",
    cloud: "OpenRouter",
};
/** What the summary box says about its state ("" when it just shows it). */
export function summaryState(s, running) {
    if (!s)
        return "";
    if (running)
        return "Writing the summary… Dictation keeps priority.";
    if (s.status === "pending")
        return "Waiting to be summarised";
    if (s.status === "failed")
        return `No summary: ${s.error ?? "unknown error"}`;
    return "";
}
function showSummary(m) {
    const box = $("mt_summary");
    const s = m.summary;
    box.hidden = m.status !== "done" || !s;
    if (!s || box.hidden)
        return;
    const running = view.summarising === m.id;
    const state = summaryState(s, running);
    const st = $("mt_summary_state");
    st.textContent = state;
    st.hidden = !state;
    st.classList.toggle("is-err", s.status === "failed" && !running);
    $("mt_summary_by").textContent =
        s.status === "done" && s.engine ? `by ${SUMMARY_BY[s.engine] ?? s.engine}` : "";
    $("mt_summary_text").textContent = s.text;
    $("mt_items").replaceChildren(...s.action_items.map((t) => el("li", "", t)));
    $("mt_items_title").hidden = s.action_items.length === 0;
    $("mt_regen").disabled = running || s.status === "pending";
}
const ROUTE_NAMES = {
    local: "This PC",
    remote: "Your server",
    cloud: "OpenRouter",
};
/** "Transcribed on: This PC"; with several routes, the most used first
 * with the share of chunks ("This PC 4, Your server 2"). */
export function routesText(routes) {
    const used = Object.entries(routes ?? {})
        .filter(([r, n]) => n > 0 && r in ROUTE_NAMES)
        .sort((a, b) => b[1] - a[1]);
    if (!used.length)
        return "";
    const names = used.length === 1
        ? ROUTE_NAMES[used[0][0]]
        : used.map(([r, n]) => `${ROUTE_NAMES[r]} ${n}`).join(", ");
    return `Transcribed on: ${names}`;
}
function showDetail(m) {
    $("mt_title").textContent = startedText(m.started_ms);
    const facts = [duration(m.duration_ms), appName(m.app), statusText(m), routesText(m.routes)].filter(Boolean);
    $("mt_sub").textContent = facts.join(" · ");
    const note = $("mt_note");
    note.textContent = m.system_silent
        ? "No sound from the PC was recorded, so the transcript has only your side."
        : "";
    note.hidden = !m.system_silent;
    $("mt_copy").disabled = m.status !== "done";
    $("mt_delete").disabled = m.status === "recording";
    $("mt_retry").hidden = m.status !== "failed";
    const turns = m.turns.map((t) => {
        const r = el("div", `lw-mt-turn is-${t.speaker}`);
        r.append(el("span", "lw-mt-stamp", clock(t.start_ms)), el("span", "lw-mt-who", t.speaker === "me" ? "Me" : "Others"), el("p", `lw-mt-said${t.error ? " is-err" : ""}`, t.error ? `Not transcribed: ${t.error}` : t.text));
        return r;
    });
    if (!turns.length && m.status === "done") {
        turns.push(el("p", "lw-mt-note", "Nothing was said in this meeting."));
    }
    $("mt_turns").replaceChildren(...turns);
    showSummary(m);
}
async function openMeeting(id) {
    try {
        const m = await invoke("meeting_get", { id });
        openId = id;
        showDetail(m);
        $("mt_list_view").hidden = true;
        $("mt_detail").hidden = false;
    }
    catch (e) {
        say(String(e), false);
        await refreshList();
    }
}
function closeMeeting() {
    openId = null;
    $("mt_detail").hidden = true;
    $("mt_list_view").hidden = false;
    void refreshList();
}
async function copy(id) {
    try {
        await invoke("meeting_copy", { id });
        say("Copied");
    }
    catch (e) {
        say(String(e), false);
    }
}
async function retry(id) {
    try {
        await invoke("meeting_retry", { id });
        say("Transcribing again");
    }
    catch (e) {
        say(String(e), false);
    }
    await refresh();
}
async function regenerate(id) {
    $("mt_regen").disabled = true;
    try {
        await invoke("meeting_summarise", { id });
    }
    catch (e) {
        say(String(e), false);
    }
    await refresh();
}
async function remove(id) {
    if (!(await askDialog("mt_del_dlg", "mt_del_ok", "mt_del_cancel")))
        return;
    try {
        await invoke("meeting_delete", { id });
        say("Deleted");
        if (openId === id)
            closeMeeting();
    }
    catch (e) {
        say(String(e), false);
    }
    await refreshList();
}
async function toggle() {
    const b = $("mt_toggle");
    b.disabled = true;
    try {
        await invoke("meeting_toggle");
    }
    catch (e) {
        say(String(e), false);
    }
    b.disabled = false;
    await refresh();
}
async function refresh() {
    try {
        view = await invoke("meeting_state");
    }
    catch {
        // not ready yet: keep the last view
    }
    showLive();
    if (openId)
        await openMeeting(openId);
    else
        await refreshList();
}
function showDetect(p) {
    const v = p.meeting_detection;
    $("mt_detect").value = v === "ask" || v === "auto" ? v : "off";
}
async function saveDetect() {
    const value = $("mt_detect").value;
    try {
        await prefsStore.save({ meeting_detection: value });
        say(value === "off" ? "Meeting detection is off" : "Saved");
    }
    catch (e) {
        say(String(e), false);
        const p = prefsStore.get();
        if (p)
            showDetect(p);
    }
}
function showShortcut(p) {
    const combo = p.shortcuts.meeting?.trim() ?? "";
    const keys = combo.split("+").map((k) => k.trim()).filter(Boolean);
    const box = $("mt_keys");
    box.replaceChildren(...keys.map((k) => el("kbd", "fl-kbd", k)));
    box.hidden = keys.length === 0;
    $("mt_shortcut").textContent = keys.length ? "Change" : "Set a shortcut";
}
export const meetingsPage = {
    id: "meetings",
    area: "main",
    template: TEMPLATE,
    init() {
        $("mt_toggle").addEventListener("click", () => void toggle());
        $("mt_back").addEventListener("click", closeMeeting);
        $("mt_copy").addEventListener("click", () => openId && void copy(openId));
        $("mt_delete").addEventListener("click", () => openId && void remove(openId));
        $("mt_retry").addEventListener("click", () => openId && void retry(openId));
        $("mt_regen").addEventListener("click", () => openId && void regenerate(openId));
        prefsStore.subscribe(showShortcut);
        prefsStore.subscribe(showDetect);
        $("mt_detect").addEventListener("change", () => void saveDetect());
        void onEvent("meetings:changed", (e) => {
            view = e.payload;
            showLive();
            if (router.active() !== "meetings")
                return;
            if (openId)
                void openMeeting(openId);
            else
                void refreshList();
        });
    },
    show() {
        void refresh();
    },
};
