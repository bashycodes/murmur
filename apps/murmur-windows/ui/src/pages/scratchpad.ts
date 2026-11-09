// Scratchpad page (PageScratchpad.dc.html): the notes kept on this PC,
// pinned first, then the most recent. Opening a note (or New note)
// shows it in the notes window (`note_open`; ui/src/scratchpad.ts is that
// window's editor). This window never reads or writes a note's text itself:
// it only lists (`note_list`), creates (`note_create`) and pins
// (`note_set_pinned`) — the capability split tests/capabilities.rs checks.
// The design's "Add to Murmur bar" toggle has no backend and is left out.

import { $, el } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { router, type Page } from "../lib/router.js";

/** `notes::NoteMeta` (src/notes.rs). */
export interface NoteMeta {
  id: string;
  title: string;
  /** The Quick note's first line; other notes' line under the title. */
  preview: string;
  words: number;
  created_ms: number;
  updated_ms: number;
  pinned: boolean;
  quick: boolean;
}

const NOTE_ICON = "M5 4h14v16H5zM8.5 9h7M8.5 13h4";
const PIN_ICON = "M9 4h6M10 4v6l-3 4h10l-3-4V4M12 14v6";

const TEMPLATE = `
<div class="lw-page lw-sp">
  <div class="lw-page-head lw-sp-head">
    <h1 class="fl-page-title">Scratchpad</h1>
    <span class="fl-badge fl-badge-dark">Beta</span>
    <span class="lw-sp-grow"></span>
    <span class="fl-kbd-group" id="sp_keys" hidden title="Opens the Quick note from any app"></span>
    <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="sp_shortcut" data-go="settings-general">Set a shortcut</button>
  </div>

  <div class="fl-banner fl-banner-amber">
    <h3 class="fl-banner-title">Notes you dictate and keep</h3>
    <p class="fl-banner-body">Speak a list, draft a reply before it goes out, or get an idea down while it's fresh. Notes are saved on this PC only.</p>
    <div class="fl-banner-actions">
      <button type="button" class="fl-btn fl-btn-lumen" id="sp_new">${iconSvg(ICON.plus)}New note</button>
    </div>
  </div>

  <div class="lw-sp-bar">
    <span class="lw-sp-bar-title">Recents</span>
    <input type="search" class="fl-input lw-sp-search" id="sp_search" placeholder="Search notes" aria-label="Search notes" hidden />
    <button type="button" class="fl-iconbtn is-toolbar" id="sp_search_btn" aria-label="Search notes" title="Search notes" aria-expanded="false">${iconSvg(ICON.search)}</button>
    <button type="button" class="fl-iconbtn is-toolbar" id="sp_new2" aria-label="New note" title="New note">${iconSvg(ICON.plus)}</button>
  </div>

  <section id="sp_pinned_sec" hidden>
    <div class="fl-overline lw-sp-sub">Pinned</div>
    <div class="fl-list" id="sp_pinned"></div>
  </section>
  <section id="sp_recent_sec">
    <div class="fl-overline lw-sp-sub" id="sp_recent_title" hidden>Recent</div>
    <div class="fl-list" id="sp_recent"></div>
  </section>
  <p class="lw-sp-empty" id="sp_empty" hidden>No notes match.</p>
  <p class="fl-help"><span id="sp_status" role="status"></span></p>
</div>`;

/** "2:14pm" today, "Yesterday", "Oct 22" this year, else "Oct 22, 2025". */
export function noteWhen(ms: number, now = new Date()): string {
  if (!ms) return "";
  const d = new Date(ms);
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).replace(/\s?([AP])M$/i, (_, a: string) => `${a.toLowerCase()}m`);
  }
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], d.getFullYear() === now.getFullYear() ? { month: "short", day: "numeric" } : { month: "short", day: "numeric", year: "numeric" });
}

let searchTimer = 0;
let statusTimer = 0;

function say(text: string, ok = true): void {
  const s = $<HTMLElement>("sp_status");
  s.textContent = text;
  s.classList.toggle("ok", ok && text !== "");
  s.classList.toggle("err", !ok);
  clearTimeout(statusTimer);
  if (ok) statusTimer = setTimeout(() => (s.textContent = ""), 3000);
}

async function openNote(id: string): Promise<void> {
  try {
    await invoke("note_open", { id });
  } catch (e) {
    say(`Could not open the note: ${e}`, false);
  }
}

async function newNote(): Promise<void> {
  try {
    const id = await invoke<string>("note_create");
    await openNote(id);
  } catch (e) {
    say(`Could not create a note: ${e}`, false);
  }
  await refresh();
}

function row(n: NoteMeta): HTMLElement {
  const r = el("div", "fl-list-row lw-sp-row");
  r.dataset.id = n.id;
  r.classList.toggle("is-pinned", n.pinned);

  const open = el("button", "lw-sp-open");
  open.type = "button";
  open.title = "Open in the note window";
  const text = el("span", "lw-sp-text");
  const title = el("span", "lw-sp-title", n.title);
  text.append(title);
  const sub = n.preview || (n.words ? "" : "Empty");
  if (sub) text.append(el("span", "lw-sp-preview", sub));
  const ico = iconEl(NOTE_ICON, 20);
  ico.classList.add("lw-sp-ico");
  open.append(ico, text, el("span", "fl-list-time lw-sp-when", noteWhen(n.updated_ms)));
  open.addEventListener("click", () => void openNote(n.id));

  const pin = el("button", "fl-iconbtn is-toolbar lw-sp-pin");
  pin.type = "button";
  const label = n.pinned ? "Unpin" : "Pin to the top";
  pin.title = label;
  pin.setAttribute("aria-label", `${label}: ${n.title}`);
  pin.setAttribute("aria-pressed", String(n.pinned));
  pin.append(iconEl(PIN_ICON, 18));
  pin.addEventListener("click", async () => {
    try {
      await invoke("note_set_pinned", { id: n.id, pinned: !n.pinned });
    } catch (e) {
      say(String(e), false);
    }
    await refresh();
  });

  r.append(open, pin);
  return r;
}

async function refresh(): Promise<void> {
  const query = $<HTMLInputElement>("sp_search").value;
  let notes: NoteMeta[];
  try {
    notes = await invoke<NoteMeta[]>("note_list", { query });
  } catch (e) {
    say(`Could not list the notes: ${e}`, false);
    return;
  }
  const pinned = notes.filter((n) => n.pinned);
  const recent = notes.filter((n) => !n.pinned);
  $<HTMLElement>("sp_pinned").replaceChildren(...pinned.map(row));
  $<HTMLElement>("sp_recent").replaceChildren(...recent.map(row));
  $<HTMLElement>("sp_pinned_sec").hidden = pinned.length === 0;
  $<HTMLElement>("sp_recent_title").hidden = pinned.length === 0 || recent.length === 0;
  $<HTMLElement>("sp_recent_sec").hidden = recent.length === 0;
  $<HTMLElement>("sp_empty").hidden = notes.length > 0;
}

function showShortcut(p: Prefs): void {
  const combo = p.shortcuts.scratchpad?.trim() ?? "";
  const keys = combo.split("+").map((k) => k.trim()).filter(Boolean);
  const box = $<HTMLElement>("sp_keys");
  box.replaceChildren(...keys.map((k) => el("kbd", "fl-kbd", k)));
  box.hidden = keys.length === 0;
  const b = $<HTMLButtonElement>("sp_shortcut");
  b.textContent = keys.length ? "Change" : "Set a shortcut";
  b.className = keys.length ? "fl-btn fl-btn-ghost fl-btn-sm" : "fl-btn fl-btn-secondary fl-btn-sm";
  b.title = keys.length ? "Settings › General › Shortcuts" : "Open the Scratchpad from any app";
}

export const scratchpadPage: Page = {
  id: "scratchpad",
  area: "main",
  template: TEMPLATE,
  init(): void {
    $<HTMLButtonElement>("sp_new").addEventListener("click", () => void newNote());
    $<HTMLButtonElement>("sp_new2").addEventListener("click", () => void newNote());
    const search = $<HTMLInputElement>("sp_search");
    const searchBtn = $<HTMLButtonElement>("sp_search_btn");
    searchBtn.addEventListener("click", () => {
      const show = search.hidden;
      search.hidden = !show;
      searchBtn.setAttribute("aria-expanded", String(show));
      if (show) search.focus();
      else if (search.value) {
        search.value = "";
        void refresh();
      }
    });
    search.addEventListener("input", () => {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => void refresh(), 150);
    });
    search.addEventListener("keydown", (e) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      searchBtn.click();
      searchBtn.focus();
    });
    prefsStore.subscribe(showShortcut);
    // Edits happen in the notes window: list again when this one comes back.
    window.addEventListener("focus", () => {
      if (router.active() === "scratchpad") void refresh();
    });
  },
  show(): void {
    void refresh();
  },
};
