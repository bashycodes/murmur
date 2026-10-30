// Transforms page (PageTransforms.dc.html, docs/proposals/transforms.md):
// the master "Opt in", the "view changes" keys, the auto-apply choice and
// the saved transforms (built-ins Polish and Prompt Engineer plus the
// user's own), each with its shortcut. Everything is `prefs.transforms`
// (src/transforms.rs), saved through prefsStore; "Reset to defaults" is
// `transforms_reset` (the built-ins come back, custom ones stay) and the
// banner's "Try it out" runs a transform on sample text (`transform_try`).

import { $, askDialog, el } from "../lib/dom.js";
import { ICON, iconEl, iconSvg } from "../lib/icons.js";
import { invoke } from "../lib/ipc.js";
import { captureChord, RECORDING_TEXT, stopCapture } from "../lib/keycapture.js";
import { prefsStore, type Prefs, type TransformItem, type TransformPrefs } from "../lib/prefs-store.js";
import type { Page } from "../lib/router.js";

/** flow_core::transform limits (the backend checks them again). */
const MAX_TRANSFORMS = 20;
const MAX_NAME = 40;
const MAX_DESCRIPTION = 200;
const MAX_INSTRUCTION = 2000;

const REFRESH_ICON = "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7";

const TEMPLATE = `
<div class="lw-page lw-tf">
  <div class="lw-page-head lw-tf-head">
    <h1 class="fl-page-title">Transforms</h1>
    <span class="fl-badge fl-badge-dark">Beta</span>
    <span class="lw-tf-grow"></span>
    <label class="lw-tf-optin" for="tf_enabled">Opt in</label>
    <input type="checkbox" role="switch" class="fl-toggle" id="tf_enabled" aria-label="Opt in to Transforms" />
    <span class="fl-kbd-group lw-tf-view" id="tf_view" title="Opens Home with the last transform's before and after"><span id="tf_view_keys"></span><span class="fl-kbd-text">to view changes</span></span>
  </div>

  <div class="fl-banner fl-banner-mist">
    <h3 class="fl-banner-title">Transform works anywhere you write</h3>
    <p class="fl-banner-body">Select text in any app, press a shortcut, and it's rewritten in place. Uses your AI cleanup engine: OpenRouter, then this PC.</p>
    <div class="fl-banner-actions">
      <button type="button" class="fl-btn fl-btn-lumen" id="tf_try">Try it out</button>
      <button type="button" class="fl-btn fl-btn-link-light" data-go="settings-cleanup">How it works</button>
    </div>
  </div>

  <div class="fl-row lw-tf-auto">
    <div class="fl-row-text"><div class="fl-row-title">Auto Apply After Dictation</div><div class="fl-row-desc" id="tf_auto_desc">Pass each dictation through the chosen transform before it is pasted.</div></div>
    <select class="fl-select lw-tf-auto-pick" id="tf_auto_pick" aria-label="Transform to apply after dictation"></select>
    <input type="checkbox" role="switch" class="fl-toggle" id="tf_auto" aria-label="Auto apply after dictation" />
  </div>

  <div class="lw-tf-bar">
    <h2 class="fl-serif lw-tf-h2">My Transforms</h2>
    <button type="button" class="fl-btn fl-btn-ghost fl-btn-sm" id="tf_reset">${iconSvg(REFRESH_ICON, 16)}Reset to defaults</button>
    <button type="button" class="fl-btn fl-btn-primary fl-btn-sm" id="tf_new">Create New</button>
  </div>

  <div class="lw-tf-grid" id="tf_grid"></div>
  <p class="fl-help"><span id="tf_status" role="status"></span></p>

  <dialog id="tf_edit_dlg" class="fl-dialog lw-tf-dialog" aria-labelledby="tf_edit_title">
    <h3 id="tf_edit_title">New transform</h3>
    <label class="lw-tf-field"><span>Name</span>
      <input class="fl-input" id="tf_name" maxlength="${MAX_NAME}" placeholder="Make it friendlier" /></label>
    <label class="lw-tf-field"><span>Description <em>(optional)</em></span>
      <input class="fl-input" id="tf_desc" maxlength="${MAX_DESCRIPTION}" placeholder="Shown on the card" /></label>
    <label class="lw-tf-field"><span>Instruction</span>
      <textarea class="fl-input" id="tf_instruction" rows="5" maxlength="${MAX_INSTRUCTION}" placeholder="Rewrite the text so it sounds warm and friendly. Keep it short."></textarea></label>
    <div class="lw-tf-field"><label for="tf_shortcut">Shortcut <em>(optional)</em></label>
      <span class="lw-tf-keyrow"><input class="fl-input" id="tf_shortcut" placeholder="Win+Alt+3" spellcheck="false" />
      <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="tf_shortcut_rec">Press keys</button></span></div>
    <p class="lw-tf-hint" id="tf_builtin_hint" hidden>A built-in transform: Reset to defaults puts back its shipped wording.</p>
    <p class="lw-tf-err" id="tf_edit_err" role="alert"></p>
    <div class="lw-dialog-actions">
      <button type="button" class="fl-btn fl-btn-destructive lw-tf-del" id="tf_edit_delete">Delete</button>
      <button type="button" class="fl-btn fl-btn-secondary" id="tf_edit_cancel">Cancel</button>
      <button type="button" class="fl-btn fl-btn-primary" id="tf_edit_save">Save</button>
    </div>
  </dialog>

  <dialog id="tf_reset_dlg" class="fl-dialog">
    <h3>Reset to defaults?</h3>
    <p>Polish and Prompt Engineer go back to their shipped wording and shortcuts. Transforms you created are kept.</p>
    <div class="lw-dialog-actions">
      <button type="button" class="fl-btn fl-btn-secondary" id="tf_reset_cancel">Cancel</button>
      <button type="button" class="fl-btn fl-btn-primary" id="tf_reset_ok">Reset</button>
    </div>
  </dialog>

  <dialog id="tf_try_dlg" class="fl-dialog lw-tf-dialog" aria-labelledby="tf_try_title">
    <h3 id="tf_try_title">Try a transform</h3>
    <label class="lw-tf-field"><span>Transform</span>
      <select class="fl-select" id="tf_try_pick"></select></label>
    <label class="lw-tf-field"><span>Text</span>
      <textarea class="fl-input" id="tf_try_text" rows="4">hey so i was thinking we could maybe move the meeting to thursday if thats ok with everyone, let me know</textarea></label>
    <div class="lw-tf-field"><span>Result</span>
      <p class="lw-tf-result" id="tf_try_out" aria-live="polite"></p></div>
    <div class="lw-dialog-actions">
      <button type="button" class="fl-btn fl-btn-secondary" id="tf_try_close">Close</button>
      <button type="button" class="fl-btn fl-btn-primary" id="tf_try_run">Transform</button>
    </div>
  </dialog>
</div>`;

const DEFAULTS: TransformPrefs = { enabled: false, auto_apply: null, view_changes: "Win+Alt+O", items: [] };

function current(): TransformPrefs {
  return prefsStore.get()?.transforms ?? DEFAULTS;
}

function keys(combo: string | null | undefined): HTMLElement[] {
  return (combo ?? "")
    .split("+")
    .map((k) => k.trim())
    .filter(Boolean)
    .map((k) => el("kbd", "fl-kbd", k));
}

let statusTimer = 0;
function say(text: string, ok = true): void {
  const s = $<HTMLElement>("tf_status");
  s.textContent = text;
  s.classList.toggle("ok", ok && text !== "");
  s.classList.toggle("err", !ok);
  clearTimeout(statusTimer);
  if (ok) statusTimer = setTimeout(() => (s.textContent = ""), 3000);
}

/** Save the whole `transforms` block; returns the backend's error, if any. */
async function save(next: TransformPrefs, okText = "Saved"): Promise<string | null> {
  try {
    await prefsStore.save({ transforms: next });
    say(okText);
    return null;
  } catch (e) {
    const msg = String(e);
    say(msg, false);
    render(prefsStore.get() as Prefs); // put the controls back
    return msg;
  }
}

/** `My Transform!` → `my-transform`, unique among `taken`. */
export function newId(name: string, taken: string[]): string {
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9_]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40) || "transform";
  let id = base;
  for (let k = 2; taken.includes(id); k++) id = `${base}-${k}`;
  return id;
}

/** The first Win+Alt+digit no shortcut uses yet ("" when all are taken). */
function freeChord(t: TransformPrefs): string {
  const used = new Set(t.items.map((i) => (i.shortcut ?? "").toLowerCase()));
  for (let d = 1; d <= 9; d++) {
    const c = `Win+Alt+${d}`;
    if (!used.has(c.toLowerCase())) return c;
  }
  return "";
}

function card(item: TransformItem): HTMLElement {
  const t = item.transform;
  const c = el("button", "lw-tf-card");
  c.type = "button";
  c.dataset.id = t.id;
  c.title = "Edit";
  const k = el("span", "fl-kbd-group lw-tf-keys");
  const ks = keys(item.shortcut);
  k.replaceChildren(...(ks.length ? ks : [el("span", "lw-tf-nokeys", "No shortcut")]));
  c.append(k, el("span", "lw-tf-name", t.name));
  if (t.description) c.append(el("span", "lw-tf-desc", t.description));
  c.addEventListener("click", () => edit(t.id));
  return c;
}

function render(p: Prefs): void {
  const t = p.transforms ?? DEFAULTS;
  $<HTMLInputElement>("tf_enabled").checked = t.enabled;
  $<HTMLElement>("tf_view_keys").replaceChildren(...keys(t.view_changes));
  $<HTMLElement>("tf_view").hidden = !t.view_changes;

  // Auto apply: the toggle saves the picked transform (or none).
  const pick = $<HTMLSelectElement>("tf_auto_pick");
  const keep = t.auto_apply ?? (pick.value || t.items[0]?.transform.id || "");
  pick.replaceChildren(...t.items.map((i) => new Option(i.transform.name, i.transform.id)));
  pick.value = t.items.some((i) => i.transform.id === keep) ? keep : (t.items[0]?.transform.id ?? "");
  const auto = $<HTMLInputElement>("tf_auto");
  auto.checked = t.auto_apply !== null && t.items.some((i) => i.transform.id === t.auto_apply);
  auto.disabled = !t.enabled || t.items.length === 0;
  pick.disabled = !auto.checked || auto.disabled;
  $<HTMLElement>("tf_auto_desc").textContent = t.enabled
    ? "Pass each dictation through the chosen transform before it is pasted."
    : "Pass each dictation through the chosen transform before it is pasted. Opt in to Transforms first.";

  const grid = $<HTMLElement>("tf_grid");
  const own = el("button", "lw-tf-card lw-tf-own");
  own.type = "button";
  own.id = "tf_own";
  const plus = el("span", "lw-tf-plus");
  plus.append(iconEl(ICON.plus, 14));
  own.append(plus, el("span", "lw-tf-name", "Create your own"), el("span", "lw-tf-desc", "Write your own instruction"));
  own.addEventListener("click", () => edit(null));
  const full = t.items.length >= MAX_TRANSFORMS;
  own.hidden = full;
  grid.replaceChildren(...t.items.map(card), own);
  const nb = $<HTMLButtonElement>("tf_new");
  nb.disabled = full;
  nb.title = full ? `At most ${MAX_TRANSFORMS} transforms` : "";
}

// ------------------------------------------------------------ editor

let editing: string | null = null;

function closeEditor(): void {
  $<HTMLDialogElement>("tf_edit_dlg").close();
}

// "Press keys": the keyboard hook records the chord (lib/keycapture.ts);
// the field stays for typing one by hand.
let recordingKeys = false;

function showRecordingKeys(on: boolean): void {
  recordingKeys = on;
  const input = $<HTMLInputElement>("tf_shortcut");
  input.placeholder = on ? RECORDING_TEXT : "Win+Alt+3";
  input.classList.toggle("is-recording", on);
  $<HTMLButtonElement>("tf_shortcut_rec").textContent = on ? "Stop" : "Press keys";
}

async function recordShortcut(): Promise<void> {
  const input = $<HTMLInputElement>("tf_shortcut");
  if (recordingKeys) {
    showRecordingKeys(false);
    stopCapture();
    return;
  }
  const before = input.value;
  input.value = "";
  showRecordingKeys(true);
  const r = await captureChord();
  if (!recordingKeys) {
    input.value ||= before;
    return;
  }
  showRecordingKeys(false);
  if ("chord" in r) {
    input.value = r.chord;
  } else {
    input.value = before;
    if (r.message) $<HTMLElement>("tf_edit_err").textContent = r.message;
  }
}

function stopRecordingKeys(): void {
  if (!recordingKeys) return;
  showRecordingKeys(false);
  stopCapture();
}

function edit(id: string | null): void {
  const t = current();
  const item = id ? t.items.find((i) => i.transform.id === id) : undefined;
  editing = item ? item.transform.id : null;
  $<HTMLElement>("tf_edit_title").textContent = item ? "Edit transform" : "New transform";
  $<HTMLInputElement>("tf_name").value = item?.transform.name ?? "";
  $<HTMLInputElement>("tf_desc").value = item?.transform.description ?? "";
  $<HTMLTextAreaElement>("tf_instruction").value = item?.transform.instruction ?? "";
  $<HTMLInputElement>("tf_shortcut").value = item ? (item.shortcut ?? "") : freeChord(t);
  $<HTMLElement>("tf_builtin_hint").hidden = !(item && BUILTINS.includes(item.transform.id));
  $<HTMLElement>("tf_edit_delete").hidden = !item;
  $<HTMLElement>("tf_edit_err").textContent = "";
  $<HTMLDialogElement>("tf_edit_dlg").showModal();
  $<HTMLInputElement>("tf_name").focus();
}

const BUILTINS = ["polish", "prompt_engineer"];

async function saveEditor(): Promise<void> {
  const err = $<HTMLElement>("tf_edit_err");
  const name = $<HTMLInputElement>("tf_name").value.trim();
  const description = $<HTMLInputElement>("tf_desc").value.trim();
  const instruction = $<HTMLTextAreaElement>("tf_instruction").value.trim();
  const shortcut = $<HTMLInputElement>("tf_shortcut").value.trim();
  if (!name) {
    err.textContent = "Give the transform a name.";
    $<HTMLInputElement>("tf_name").focus();
    return;
  }
  if (!instruction) {
    err.textContent = "Write the instruction: what should happen to the text?";
    $<HTMLTextAreaElement>("tf_instruction").focus();
    return;
  }
  const t = current();
  const items = t.items.map((i) => structuredClone(i));
  if (editing) {
    const it = items.find((i) => i.transform.id === editing);
    if (it) {
      it.transform = { ...it.transform, name, description, instruction };
      it.shortcut = shortcut || null;
    }
  } else {
    const id = newId(name, items.map((i) => i.transform.id));
    items.push({ transform: { id, name, description, instruction }, shortcut: shortcut || null });
  }
  const problem = await save({ ...t, items }, editing ? "Saved" : `Added ${name}`);
  if (problem) {
    err.textContent = problem;
    return;
  }
  closeEditor();
}

async function deleteEditing(): Promise<void> {
  if (!editing) return;
  const t = current();
  const gone = t.items.find((i) => i.transform.id === editing);
  const items = t.items.filter((i) => i.transform.id !== editing);
  const auto_apply = t.auto_apply === editing ? null : t.auto_apply;
  const problem = await save({ ...t, items, auto_apply }, `Deleted ${gone?.transform.name ?? ""}`.trim());
  if (problem) {
    $<HTMLElement>("tf_edit_err").textContent = problem;
    return;
  }
  closeEditor();
}

// ------------------------------------------------------------ try it out

function openTry(): void {
  const t = current();
  const pick = $<HTMLSelectElement>("tf_try_pick");
  const keep = pick.value;
  pick.replaceChildren(...t.items.map((i) => new Option(i.transform.name, i.transform.id)));
  if (t.items.some((i) => i.transform.id === keep)) pick.value = keep;
  $<HTMLElement>("tf_try_out").textContent = "";
  $<HTMLElement>("tf_try_out").classList.remove("err");
  $<HTMLButtonElement>("tf_try_run").disabled = t.items.length === 0;
  $<HTMLDialogElement>("tf_try_dlg").showModal();
}

async function runTry(): Promise<void> {
  const out = $<HTMLElement>("tf_try_out");
  const run = $<HTMLButtonElement>("tf_try_run");
  const text = $<HTMLTextAreaElement>("tf_try_text").value;
  if (!text.trim()) {
    out.textContent = "Type some text to transform.";
    out.classList.add("err");
    return;
  }
  run.disabled = true;
  out.classList.remove("err");
  out.textContent = "Transforming…";
  try {
    out.textContent = await invoke<string>("transform_try", { id: $<HTMLSelectElement>("tf_try_pick").value, text });
  } catch (e) {
    out.textContent = String(e);
    out.classList.add("err");
  } finally {
    run.disabled = false;
  }
}

export const transformsPage: Page = {
  id: "transforms",
  area: "main",
  template: TEMPLATE,
  init(): void {
    prefsStore.subscribe(render);

    $<HTMLInputElement>("tf_enabled").addEventListener("change", (e) => {
      const on = (e.target as HTMLInputElement).checked;
      void save({ ...current(), enabled: on }, on ? "Transforms on: the shortcuts work in any app" : "Transforms off");
    });
    $<HTMLInputElement>("tf_auto").addEventListener("change", (e) => {
      const on = (e.target as HTMLInputElement).checked;
      const id = $<HTMLSelectElement>("tf_auto_pick").value || null;
      void save({ ...current(), auto_apply: on ? id : null }, on ? "Every dictation is transformed before it's pasted" : "Auto apply off");
    });
    $<HTMLSelectElement>("tf_auto_pick").addEventListener("change", (e) => {
      if (!$<HTMLInputElement>("tf_auto").checked) return;
      void save({ ...current(), auto_apply: (e.target as HTMLSelectElement).value });
    });

    $<HTMLButtonElement>("tf_new").addEventListener("click", () => edit(null));
    $<HTMLButtonElement>("tf_edit_save").addEventListener("click", () => void saveEditor());
    $<HTMLButtonElement>("tf_edit_cancel").addEventListener("click", () => closeEditor());
    $<HTMLButtonElement>("tf_edit_delete").addEventListener("click", () => void deleteEditing());
    $<HTMLButtonElement>("tf_shortcut_rec").addEventListener("click", () => void recordShortcut());
    // Typing in the field (or closing the dialog) ends a recording.
    $<HTMLInputElement>("tf_shortcut").addEventListener("input", stopRecordingKeys);
    $<HTMLDialogElement>("tf_edit_dlg").addEventListener("close", stopRecordingKeys);
    $<HTMLInputElement>("tf_name").addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        void saveEditor();
      }
    });

    $<HTMLButtonElement>("tf_reset").addEventListener("click", async () => {
      if (!(await askDialog("tf_reset_dlg", "tf_reset_ok", "tf_reset_cancel"))) return;
      try {
        prefsStore.adopt(await invoke<Prefs>("transforms_reset"));
        say("Built-in transforms restored");
      } catch (e) {
        say(String(e), false);
      }
    });

    $<HTMLButtonElement>("tf_try").addEventListener("click", openTry);
    $<HTMLButtonElement>("tf_try_run").addEventListener("click", () => void runTry());
    $<HTMLButtonElement>("tf_try_close").addEventListener("click", () => $<HTMLDialogElement>("tf_try_dlg").close());

    const p = prefsStore.get();
    if (p) render(p);
  },
};
