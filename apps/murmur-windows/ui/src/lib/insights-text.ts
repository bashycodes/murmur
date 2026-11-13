// Insights words that need no DOM: route names and shares, the words-bar
// key, the heatmap legend, the streak pair, and backend ISO days in the
// app's date format. Tested in tests/insights-text.test.mjs.

import { ENGINE_LABEL } from "./setup-engine.js";

/** Where speech was turned into text (`history::route_label`). */
export type Route = "pc" | "server" | "cloud";

export const ROUTES: Route[] = ["pc", "server", "cloud"];
export const ROUTE_LABEL: Record<Route, string> = ENGINE_LABEL;

/** Dictations per route (`InsightsView.routes_7d`). */
export type RouteCounts = Record<Route | "unknown", number>;

/** Percent per known route (unknown left out); null when none is known. */
export function routeShare(r: RouteCounts): Record<Route, number> | null {
  const known = r.pc + r.server + r.cloud;
  if (known === 0) return null;
  const pct = (n: number): number => Math.round((100 * n) / known);
  return { pc: pct(r.pc), server: pct(r.server), cloud: pct(r.cloud) };
}

/** The words-bar key: every route used, in bar order, named with its share
 *  ("This PC 62%"); empty when no route is known. */
export function splitKey(r: RouteCounts): { route: Route; text: string }[] {
  const share = routeShare(r);
  if (!share) return [];
  return ROUTES.filter((x) => r[x] > 0).map((x) => ({ route: x, text: `${ROUTE_LABEL[x]} ${share[x]}%` }));
}

/** Heat levels in legend order, lightest first: "Less ■■■■ More". */
export const LEGEND_LEVELS = ["h1", "h2", "h3", "h4"] as const;

/** The heatmap legend's markup, reading Less → More. */
export function legendHtml(): string {
  return `Less${LEGEND_LEVELS.map((h) => `<i class="${h}"></i>`).join("")}More`;
}

function days(n: number): string {
  return n === 1 ? "1 day" : `${n} days`;
}

/** The streak title, and the "Longest" caption only when it says something new. */
export function streakText(current: number, longest: number): { streak: string; longest: string | null } {
  return {
    streak: `${current} day streak`,
    longest: longest > current ? `Longest ${days(longest)}` : null,
  };
}

/** A backend day ("2026-11-03") in the app's date format ("Nov 3, 2026");
 *  read as a local day, so no timezone shifts it. Anything else, including
 *  an impossible day such as "2026-13-01" or "2026-02-30", passes through. */
export function dayText(iso: string, locales?: string | string[]): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (!m) return iso;
  const [y, mo, da] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const d = new Date(y, mo - 1, da);
  if (d.getFullYear() !== y || d.getMonth() !== mo - 1 || d.getDate() !== da) return iso;
  return d.toLocaleDateString(locales ?? [], { dateStyle: "medium" });
}
