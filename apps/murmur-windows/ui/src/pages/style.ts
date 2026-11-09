// Style page (docs/design/app-design/PageStyle.dc.html): a tab per app group,
// a banner with the apps in that group, and a card per writing style.
// Below it, which group each app is in ("Style per app", D27). Rows of `prefs.app_buckets` (exe stem or web host -> group).
// Keys keep the spelling they were stored with, so old prefs save back
// unchanged; the backend matches them case- and `.exe`-insensitively
// (`context::bucket_key`).

import { $, button, el, setStatus } from "../lib/dom.js";
import { ICON, iconEl } from "../lib/icons.js";
import { invoke } from "../lib/ipc.js";
import { prefsStore, type Prefs } from "../lib/prefs-store.js";
import { router, type Page } from "../lib/router.js";

const TEMPLATE = `
<div class="lw-page lw-style">
  <div class="lw-page-head">
    <h1 class="fl-page-title">Style</h1>
    <span id="style_status" class="lw-status"></span>
  </div>
  <div class="fl-tabs">
    <div class="fl-tabs-list" role="tablist" aria-label="Kind of app" id="style_tabs"></div>
  </div>
  <div class="fl-banner fl-banner-night lw-style-banner" role="tabpanel" id="style_panel">
    <div class="lw-style-banner-text">
      <h3 class="fl-banner-title" id="style_banner_title"></h3>
      <p class="fl-banner-body">How much AI rewrites your words is set in <button type="button" class="lw-link lw-style-banner-link" id="style_cleanup_link">AI cleanup →</button></p>
    </div>
    <div class="lw-style-chips" id="style_chips"></div>
  </div>
  <div class="lw-style-options" role="radiogroup" aria-label="Writing style" id="style_options"></div>
  <section class="lw-section lw-style-apps" id="style_apps">
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

const BUCKETS = ["personal", "work", "email", "other"] as const;
type Bucket = (typeof BUCKETS)[number];
const BUCKET_NAMES: string[] = [...BUCKETS];

const BUCKET_LABELS: Record<string, string> = {
  personal: "Personal",
  work: "Work",
  email: "Email",
  other: "Other",
};

/** Tab label and banner line per group (PageStyle artboard). */
const TABS: Record<Bucket, { label: string; banner: string }> = {
  personal: { label: "Personal chats", banner: "Used when you dictate into chat apps for friends and family" },
  work: { label: "Work chats", banner: "Used when you dictate into team chat apps" },
  email: { label: "Email", banner: "Used when you dictate into email" },
  other: { label: "Other", banner: "Used in every other app" },
};

interface StyleOption {
  id: string;
  title: string;
  sub: string;
  sample: string;
}
/** The four backend styles (flow-core `Style`). Casual drops optional
 *  commas and very casual lowercases only with AI cleanup on; without it
 *  both just drop the final period. */
const OPTIONS: Record<string, StyleOption> = {
  formal: {
    id: "formal",
    title: "Formal.",
    sub: "Capitals, full punctuation",
    sample: "Hey, are you free for lunch tomorrow? Let's do 12 if that works for you.",
  },
  casual: {
    id: "casual",
    title: "Casual",
    sub: "Capitals, lighter punctuation",
    sample: "Hey are you free for lunch tomorrow? Let's do 12 if that works for you",
  },
  very_casual: {
    id: "very_casual",
    title: "very casual",
    sub: "Lowercase, lighter punctuation",
    sample: "hey are you free for lunch tomorrow? let's do 12 if that works for you",
  },
  excited: {
    id: "excited",
    title: "Excited!",
    sub: "More exclamations",
    sample: "Hey, are you free for lunch tomorrow? Let's do 12 if that works for you!",
  },
};
/** Chats offer very casual, email and the rest offer excited (as in the
 *  design); a stored style outside a group's three still shows as a card. */
const GROUP_OPTIONS: Record<Bucket, string[]> = {
  personal: ["formal", "casual", "very_casual"],
  work: ["formal", "casual", "very_casual"],
  email: ["formal", "casual", "excited"],
  other: ["formal", "casual", "excited"],
};
const TONES = ["lilac", "pink", "maroon", "maroon"];
/** Banner chips before "+N more". */
const MAX_CHIPS = 6;

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

/** The group whose tab is open, and each group's style. */
let tab: Bucket = "personal";
const picked: Record<Bucket, string> = { personal: "formal", work: "formal", email: "formal", other: "formal" };

/** Prefs spell styles "Very Casual" / "very_casual" / "veryCasual". */
function styleId(v: string | undefined): string {
  const k = (v ?? "").replace(/([a-z])([A-Z])/g, "$1_$2").toLowerCase().replace(/[ -]/g, "_");
  return k in OPTIONS ? k : "formal";
}

function renderTabs(): void {
  $<HTMLElement>("style_tabs").replaceChildren(
    ...BUCKETS.map((b) => {
      const on = b === tab;
      const t = button(TABS[b].label, () => pickTab(b), on ? "fl-tab is-active" : "fl-tab");
      t.id = `style_tab_${b}`;
      t.setAttribute("role", "tab");
      t.setAttribute("aria-selected", String(on));
      t.setAttribute("aria-controls", "style_panel");
      t.tabIndex = on ? 0 : -1;
      t.dataset.bucket = b;
      return t;
    }),
  );
  $<HTMLElement>("style_panel").setAttribute("aria-labelledby", `style_tab_${tab}`);
}

function pickTab(b: Bucket, focus = false): void {
  tab = b;
  renderTabs();
  renderBanner();
  renderOptions();
  if (focus) $<HTMLElement>(`style_tab_${b}`).focus();
}

/** Apps that land in `b` now: your rows, then built-ins you didn't move. */
function appsIn(b: Bucket): string[] {
  const names: string[] = [];
  const push = (n: string) => {
    if (!names.includes(n)) names.push(n);
  };
  for (const r of appRows) if (r.bucket === b) push(choiceFor(r.key)?.name ?? r.key);
  for (const c of styleChoices.defaults)
    if (c.default_bucket === b && !hasAppRow(c.key)) push(c.name ?? c.exe);
  return names;
}

function renderBanner(): void {
  $<HTMLElement>("style_banner_title").textContent = TABS[tab].banner;
  const names = appsIn(tab);
  const chips: HTMLElement[] = names.slice(0, MAX_CHIPS).map((n) => el("span", "lw-style-chip", n));
  if (names.length > MAX_CHIPS) {
    const more = el("span", "lw-style-chip", `+${names.length - MAX_CHIPS} more`);
    more.title = names.slice(MAX_CHIPS).join(", ");
    chips.push(more);
  }
  if (names.length === 0)
    chips.push(el("span", "lw-style-chip is-muted", tab === "other" ? "Every other app" : "No apps yet"));
  const add = button("", () => {
    $<HTMLElement>("style_apps").scrollIntoView({ behavior: "smooth", block: "start" });
    const recent = $<HTMLSelectElement>("app_recent");
    (recent.disabled ? $<HTMLInputElement>("app_manual") : recent).focus({ preventScroll: true });
  }, "lw-style-chip-add");
  add.setAttribute("aria-label", "Add an app to a group");
  add.title = "Add an app";
  add.append(iconEl(ICON.plus, 14));
  chips.push(add);
  $<HTMLElement>("style_chips").replaceChildren(...chips);
}

function renderOptions(): void {
  const cur = picked[tab];
  const ids = GROUP_OPTIONS[tab].includes(cur) ? GROUP_OPTIONS[tab] : [...GROUP_OPTIONS[tab], cur];
  const box = $<HTMLElement>("style_options");
  box.style.setProperty("--lw-style-cols", String(ids.length));
  box.replaceChildren(
    ...ids.map((id, i) => {
      const o = OPTIONS[id];
      const on = id === cur;
      const card = el("button", on ? "fl-option is-selected" : "fl-option");
      card.type = "button";
      card.setAttribute("role", "radio");
      card.setAttribute("aria-checked", String(on));
      card.tabIndex = on ? 0 : -1;
      card.dataset.style = id;
      const avatar = el("span", `fl-avatar fl-avatar-lg fl-avatar-${TONES[i]}`, "J");
      avatar.setAttribute("aria-hidden", "true");
      const wrap = el("span", "fl-bubble-wrap");
      wrap.append(el("span", "fl-bubble", o.sample), avatar);
      card.append(el("span", "fl-option-title", o.title), el("span", "fl-option-sub", o.sub), wrap);
      card.addEventListener("click", () => pickStyle(id));
      return card;
    }),
  );
}

function pickStyle(id: string, focus = false): void {
  if (picked[tab] === id) return;
  picked[tab] = id;
  renderOptions();
  if (focus) $<HTMLElement>("style_options").querySelector<HTMLElement>(`[data-style="${id}"]`)?.focus();
  void saveStyle();
}

/** Arrow keys move along the tabs / style cards (ARIA tablist, radiogroup). */
function arrowStep(e: KeyboardEvent): number {
  if (e.key === "ArrowRight" || e.key === "ArrowDown") return 1;
  if (e.key === "ArrowLeft" || e.key === "ArrowUp") return -1;
  return 0;
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
          renderBanner();
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
  renderBanner();
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
  for (const b of BUCKETS) picked[b] = styleId(p.styles?.[b]);
  renderOptions();
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
    styles: { ...picked },
    app_buckets,
  };
}

export const stylePage: Page = {
  id: "style",
  area: "main",
  template: TEMPLATE,
  init(): void {
    pickTab(tab);
    $<HTMLElement>("style_tabs").addEventListener("keydown", (e) => {
      const d = arrowStep(e);
      if (!d) return;
      e.preventDefault();
      pickTab(BUCKETS[(BUCKETS.indexOf(tab) + d + BUCKETS.length) % BUCKETS.length], true);
    });
    $<HTMLElement>("style_options").addEventListener("keydown", (e) => {
      const d = arrowStep(e);
      if (!d) return;
      e.preventDefault();
      const ids = [...$<HTMLElement>("style_options").querySelectorAll<HTMLElement>("[data-style]")].map(
        (c) => c.dataset.style ?? "",
      );
      pickStyle(ids[(ids.indexOf(picked[tab]) + d + ids.length) % ids.length], true);
    });
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
