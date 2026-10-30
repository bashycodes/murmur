// Writing style per app group, and which group each app is in ("Style per
// app", D27). Rows of `prefs.app_buckets` (exe stem or web host -> group).
// Keys keep the spelling they were stored with, so old prefs save back
// unchanged; the backend matches them case- and `.exe`-insensitively
// (`context::bucket_key`).

import { $, button, el, setStatus } from "../lib/dom.js";
import { invoke } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { router, type Page } from "../lib/router.js";

const TEMPLATE = `
<div class="lw-page">
  <div class="lw-page-head">
    <div class="lw-grow">
      <h1 class="fl-page-title">Style</h1>
      <p class="lw-sub">How your words are written in each kind of app. How much AI rewrites them is set in <button type="button" class="lw-link" id="style_cleanup_link">AI cleanup →</button></p>
    </div>
    <span id="style_status" class="lw-status"></span>
  </div>
  <section class="lw-section">
    <h3 class="fl-group-title">Writing style</h3>
    <div class="fl-group">
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Personal messages</div><div class="fl-row-desc">WhatsApp, Telegram, Signal, Messenger, Discord, Instagram. <span class="lw-preview" id="style_personal_preview"></span></div></div>
        <div class="fl-row-control"><select id="style_personal" class="fl-select is-wide" aria-label="Personal messages style"></select></div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Work messages</div><div class="fl-row-desc">Slack, Teams. <span class="lw-preview" id="style_work_preview"></span></div></div>
        <div class="fl-row-control"><select id="style_work" class="fl-select is-wide" aria-label="Work messages style"></select></div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Email</div><div class="fl-row-desc">Outlook, Gmail. <span class="lw-preview" id="style_email_preview"></span></div></div>
        <div class="fl-row-control"><select id="style_email" class="fl-select is-wide" aria-label="Email style"></select></div>
      </div>
      <div class="fl-row">
        <div class="fl-row-text"><div class="fl-row-title">Everything else</div><div class="fl-row-desc"><span class="lw-preview" id="style_other_preview"></span></div></div>
        <div class="fl-row-control"><select id="style_other" class="fl-select is-wide" aria-label="Everything else style"></select></div>
      </div>
    </div>
  </section>
  <section class="lw-section">
    <h3 class="fl-group-title">Style per app</h3>
    <p class="lw-sub">Put an app in another group. Yours override the built-in groups below.</p>
    <div class="fl-list">
      <table class="apps">
        <tbody id="app_rows"></tbody>
      </table>
      <p id="app_rows_empty" class="lw-empty-row">No apps of your own yet. The built-in groups apply.</p>
    </div>
    <div class="lw-toolbar">
      <select id="app_recent" class="fl-select lw-grow" aria-label="Recently used app"></select>
      <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="app_add_recent">Add</button>
    </div>
    <div class="lw-toolbar">
      <input id="app_manual" class="fl-input lw-grow" type="text" spellcheck="false" placeholder="Other app: mychat.exe or mail.example.com" aria-label="Exe name or web host" />
      <button type="button" class="fl-btn fl-btn-secondary fl-btn-sm" id="app_add_manual">Add</button>
    </div>
    <p class="fl-help"><span id="app_add_status"></span> Recent apps are the exe names you dictated into (names only, kept on this PC). <button type="button" class="lw-link" id="app_recent_clear">Forget recent apps</button></p>
    <details class="defaults">
      <summary>Built-in groups (<span id="app_defaults_count">0</span> apps)</summary>
      <div class="fl-list">
        <table class="apps">
          <tbody id="app_default_rows"></tbody>
        </table>
      </div>
      <p class="fl-help">Web hosts (<code>mail.google.com</code>, <code>app.slack.com</code>…) are grouped the same way in a browser.</p>
    </details>
  </section>
</div>`;

const STYLES: [string, string][] = [
  ["formal", "Formal — capitals and full punctuation"],
  ["casual", "Casual — capitals, no final period"],
  ["very_casual", "Very casual — no final period; lowercase with AI cleanup"],
  ["excited", "Excited — more exclamation marks (AI cleanup)"],
];
/** Example line per style (shared with Android's Writing style page). */
const STYLE_PREVIEW: Record<string, string> = {
  formal: "“Sounds good. See you at 7:30.”",
  casual: "“Sounds good. See you at 7:30”",
  very_casual: "“sounds good. see you at 7:30”",
  excited: "“Sounds good! See you at 7:30!”",
};
const BUCKETS = ["personal", "work", "email", "other"] as const;
const BUCKET_NAMES = ["personal", "work", "email", "other"];

const BUCKET_LABELS: Record<string, string> = {
  personal: "Personal",
  work: "Work",
  email: "Email",
  other: "Other",
};

/** `app_style_choices`: recent exe names + the built-in exe -> group list. */
interface AppChoice {
  key: string;
  exe: string;
  name: string | null;
  default_bucket: string;
}
interface StyleChoices {
  recent: AppChoice[];
  defaults: AppChoice[];
}

let appRows: { key: string; bucket: string }[] = [];
let styleChoices: StyleChoices = { recent: [], defaults: [] };

function showStylePreview(b: string): void {
  const v = $<HTMLSelectElement>(`style_${b}`).value;
  $<HTMLElement>(`style_${b}_preview`).textContent = STYLE_PREVIEW[v] ?? "";
}

/** Mirrors `context::bucket_key`: `Slack.exe` -> `slack`, `www.X.com` -> `x.com`. */
function appKey(s: string): string {
  let k = s.trim().toLowerCase();
  if (k.endsWith(".exe")) k = k.slice(0, -4);
  if (k.startsWith("www.")) k = k.slice(4);
  return k;
}

/** Manual entry -> key to store, or an error. Accepts an exe name, a full
 *  exe path, a web host or a URL (its host is used). */
function parseAppInput(raw: string): { key: string } | { error: string } {
  let s = raw.trim();
  const url = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(s);
  if (url) s = url[1].replace(/^.*@/, "").replace(/:\d+$/, "");
  else if (/[\\/]/.test(s)) s = s.split(/[\\/]/).pop() ?? "";
  const key = appKey(s);
  if (!key) return { error: "Type an exe name (mychat.exe) or a web host (mail.example.com)" };
  if (key.includes("=")) return { error: "An app name can't contain “=”" };
  return { key };
}

function choiceFor(key: string): AppChoice | undefined {
  const k = appKey(key);
  return (
    styleChoices.recent.find((c) => c.key === k) ?? styleChoices.defaults.find((c) => c.key === k)
  );
}

function hasAppRow(key: string): boolean {
  const k = appKey(key);
  return appRows.some((r) => appKey(r.key) === k);
}

/** "Slack  slack.exe" (name when known, the stored key otherwise). */
function appCell(key: string, name: string | null | undefined, exe?: string): HTMLTableCellElement {
  const td = el("td");
  td.append(el("span", "", name || exe || key));
  if (name) td.append(el("span", "exe", exe ?? key));
  return td;
}

function bucketSelect(value: string, onChange: (v: string) => void): HTMLSelectElement {
  const sel = el("select", "fl-select");
  sel.setAttribute("aria-label", "Group");
  for (const b of BUCKETS) sel.append(new Option(BUCKET_LABELS[b], b));
  sel.value = BUCKET_NAMES.includes(value) ? value : "other";
  sel.addEventListener("change", () => onChange(sel.value));
  return sel;
}

function addAppRow(key: string, bucket: string): void {
  setStatus("app_add_status", "", true);
  if (hasAppRow(key)) {
    setStatus("app_add_status", "Already in the list", false);
    return;
  }
  appRows.push({ key, bucket });
  renderAppRows();
  void saveStyle();
  const sels = $<HTMLTableSectionElement>("app_rows").querySelectorAll("select");
  sels[appRows.length - 1]?.focus();
}

function renderAppRows(): void {
  const body = $<HTMLTableSectionElement>("app_rows");
  body.replaceChildren(
    ...appRows.map((row, i) => {
      const c = choiceFor(row.key);
      const label = c?.name ?? row.key;
      const del = button("Remove", () => {
        appRows.splice(i, 1);
        renderAppRows();
        void saveStyle();
      }, "fl-btn fl-btn-ghost fl-btn-sm");
      del.setAttribute("aria-label", `Remove ${label}`);
      const group = el("td");
      group.append(
        bucketSelect(row.bucket, (v) => {
          row.bucket = v;
          void saveStyle();
        }),
      );
      const act = el("td");
      act.append(del);
      const tr = el("tr");
      tr.append(appCell(row.key, c?.name, c?.exe), group, act);
      return tr;
    }),
  );
  $<HTMLElement>("app_rows_empty").hidden = appRows.length > 0;
  renderRecentPicker();
  renderDefaultRows();
}

function renderRecentPicker(): void {
  const sel = $<HTMLSelectElement>("app_recent");
  const free = styleChoices.recent.filter((c) => !hasAppRow(c.key));
  if (free.length === 0) {
    const o = new Option(
      styleChoices.recent.length
        ? "All recent apps are in the list"
        : "No recent apps yet — dictate into an app first",
      "",
    );
    sel.replaceChildren(o);
    sel.disabled = true;
  } else {
    sel.replaceChildren(
      ...free.map(
        (c) =>
          new Option(
            `${c.name ? `${c.name} (${c.exe})` : c.exe} — built-in: ${BUCKET_LABELS[c.default_bucket] ?? "Other"}`,
            c.key,
          ),
      ),
    );
    sel.disabled = false;
  }
  $<HTMLButtonElement>("app_add_recent").disabled = free.length === 0;
}

function renderDefaultRows(): void {
  const defs = styleChoices.defaults;
  $<HTMLElement>("app_defaults_count").textContent = String(defs.length);
  $<HTMLTableSectionElement>("app_default_rows").replaceChildren(
    ...defs.map((c) => {
      const tr = el("tr");
      const mine = hasAppRow(c.key);
      if (mine) tr.className = "overridden";
      const group = el("td");
      group.append(el("span", "fl-badge fl-badge-light", `default: ${BUCKET_LABELS[c.default_bucket] ?? c.default_bucket}`));
      const act = el("td");
      if (mine) act.append(el("span", "lw-muted", "yours above"));
      else act.append(button("Change", () => addAppRow(c.key, c.default_bucket)));
      tr.append(appCell(c.key, c.name, c.exe), group, act);
      return tr;
    }),
  );
}

function fillAppRows(m: Record<string, string> | undefined): void {
  appRows = Object.entries(m ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, bucket]) => ({ key, bucket }));
  renderAppRows();
}

async function loadAppChoices(): Promise<void> {
  try {
    styleChoices = await invoke<StyleChoices>("app_style_choices");
  } catch {
    styleChoices = { recent: [], defaults: [] };
  }
  renderAppRows();
}

function addRecentApp(): void {
  const key = $<HTMLSelectElement>("app_recent").value;
  const c = styleChoices.recent.find((r) => r.key === key);
  if (c) addAppRow(c.key, c.default_bucket);
}

function addManualApp(): void {
  const input = $<HTMLInputElement>("app_manual");
  const r = parseAppInput(input.value);
  if ("error" in r) {
    setStatus("app_add_status", r.error, false);
    return;
  }
  const before = appRows.length;
  addAppRow(r.key, choiceFor(r.key)?.default_bucket ?? "other");
  if (appRows.length > before) input.value = "";
}

function fill(p: Prefs): void {
  for (const b of BUCKETS) {
    const v = p.styles[b].toLowerCase().replace(/[ -]/g, "_");
    $<HTMLSelectElement>(`style_${b}`).value = STYLES.some(([k]) => k === v) ? v : "formal";
    showStylePreview(b);
  }
  fillAppRows(p.app_buckets);
}

/** Every change saves at once (the page has no Save button). */
async function saveStyle(): Promise<void> {
  try {
    await prefsStore.save(collectStyle());
    setStatus("style_status", "Saved", true);
  } catch (e) {
    setStatus("style_status", String(e), false);
  }
}

/** The prefs fields this page owns. */
function collectStyle(): Partial<Prefs> {
  const app_buckets: Record<string, string> = {};
  for (const r of appRows) app_buckets[r.key] = r.bucket;
  return {
    styles: {
      personal: $<HTMLSelectElement>("style_personal").value,
      work: $<HTMLSelectElement>("style_work").value,
      email: $<HTMLSelectElement>("style_email").value,
      other: $<HTMLSelectElement>("style_other").value,
    },
    app_buckets,
  };
}

export const stylePage: Page = {
  id: "style",
  area: "main",
  template: TEMPLATE,
  init(): void {
    for (const b of BUCKETS) {
      $<HTMLSelectElement>(`style_${b}`).replaceChildren(
        ...STYLES.map(([v, label]) => new Option(label, v)),
      );
      $<HTMLSelectElement>(`style_${b}`).addEventListener("change", () => {
        showStylePreview(b);
        void saveStyle();
      });
    }
    $<HTMLButtonElement>("style_cleanup_link").addEventListener("click", () => router.go("settings-cleanup"));
    $<HTMLButtonElement>("app_add_recent").addEventListener("click", addRecentApp);
    $<HTMLButtonElement>("app_add_manual").addEventListener("click", addManualApp);
    $<HTMLInputElement>("app_manual").addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        addManualApp();
      }
    });
    $<HTMLButtonElement>("app_recent_clear").addEventListener("click", () => {
      void invoke("recent_apps_clear").then(loadAppChoices, (e) =>
        setStatus("app_add_status", String(e), false),
      );
    });
    // First load only: this page's own saves already show what they saved.
    let loaded = false;
    prefsStore.subscribe((p) => {
      if (loaded) return;
      loaded = true;
      fill(p);
      void loadAppChoices();
    });
  },
  show(): void {
    void loadAppChoices();
  },
};
