// Insights page: local usage counts (`insights_get` / `insights_reset`; kept
// apart from History, never sent anywhere) and a 7-day SVG chart.

import { $, askDialog, el, setStatus, svg } from "../lib/dom.js";
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

const TEMPLATE = `
<div class="lw-page">
  <div class="lw-page-head">
    <h1 class="fl-page-title">Insights</h1>
    <button type="button" id="ins_reset" class="fl-btn fl-btn-ghost fl-btn-sm lw-danger">Reset stats</button>
  </div>
  <div class="lw-cards">
    <div class="fl-card"><div class="fl-metric-cap">Words dictated</div><div class="fl-metric" id="ins_total">0</div></div>
    <div class="fl-card"><div class="fl-metric-cap">Last 7 days</div><div class="fl-metric" id="ins_week">0</div></div>
    <div class="fl-card"><div class="fl-metric-cap">Speaking speed</div><div class="fl-metric"><span id="ins_wpm">–</span> <small>wpm</small></div></div>
    <div class="fl-card"><div class="fl-metric-cap">Time saved</div><div class="fl-metric" id="ins_saved">0 min</div></div>
    <div class="fl-card"><div class="fl-metric-cap">Day streak</div><div class="fl-metric" id="ins_streak">0</div></div>
    <div class="fl-card"><div class="fl-metric-cap">Longest streak</div><div class="fl-metric" id="ins_longest">0</div></div>
    <div class="fl-card"><div class="fl-metric-cap">Dictations</div><div class="fl-metric" id="ins_count">0</div></div>
  </div>
  <div class="lw-cards lw-cards-2">
    <div class="fl-card">
      <div class="fl-metric-cap">Words per day</div>
      <svg id="ins_chart" class="chart" viewBox="0 0 350 140" role="img" aria-label="Words per day, last 7 days"></svg>
    </div>
    <div class="fl-card">
      <div class="fl-metric-cap">Top apps</div>
      <ol id="ins_apps" class="lw-plain lw-apps"></ol>
      <p id="ins_apps_empty" class="hint">No dictations yet.</p>
    </div>
  </div>
  <p class="fl-help"><span id="ins_assume"></span> Counted on this PC only (words, recording length and app name per day; no text). Nothing is sent anywhere. Deleting history keeps these numbers. <span id="ins_status"></span></p>

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

/** 7 bars, oldest left; today's bar is highlighted. */
function drawChart(last7: InsightsSummary["last7"]): void {
  const W = 350, H = 140, top = 16, bottom = 22, gap = 12;
  const bw = (W - gap * (last7.length + 1)) / last7.length;
  const max = Math.max(1, ...last7.map((d) => d.words));
  const parts: SVGElement[] = [svg("line", { class: "base", x1: 0, x2: W, y1: H - bottom, y2: H - bottom })];
  last7.forEach((d, i) => {
    const x = gap + i * (bw + gap);
    const h = ((H - top - bottom) * d.words) / max;
    const y = H - bottom - h;
    const [yy, mm, dd] = d.date.split("-").map(Number);
    const day = new Date(yy, mm - 1, dd).toLocaleDateString(undefined, { weekday: "short" });
    const bar = svg("rect", {
      class: i === last7.length - 1 ? "bar today" : "bar",
      x, y, width: bw, height: Math.max(h, d.words > 0 ? 1 : 0), rx: 3,
    });
    bar.appendChild(svg("title", {}, `${d.date}: ${d.words.toLocaleString()} words`));
    parts.push(bar);
    if (d.words > 0) parts.push(svg("text", { x: x + bw / 2, y: y - 4 }, d.words.toLocaleString()));
    parts.push(svg("text", { x: x + bw / 2, y: H - 6 }, day));
  });
  $<HTMLElement>("ins_chart").replaceChildren(...parts);
}

function showInsights(s: InsightsSummary): void {
  $<HTMLElement>("ins_total").textContent = s.total_words.toLocaleString();
  $<HTMLElement>("ins_week").textContent = s.week_words.toLocaleString();
  $<HTMLElement>("ins_wpm").textContent = s.avg_wpm === null ? "–" : String(Math.round(s.avg_wpm));
  $<HTMLElement>("ins_saved").textContent = minutesText(s.saved_minutes);
  $<HTMLElement>("ins_streak").textContent = days(s.current_streak);
  $<HTMLElement>("ins_longest").textContent = days(s.longest_streak);
  $<HTMLElement>("ins_count").textContent = s.dictations.toLocaleString();
  $<HTMLElement>("ins_assume").textContent =
    `Time saved assumes typing at ${s.typing_wpm} words per minute, minus the time spent speaking.` +
    (s.since ? ` Counting since ${s.since}.` : "");
  drawChart(s.last7);
  const max = Math.max(1, ...s.top_apps.map((a) => a.words));
  $<HTMLOListElement>("ins_apps").replaceChildren(
    ...s.top_apps.map((a) => {
      const li = el("li");
      const bar = el("span", "lw-app-bar");
      bar.style.width = `${Math.max(2, (100 * a.words) / max)}%`;
      li.append(el("span", "lw-app-name", a.app || "Unknown app"), el("span", "lw-app-words", a.words.toLocaleString()), bar);
      return li;
    }),
  );
  $<HTMLElement>("ins_apps_empty").hidden = s.top_apps.length > 0;
}

async function loadInsights(): Promise<void> {
  try {
    showInsights(await invoke<InsightsSummary>("insights_get"));
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
