// Insights page, after PageInsights.dc.html: speaking speed (gauge against
// typing), fixes made, total words with where speech ran this week, top apps
// and the streak heatmap. Everything is counted on this PC (`insights_get` /
// `insights_reset`; kept apart from History, never sent anywhere).

import { $, askDialog, el, setStatus } from "../lib/dom.js";
import { ICON, iconEl } from "../lib/icons.js";
import { invoke, onEvent } from "../lib/ipc.js";
import { router, type Page } from "../lib/router.js";

/** `flow_core::insights::Summary` (counts only; computed on this PC). */
export interface InsightsSummary {
  total_words: number;
  week_words: number;
  dictations: number;
  total_ms: number;
  avg_wpm: number | null;
  current_streak: number;
  longest_streak: number;
  typing_wpm: number;
  saved_minutes: number;
  top_apps: { app: string; words: number }[];
  last7: { date: string; words: number }[];
  since: string | null;
}

/** Where speech was turned into text (`history::route_label`). */
export type Route = "pc" | "server" | "cloud";

/** What `insights_get` returns: the summary plus the desktop extras. */
export interface InsightsView extends InsightsSummary {
  /** 18 Monday-first weeks × 7 days, oldest first; `words: null` = after today. */
  heatmap: { date: string; words: number | null }[];
  /** Dictations per route, today and the 6 days before. */
  routes_7d: Record<Route | "unknown", number>;
  fixes: { ai: number; dictionary: number; total: number };
}

export const ROUTES: Route[] = ["pc", "server", "cloud"];
export const ROUTE_LABEL: Record<Route, string> = { pc: "This PC", server: "Your server", cloud: "Cloud" };

/** Percent per known route (unknown left out); null when none is known. */
export function routeShare(r: InsightsView["routes_7d"]): Record<Route, number> | null {
  const known = r.pc + r.server + r.cloud;
  if (known === 0) return null;
  const pct = (n: number): number => Math.round((100 * n) / known);
  return { pc: pct(r.pc), server: pct(r.server), cloud: pct(r.cloud) };
}

const TEMPLATE = `
<div class="lw-ins">
  <div class="lw-ins-head">
    <h1 class="fl-page-title">Insights</h1>
    <span class="lw-ins-sub">Worked out on this PC from your history</span>
    <button type="button" id="ins_reset" class="fl-btn fl-btn-ghost fl-btn-sm lw-danger">Reset stats</button>
  </div>

  <div class="lw-ins-row3">
    <div class="fl-card lw-ins-card">
      <div class="fl-metric" id="ins_wpm">–</div>
      <div class="fl-metric-cap">Words per minute</div>
      <div class="lw-ins-gauge">
        <svg viewBox="0 0 120 64" width="120" height="64" role="img" id="ins_gauge">
          <path d="M10 60a50 50 0 0 1 100 0" pathLength="100" fill="none" stroke="var(--background-secondary)" stroke-width="12" stroke-linecap="round"></path>
          <path id="ins_gauge_arc" d="M10 60a50 50 0 0 1 100 0" pathLength="100" fill="none" stroke="var(--fathom-900)" stroke-width="12" stroke-linecap="round" stroke-dasharray="0 100"></path>
        </svg>
        <span id="ins_vs_typing"></span>
      </div>
    </div>
    <div class="fl-card lw-ins-card">
      <div class="fl-metric" id="ins_fixes">0</div>
      <div class="fl-metric-cap">Corrections applied</div>
      <div class="lw-ins-rule"></div>
      <div class="lw-ins-kv"><span>By AI cleanup</span><span id="ins_fix_ai">0</span></div>
      <div class="lw-ins-kv"><span>From your dictionary</span><span id="ins_fix_dict">0</span></div>
    </div>
    <div class="fl-card lw-ins-card">
      <div class="fl-metric" id="ins_total">0</div>
      <div class="fl-metric-cap">Words dictated so far</div>
      <div class="lw-ins-rule"></div>
      <div class="lw-split" id="ins_split"></div>
      <span class="lw-ins-note" id="ins_split_note">Where your speech was turned into text this week</span>
    </div>
  </div>

  <div class="lw-ins-row2">
    <div class="fl-card lw-ins-card lw-ins-big">
      <div class="lw-ins-cardhead"><span class="lw-ins-title">Where you dictate</span><span class="fl-metric-cap">Top apps by words</span></div>
      <div class="fl-usage" id="ins_apps"></div>
      <p id="ins_apps_empty" class="lw-ins-note">No dictations yet.</p>
    </div>
    <div class="fl-card lw-ins-card lw-ins-big">
      <div class="lw-ins-cardhead"><span class="lw-ins-title" id="ins_streak">0 day streak</span><span class="fl-metric-cap" id="ins_longest">Longest 0 days</span></div>
      <div>
        <div class="fl-heat" id="ins_heat" role="img" aria-label="Words per day, last 18 weeks"></div>
        <div class="fl-heat-legend">More<i class="h4"></i><i class="h3"></i><i class="h2"></i><i class="h1"></i>Less</div>
      </div>
    </div>
  </div>

  <p class="fl-help lw-ins-foot"><span id="ins_assume"></span> Counted on this PC only (words, recording length, app name and where speech ran, per day; no text). Nothing is sent anywhere. Deleting history keeps these numbers. <span id="ins_status"></span></p>

  <dialog id="ins_reset_dlg" class="fl-dialog">
    <h3>Reset all stats?</h3>
    <p>Words, streaks and app counts go back to zero. History is not affected and won't be counted again.</p>
    <div class="lw-dialog-actions">
      <button type="button" class="fl-btn fl-btn-secondary" id="ins_reset_cancel">Cancel</button>
      <button type="button" class="fl-btn fl-btn-destructive" id="ins_reset_ok">Reset stats</button>
    </div>
  </dialog>
</div>`;

export function minutesText(m: number): string {
  if (m < 1) return `${Math.round(m * 60)} s`;
  if (m < 60) return `${Math.round(m)} min`;
  return `${(m / 60).toFixed(1)} h`;
}

function days(n: number): string {
  return n === 1 ? "1 day" : `${n} days`;
}

/** Speed against typing: the gauge fills at 4.5× typing speed. */
function showSpeed(s: InsightsView): void {
  const arc = $<HTMLElement>("ins_gauge_arc");
  const vs = $<HTMLElement>("ins_vs_typing");
  if (s.avg_wpm === null || s.typing_wpm <= 0) {
    $<HTMLElement>("ins_wpm").textContent = "–";
    arc.setAttribute("stroke-dasharray", "0 100");
    vs.textContent = "Dictate a few times to see your speed";
  } else {
    const ratio = s.avg_wpm / s.typing_wpm;
    $<HTMLElement>("ins_wpm").textContent = String(Math.round(s.avg_wpm));
    arc.setAttribute("stroke-dasharray", `${Math.min(100, (100 * ratio) / 4.5).toFixed(1)} 100`);
    const x = ratio < 2 ? ratio.toFixed(1).replace(/\.0$/, "") : String(Math.round(ratio));
    vs.textContent = `About ${x}× your typing speed`;
  }
  $<HTMLElement>("ins_gauge").setAttribute("aria-label", vs.textContent ?? "");
}

/** Split bar of where speech ran this week (known routes only). */
function showSplit(s: InsightsView): void {
  const share = routeShare(s.routes_7d);
  const bar = $<HTMLElement>("ins_split");
  bar.hidden = share === null;
  $<HTMLElement>("ins_split_note").textContent = share
    ? "Where your speech was turned into text this week"
    : "No dictations with a known engine this week";
  if (!share) return;
  bar.replaceChildren(
    ...ROUTES.filter((r) => s.routes_7d[r] > 0).map((r) => {
      const seg = el("span", `is-${r}`, share[r] >= 18 ? ROUTE_LABEL[r] : "");
      seg.style.flex = String(Math.max(share[r], 3));
      seg.title = `${ROUTE_LABEL[r]}: ${share[r]}%`;
      return seg;
    }),
  );
}

function showApps(s: InsightsView): void {
  const total = Math.max(1, s.total_words);
  const pcts = s.top_apps.map((a) => Math.round((100 * a.words) / total));
  const max = Math.max(1, ...pcts);
  const cells: HTMLElement[] = [];
  s.top_apps.forEach((a, i) => {
    const icon = el("span", "lw-ins-appicon");
    icon.append(iconEl(ICON.system, 20));
    const track = el("span", "fl-usage-track");
    const bar = el("span", `fl-usage-bar ${i === 0 ? "is-top" : "is-mid"}`, `${pcts[i]}%`);
    bar.style.width = `${Math.max(30, Math.round((pcts[i] / max) * 180))}px`;
    const name = (a.app || "Unknown app").replace(/\.exe$/i, "");
    track.append(bar, el("span", "fl-usage-label", name));
    track.title = `${name}: ${a.words.toLocaleString()} words`;
    cells.push(icon, track);
  });
  $<HTMLElement>("ins_apps").replaceChildren(...cells);
  $<HTMLElement>("ins_apps_empty").hidden = s.top_apps.length > 0;
}

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Heat level 1–4 on a square-root scale against the busiest day, so one
 *  long day doesn't wash out the rest (h0 = no words, "" = future). */
function level(words: number | null, max: number): string {
  if (words === null) return "";
  if (words === 0) return "h0";
  return `h${Math.min(4, Math.max(1, Math.ceil(4 * Math.sqrt(words / max))))}`;
}

/** Rows = weekdays (Mon first), columns = weeks (oldest left). */
function showHeatmap(s: InsightsView): void {
  const weeks: InsightsView["heatmap"][] = [];
  for (let i = 0; i < s.heatmap.length; i += 7) weeks.push(s.heatmap.slice(i, i + 7));
  const max = Math.max(1, ...s.heatmap.map((d) => d.words ?? 0));
  const heat = $<HTMLElement>("ins_heat");
  heat.style.setProperty("--cols", String(weeks.length));
  const cells: HTMLElement[] = [];
  WEEKDAYS.forEach((name, d) => {
    cells.push(el("span", undefined, name));
    for (const w of weeks) {
      const day = w[d];
      const cell = el("i", day ? level(day.words, max) : "");
      if (day && day.words !== null) cell.title = `${day.date}: ${day.words.toLocaleString()} words`;
      cells.push(cell);
    }
  });
  heat.replaceChildren(...cells);
  $<HTMLElement>("ins_streak").textContent = `${s.current_streak} day streak`;
  $<HTMLElement>("ins_longest").textContent = `Longest ${days(s.longest_streak)}`;
}

function showInsights(s: InsightsView): void {
  showSpeed(s);
  $<HTMLElement>("ins_fixes").textContent = s.fixes.total.toLocaleString();
  $<HTMLElement>("ins_fix_ai").textContent = s.fixes.ai.toLocaleString();
  $<HTMLElement>("ins_fix_dict").textContent = s.fixes.dictionary.toLocaleString();
  $<HTMLElement>("ins_total").textContent = s.total_words.toLocaleString();
  showSplit(s);
  showApps(s);
  showHeatmap(s);
  $<HTMLElement>("ins_assume").textContent =
    `${s.dictations.toLocaleString()} dictations; about ${minutesText(s.saved_minutes)} saved against typing at ${s.typing_wpm} words per minute.` +
    (s.since ? ` Counting since ${s.since}.` : "");
}

async function loadInsights(): Promise<void> {
  try {
    showInsights(await invoke<InsightsView>("insights_get"));
  } catch (e) {
    setStatus("ins_status", String(e), false);
  }
}

async function resetInsights(): Promise<void> {
  if (!(await askDialog("ins_reset_dlg", "ins_reset_ok", "ins_reset_cancel"))) return;
  try {
    await invoke("insights_reset");
    setStatus("ins_status", "Stats reset", true);
  } catch (e) {
    setStatus("ins_status", String(e), false);
  }
  await loadInsights();
}

export const insightsPage: Page = {
  id: "insights",
  area: "main",
  template: TEMPLATE,
  init(): void {
    $<HTMLButtonElement>("ins_reset").addEventListener("click", () => void resetInsights());
    void onEvent<null>("insights:changed", () => {
      if (router.active() === "insights") void loadInsights();
    });
  },
  show(): void {
    void loadInsights();
  },
};
