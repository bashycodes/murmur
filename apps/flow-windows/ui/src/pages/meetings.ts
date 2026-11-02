// Meetings page (Notetaker, D45): record a meeting (mic = "Me", what the
// PC plays = "Others"), then read, copy or delete its transcript. The
// recording is transcribed after it stops, on its own engine, and never
// holds up a dictation. Audio of the last 3 meetings is kept (for Retry);
// transcripts stay until deleted. Backend: `meeting_*` in src/app.rs,
// events `meetings:changed` (src/meetings/mod.rs `View`). Meeting
// detection (D48): `prefs.meeting_detection` = off / ask / auto.

import { $, askDialog, el } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { router, type Page } from "../lib/router.js";

type Status = "recording" | "transcribing" | "done" | "failed";

/** `meetings::store::MeetingMeta`. */
export interface MeetingMeta {
  id: string;
  started_ms: number;
  duration_ms: number;
  app: string | null;
  status: Status;
  error: string | null;
  turns: number;
  preview: string;
  has_audio: boolean;
}

/** `meetings::transcript::Turn`. */
export interface Turn {
  speaker: "me" | "others";
  start_ms: number;
  end_ms: number;
  text: string;
  error?: string;
}

/** `meetings::store::Meeting`. */
export interface Meeting {
  id: string;
  started_ms: number;
  duration_ms: number;
  app?: string;
  status: Status;
  error?: string;
  system_silent?: boolean;
  turns: Turn[];
}

/** `meetings::View`. */
export interface MeetingView {
  recording: { id: string; started_ms: number } | null;
  transcribing: { id: string; percent: number } | null;
  queued: number;
  warning: string;
}

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

let view: MeetingView = { recording: null, transcribing: null, queued: 0, warning: "" };
let openId: string | null = null;
let liveTimer = 0;
let statusTimer = 0;

/** 65_000 → "1:05", 3_725_000 → "1:02:05". */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** 65_000 → "1 min", 30_000 → "30 s", 3_725_000 → "1 h 2 min". */
export function duration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${m % 60} min`;
}

function startedText(ms: number): string {
  return new Date(ms).toLocaleString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function appName(app: string | null | undefined): string {
  return (app ?? "").replace(/\.exe$/i, "");
}

function say(text: string, ok = true): void {
  const s = $<HTMLElement>("mt_status");
  s.textContent = text;
  s.classList.toggle("ok", ok && text !== "");
  s.classList.toggle("err", !ok);
  clearTimeout(statusTimer);
  if (ok) statusTimer = setTimeout(() => (s.textContent = ""), 3000);
}

function statusText(m: { id: string; status: Status; error?: string | null }): string {
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

function showLive(): void {
  clearInterval(liveTimer);
  const live = $<HTMLElement>("mt_live");
  const text = $<HTMLElement>("mt_live_text");
  const toggle = $<HTMLButtonElement>("mt_toggle");
  const rec = view.recording;
  toggle.textContent = rec ? "Stop recording" : "Start recording";
  toggle.className = rec ? "fl-btn fl-btn-destructive fl-btn-sm" : "fl-btn fl-btn-primary fl-btn-sm";
  $<HTMLElement>("mt_dot").classList.toggle("is-rec", !!rec);
  if (rec) {
    const tick = (): void => {
      text.textContent = `Recording · ${clock(Date.now() - rec.started_ms)} · your mic and the PC's sound`;
    };
    tick();
    liveTimer = setInterval(tick, 1000);
  } else if (view.transcribing) {
    const more = view.queued ? ` (${view.queued} more waiting)` : "";
    text.textContent = `Transcribing a meeting… ${Math.round(view.transcribing.percent)}%${more}. Dictation keeps priority.`;
  }
  live.hidden = !rec && !view.transcribing;
  const warn = $<HTMLElement>("mt_warn");
  warn.textContent = view.warning;
  warn.hidden = !view.warning;
}

function row(m: MeetingMeta): HTMLElement {
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
  if (line) text.append(el("span", `lw-mt-preview${m.status === "failed" ? " is-err" : ""}`, line));
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

function iconButton(label: string, icon: string, onClick: () => void): HTMLButtonElement {
  const b = el("button", "fl-iconbtn is-toolbar");
  b.type = "button";
  b.title = label;
  b.setAttribute("aria-label", label);
  b.append(iconEl(icon, 16));
  b.addEventListener("click", onClick);
  return b;
}

async function refreshList(): Promise<void> {
  let list: MeetingMeta[];
  try {
    list = await invoke<MeetingMeta[]>("meeting_list");
  } catch (e) {
    say(`Could not list the meetings: ${e}`, false);
    return;
  }
  $<HTMLElement>("mt_list").replaceChildren(...list.map(row));
  $<HTMLElement>("mt_empty").hidden = list.length > 0;
}

function showDetail(m: Meeting): void {
  $<HTMLElement>("mt_title").textContent = startedText(m.started_ms);
  const facts = [duration(m.duration_ms), appName(m.app), statusText(m)].filter(Boolean);
  $<HTMLElement>("mt_sub").textContent = facts.join(" · ");
  const note = $<HTMLElement>("mt_note");
  note.textContent = m.system_silent
    ? "No sound from the PC was recorded, so the transcript has only your side."
    : "";
  note.hidden = !m.system_silent;
  $<HTMLButtonElement>("mt_copy").disabled = m.status !== "done";
  $<HTMLButtonElement>("mt_delete").disabled = m.status === "recording";
  $<HTMLButtonElement>("mt_retry").hidden = m.status !== "failed";
  const turns = m.turns.map((t) => {
    const r = el("div", `lw-mt-turn is-${t.speaker}`);
    r.append(
      el("span", "lw-mt-stamp", clock(t.start_ms)),
      el("span", "lw-mt-who", t.speaker === "me" ? "Me" : "Others"),
      el("p", `lw-mt-said${t.error ? " is-err" : ""}`, t.error ? `Not transcribed: ${t.error}` : t.text),
    );
    return r;
  });
  if (!turns.length && m.status === "done") {
    turns.push(el("p", "lw-mt-note", "Nothing was said in this meeting."));
  }
  $<HTMLElement>("mt_turns").replaceChildren(...turns);
}

async function openMeeting(id: string): Promise<void> {
  try {
    const m = await invoke<Meeting>("meeting_get", { id });
    openId = id;
    showDetail(m);
    $<HTMLElement>("mt_list_view").hidden = true;
    $<HTMLElement>("mt_detail").hidden = false;
  } catch (e) {
    say(String(e), false);
    await refreshList();
  }
}

function closeMeeting(): void {
  openId = null;
  $<HTMLElement>("mt_detail").hidden = true;
  $<HTMLElement>("mt_list_view").hidden = false;
  void refreshList();
}

async function copy(id: string): Promise<void> {
  try {
    await invoke("meeting_copy", { id });
    say("Copied");
  } catch (e) {
    say(String(e), false);
  }
}

async function retry(id: string): Promise<void> {
  try {
    await invoke("meeting_retry", { id });
    say("Transcribing again");
  } catch (e) {
    say(String(e), false);
  }
  await refresh();
}

async function remove(id: string): Promise<void> {
  if (!(await askDialog("mt_del_dlg", "mt_del_ok", "mt_del_cancel"))) return;
  try {
    await invoke("meeting_delete", { id });
    say("Deleted");
    if (openId === id) closeMeeting();
  } catch (e) {
    say(String(e), false);
  }
  await refreshList();
}

async function toggle(): Promise<void> {
  const b = $<HTMLButtonElement>("mt_toggle");
  b.disabled = true;
  try {
    await invoke<boolean>("meeting_toggle");
  } catch (e) {
    say(String(e), false);
  }
  b.disabled = false;
  await refresh();
}

async function refresh(): Promise<void> {
  try {
    view = await invoke<MeetingView>("meeting_state");
  } catch {
    // not ready yet: keep the last view
  }
  showLive();
  if (openId) await openMeeting(openId);
  else await refreshList();
}

function showDetect(p: Prefs): void {
  const v = p.meeting_detection;
  $<HTMLSelectElement>("mt_detect").value = v === "ask" || v === "auto" ? v : "off";
}

async function saveDetect(): Promise<void> {
  const value = $<HTMLSelectElement>("mt_detect").value as Prefs["meeting_detection"];
  try {
    await prefsStore.save({ meeting_detection: value });
    say(value === "off" ? "Meeting detection is off" : "Saved");
  } catch (e) {
    say(String(e), false);
    const p = prefsStore.get();
    if (p) showDetect(p);
  }
}

function showShortcut(p: Prefs): void {
  const combo = p.shortcuts.meeting?.trim() ?? "";
  const keys = combo.split("+").map((k) => k.trim()).filter(Boolean);
  const box = $<HTMLElement>("mt_keys");
  box.replaceChildren(...keys.map((k) => el("kbd", "fl-kbd", k)));
  box.hidden = keys.length === 0;
  $<HTMLButtonElement>("mt_shortcut").textContent = keys.length ? "Change" : "Set a shortcut";
}

export const meetingsPage: Page = {
  id: "meetings",
  area: "main",
  template: TEMPLATE,
  init(): void {
    $<HTMLButtonElement>("mt_toggle").addEventListener("click", () => void toggle());
    $<HTMLButtonElement>("mt_back").addEventListener("click", closeMeeting);
    $<HTMLButtonElement>("mt_copy").addEventListener("click", () => openId && void copy(openId));
    $<HTMLButtonElement>("mt_delete").addEventListener("click", () => openId && void remove(openId));
    $<HTMLButtonElement>("mt_retry").addEventListener("click", () => openId && void retry(openId));
    prefsStore.subscribe(showShortcut);
    prefsStore.subscribe(showDetect);
    $<HTMLSelectElement>("mt_detect").addEventListener("change", () => void saveDetect());
    void onEvent<MeetingView>("meetings:changed", (e) => {
      view = e.payload;
      showLive();
      if (router.active() !== "meetings") return;
      if (openId) void openMeeting(openId);
      else void refreshList();
    });
  },
  show(): void {
    void refresh();
  },
};
