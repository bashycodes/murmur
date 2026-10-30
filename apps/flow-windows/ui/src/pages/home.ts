// Home (Dictation), after Main.dc.html / DarkHome.dc.html: the greeting with
// the push-to-talk keys, where speech goes, the local history (copy,
// re-paste, delete, undo AI edit = use the raw ASR text instead of the
// cleaned one; a pill per row for where speech ran) and the stats rail
// (Insights counts, where speech ran this week, OpenRouter spend).
// Retention lives in Settings → Data and privacy.

import { $, el, setStatus } from "../lib/dom.js";
import { money } from "../lib/format.js";
import { ICON, iconEl } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { prefsStore } from "../lib/prefs-store.js";
import { router, type Page } from "../lib/router.js";
import { views } from "../lib/views.js";
import { ROUTE_LABEL, ROUTES, routeShare, type InsightsView, type Route } from "./insights.js";

interface HistoryItem {
  id: number;
  ts: number;
  app: string;
  url: string | null;
  raw: string;
  formatted: string | null;
  pasted: string | null;
  edited: string | null;
  duration_ms: number;
  words: number;
  lang: string | null;
  final_text: string;
  ai_edited: boolean;
  /** Where speech was turned into text; null = unknown (older rows). */
  route: Route | null;
}

const TEMPLATE = `
<div class="lw-home">
  <section class="lw-home-feed">
    <div class="lw-home-head">
      <h1 class="fl-page-title">Welcome back</h1>
      <div class="fl-kbd-group"><span id="home_ptt"></span><span class="fl-kbd-text">Hold to dictate in any app</span></div>
    </div>

    <div class="fl-callout fl-callout-lilac lw-callout">
      <div class="fl-grow">
        <div class="fl-callout-title" id="privacy_title">Your voice stays on this PC</div>
        <div class="fl-callout-body" id="privacy_line">Everything runs on this PC. Nothing is sent over the network.</div>
      </div>
      <button type="button" class="lw-link" id="privacy_review">Review →</button>
    </div>

    <div class="lw-search lw-home-search">
      <input id="hist_query" class="fl-input" type="search" placeholder="Search your dictations" spellcheck="false" aria-label="Search your dictations" />
      <span id="hist_status" class="lw-status"></span>
    </div>
    <div id="hist_list"></div>
    <div id="hist_empty" class="fl-empty" hidden>
      <div class="fl-empty-title">Nothing here yet</div>
      <div class="fl-empty-body" id="hist_empty_body">Hold your push-to-talk keys in any app and speak. Your dictations show up here.</div>
    </div>
  </section>

  <aside class="fl-stats lw-home-stats">
    <div class="fl-stats-sec">
      <div class="fl-stat"><span class="fl-stat-value" id="home_words">0</span><span class="fl-stat-unit">total words</span></div>
      <div class="fl-stat"><span class="fl-stat-value" id="home_wpm">–</span><span class="fl-stat-unit">wpm</span></div>
      <div class="fl-stat"><span class="fl-stat-value" id="home_streak">0</span><span class="fl-stat-unit">day streak</span></div>
    </div>
    <div class="fl-stats-sec lw-home-routes">
      <span class="fl-overline">Where speech ran this week</span>
      <div class="lw-home-routebar" id="home_routebar"></div>
      <div class="lw-home-routelist" id="home_routes"></div>
    </div>
    <div class="fl-stats-sec" id="home_quota_sec" hidden>
      <div class="fl-quota" id="home_quota"></div>
      <p class="fl-help" id="home_quota_help">OpenRouter spend this month.</p>
      <button type="button" class="fl-btn fl-btn-outline fl-btn-block" id="home_usage">View usage</button>
    </div>
  </aside>
</div>`;

/** Word-level LCS diff: [kind, word] with kind "=", "-" (raw only), "+" (final only). */
function wordDiff(a: string, b: string): [string, string][] {
  const x = a.split(/\s+/).filter(Boolean);
  const y = b.split(/\s+/).filter(Boolean);
  const n = x.length;
  const m = y.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: [string, string][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) {
      out.push(["=", x[i]]);
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) out.push(["-", x[i++]]);
    else out.push(["+", y[j++]]);
  }
  while (i < n) out.push(["-", x[i++]]);
  while (j < m) out.push(["+", y[j++]]);
  return out;
}

function diffView(raw: string, final: string): HTMLElement {
  const p = el("p", "diff");
  for (const [kind, w] of wordDiff(raw, final)) {
    const tag = kind === "-" ? "del" : kind === "+" ? "ins" : "span";
    p.append(el(tag, undefined, w), " ");
  }
  return p;
}

async function act(what: Promise<unknown>, okText: string): Promise<void> {
  try {
    await what;
    setStatus("hist_status", okText, true);
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
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

function timeText(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).replace(" ", "").toLowerCase();
}

function dayText(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString([], {
    weekday: "long",
    month: "long",
    day: "numeric",
    year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
}

function historyRow(h: HistoryItem): HTMLElement {
  const row = el("div", "fl-list-row lw-hist-row");
  const body = el("div", "lw-hist-body");
  body.append(el("p", "lw-hist-text", h.final_text));
  // Only the exceptions get a badge; "AI edited" shows as the undo button.
  if (h.edited || h.pasted === null) {
    const meta = el("div", "lw-hist-meta");
    if (h.edited) meta.append(el("span", "fl-badge fl-badge-light", "You corrected it"));
    if (h.pasted === null) meta.append(el("span", "fl-badge fl-badge-destructive", "Not pasted"));
    body.append(meta);
  }
  if (h.edited) {
    const c = el("p", "lw-hist-note");
    c.append("Your correction: ", el("q", undefined, h.edited));
    body.append(c);
  }

  const actions = el("div", "lw-hist-actions");
  actions.append(
    iconButton("Copy", ICON.copy, () => void act(invoke("history_copy", { id: h.id, raw: false }), "Copied")),
    iconButton("Paste again", ICON.paste, () => void act(invoke("history_repaste", { id: h.id, raw: false }), "Pasted")),
  );
  if (h.ai_edited) {
    const undo = el("div", "lw-undo");
    undo.hidden = true;
    const again = el("button", "fl-btn fl-btn-secondary fl-btn-sm", "Paste what you said");
    again.type = "button";
    again.addEventListener("click", () =>
      void act(invoke("history_repaste", { id: h.id, raw: true }), "Pasted the original"),
    );
    const copy = el("button", "fl-btn fl-btn-ghost fl-btn-sm", "Copy what you said");
    copy.type = "button";
    copy.addEventListener("click", () => void act(invoke("history_copy", { id: h.id, raw: true }), "Copied the original"));
    const btns = el("div", "lw-undo-actions");
    btns.append(again, copy);
    undo.append(
      el("p", "lw-hist-note", "What you said (struck = removed by AI, underlined = added):"),
      diffView(h.raw, h.final_text),
      btns,
    );
    body.append(undo);
    actions.append(
      iconButton("Undo AI edit…", ICON.undo, () => {
        undo.hidden = !undo.hidden;
      }),
    );
  }
  actions.append(
    iconButton("Delete", ICON.trash, () =>
      void (async () => {
        try {
          await invoke("history_delete", { id: h.id });
          row.remove();
        } catch (e) {
          setStatus("hist_status", String(e), false);
        }
      })(),
    ),
  );
  const time = el("span", "fl-list-time", timeText(h.ts));
  const app = h.app.replace(/\.exe$/i, "");
  time.title = `${app ? `${app}, ` : ""}${h.words} ${h.words === 1 ? "word" : "words"}`;
  row.append(time, body, actions);
  if (h.route) row.append(el("span", `lw-route is-${h.route}`, ROUTE_LABEL[h.route]));
  return row;
}

/** Rows grouped by day, newest first (the backend's order). */
function render(rows: HistoryItem[]): void {
  const out: HTMLElement[] = [];
  let day = "";
  let list: HTMLElement | null = null;
  for (const h of rows) {
    const d = dayText(h.ts);
    if (d !== day || !list) {
      day = d;
      const head = el("div", "fl-list-date");
      head.append(el("span", "fl-overline", d));
      list = el("div", "fl-list");
      out.push(head, list);
    }
    list.append(historyRow(h));
  }
  $<HTMLElement>("hist_list").replaceChildren(...out);
  $<HTMLElement>("hist_empty").hidden = rows.length > 0;
  const searching = $<HTMLInputElement>("hist_query").value.trim() !== "";
  const keep = prefsStore.get()?.retention;
  $<HTMLElement>("hist_empty_body").textContent = searching
    ? "No dictation matches your search."
    : keep === "never"
      ? "History is off (Settings → Data and privacy), so nothing is kept."
      : "Hold your push-to-talk keys in any app and speak. Your dictations show up here.";
}

let histSeq = 0;
/** Reload the list (search, retention change, delete all). */
export async function reloadHistory(): Promise<void> {
  const seq = ++histSeq;
  try {
    const rows = await invoke<HistoryItem[]>("history_list", {
      query: $<HTMLInputElement>("hist_query").value,
      limit: 200,
    });
    if (seq !== histSeq) return; // a newer search is in flight
    render(rows);
  } catch (e) {
    setStatus("hist_status", String(e), false);
  }
}

// ------------------------------------------------------------ header

function showKeys(): void {
  const keys = (prefsStore.get()?.shortcuts.ptt ?? "").split("+").map((k) => k.trim()).filter(Boolean);
  $<HTMLElement>("home_ptt").replaceChildren(...keys.map((k) => el("kbd", "fl-kbd", k)));
}

/** Where speech and text can go, from the engine and OpenRouter switches. */
function showPrivacy(): void {
  const or = views.or;
  const dv = views.dev;
  const audioOut = !!or?.supported && or.stt.allow;
  const textOut = !!or?.supported && or.cleanup.allow;
  const server = !!dv?.caps.remote && !!dv.form.allow_server;
  $<HTMLElement>("privacy_title").textContent = audioOut
    ? "Your voice can go to OpenRouter"
    : server
      ? "Your voice stays on your devices"
      : "Your voice stays on this PC";
  let line: string;
  if (audioOut) {
    line = `Speech runs on your own engines and on OpenRouter (${or?.stt.model ?? ""}), in the order you set.${textOut ? " Text for AI cleanup goes there too." : ""}`;
  } else if (textOut) {
    line = `Speech runs on ${server ? "this PC or your own server" : "this PC"}. OpenRouter only gets text for AI cleanup.`;
  } else if (server) {
    line = "Speech runs on this PC or your own server. Nothing goes to a third party.";
  } else {
    line = "Everything runs on this PC. Nothing is sent over the network.";
  }
  $<HTMLElement>("privacy_line").textContent = line;
}

function compact(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, "")}K`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

async function loadStats(): Promise<void> {
  try {
    const s = await invoke<InsightsView>("insights_get");
    $<HTMLElement>("home_words").textContent = compact(s.total_words);
    $<HTMLElement>("home_wpm").textContent = s.avg_wpm === null ? "–" : String(Math.round(s.avg_wpm));
    $<HTMLElement>("home_streak").textContent = String(s.current_streak);
    lastRoutes = s.routes_7d;
    showRoutes();
  } catch {
    /* the stats rail keeps its last numbers */
  }
}

let lastRoutes: InsightsView["routes_7d"] | null = null;

/** The rail's split bar + legend; Cloud reads "Off" while OpenRouter speech is off and unused. */
function showRoutes(): void {
  if (!lastRoutes) return;
  const share = routeShare(lastRoutes);
  const bar = $<HTMLElement>("home_routebar");
  bar.replaceChildren(
    ...(share
      ? ROUTES.filter((r) => lastRoutes![r] > 0).map((r) => {
          const seg = el("span", `is-${r}`);
          seg.style.flex = String(Math.max(share[r], 2));
          seg.title = `${ROUTE_LABEL[r]}: ${share[r]}%`;
          return seg;
        })
      : [el("span", "is-none")]),
  );
  const cloudOff = !(views.or?.supported && views.or.stt.allow);
  $<HTMLElement>("home_routes").replaceChildren(
    ...ROUTES.map((r) => {
      const line = el("span", "lw-home-route");
      const off = r === "cloud" && cloudOff && lastRoutes!.cloud === 0;
      const dot = el("span", `lw-dot is-${off ? "off" : r}`);
      line.append(dot, el("span", "lw-grow", ROUTE_LABEL[r]), el("span", undefined, off ? "Off" : share ? `${share[r]}%` : "–"));
      return line;
    }),
  );
}

function showQuota(): void {
  const or = views.or;
  $<HTMLElement>("home_quota_sec").hidden = !or?.supported;
  if (!or?.supported) return;
  const u = or.usage;
  const q = $<HTMLElement>("home_quota");
  const spent = el("em", undefined, money(u.total));
  q.replaceChildren(spent, u.budget != null ? ` of $${u.budget.toFixed(2)}` : " this month");
  $<HTMLElement>("home_quota_help").textContent = u.budget_reached
    ? "Budget reached: dictation stays on your own engines until next month."
    : u.budget != null
      ? "OpenRouter spend this month. Dictation switches to your own engines when the budget runs out."
      : "OpenRouter spend this month. No budget set.";
}

export const homePage: Page = {
  id: "home",
  area: "main",
  template: TEMPLATE,
  init(): void {
    let searchTimer = 0;
    $<HTMLInputElement>("hist_query").addEventListener("input", () => {
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => void reloadHistory(), 200);
    });
    $<HTMLButtonElement>("privacy_review").addEventListener("click", () => router.go("settings-privacy"));
    $<HTMLButtonElement>("home_usage").addEventListener("click", () => router.go("settings-openrouter"));
    prefsStore.subscribe(showKeys);
    views.onDev(showPrivacy);
    views.onOr(() => {
      showPrivacy();
      showQuota();
      showRoutes();
    });
    void onEvent<null>("history:changed", () => {
      if (router.active() === "home") {
        void reloadHistory();
        void loadStats();
      }
    });
    void onEvent<null>("insights:changed", () => {
      if (router.active() === "home") void loadStats();
    });
  },
  show(): void {
    void reloadHistory();
    void loadStats();
  },
};
