// Small DOM helpers shared by every page. User text always goes in with
// textContent; `html()` is only for the pages' own static templates.

import { Serial } from "./serial.js";

export const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

export function button(label: string, onClick: () => void, cls = "fl-btn fl-btn-secondary fl-btn-sm"): HTMLButtonElement {
  const b = el("button", cls, label);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

export function setStatus(id: string, text: string, ok: boolean): void {
  const s = $<HTMLElement>(id);
  s.textContent = text;
  s.classList.toggle("ok", ok);
  s.classList.toggle("err", !ok);
}

/** A page's static markup (never user text) as nodes. */
export function html(markup: string): DocumentFragment {
  const t = document.createElement("template");
  t.innerHTML = markup.trim();
  return t.content;
}

const SVG_NS = "http://www.w3.org/2000/svg";

export function svg(tag: string, attrs: Record<string, string | number>, text?: string): SVGElement {
  const e = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  if (text !== undefined) e.textContent = text;
  return e;
}

/** One turn at a time per dialog: a second ask while it is open waits. */
const dialogTurns = new Map<string, Serial>();

/** Show a <dialog> with an OK and a Cancel button; resolves to the choice.
 *  Asked again while it is open, the new ask waits for the open one to be
 *  answered (taking over its buttons would leave the first never answered).
 *  `prepare` fills the dialog's text just before it is shown. */
export function askDialog(id: string, okId: string, cancelId: string, prepare?: () => void): Promise<boolean> {
  let turns = dialogTurns.get(id);
  if (!turns) {
    turns = new Serial();
    dialogTurns.set(id, turns);
  }
  return turns.run(() => {
    prepare?.();
    return showDialog(id, okId, cancelId);
  });
}

function showDialog(id: string, okId: string, cancelId: string): Promise<boolean> {
  const d = $<HTMLDialogElement>(id);
  return new Promise((resolve) => {
    const done = (ok: boolean): void => {
      $<HTMLButtonElement>(okId).onclick = null;
      $<HTMLButtonElement>(cancelId).onclick = null;
      d.oncancel = null;
      d.close();
      resolve(ok);
    };
    $<HTMLButtonElement>(okId).onclick = () => done(true);
    $<HTMLButtonElement>(cancelId).onclick = () => done(false);
    d.oncancel = (e) => {
      e.preventDefault();
      done(false);
    };
    d.showModal();
  });
}

/** Time today, else date + time. */
export function when(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  return d.toDateString() === today.toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

/** Set a field unless the user is typing in it. */
export function setValue(id: string, v: string): void {
  const i = $<HTMLInputElement>(id);
  if (document.activeElement !== i) i.value = v;
}
