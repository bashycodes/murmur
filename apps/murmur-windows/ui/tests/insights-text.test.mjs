// Insights words (UX review on 1.3.0): the heatmap legend reads Less → More,
// "Longest" shows only when it beats the current streak, and backend ISO days
// use the app's date format. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { dayText, legendHtml, LEGEND_LEVELS, streakText } from "../dist/lib/insights-text.js";

test("legend reads Less, lightest to darkest, More", () => {
  const html = legendHtml();
  assert.ok(html.startsWith("Less"), html);
  assert.ok(html.endsWith("More"), html);
  const levels = [...html.matchAll(/class="(h\d)"/g)].map((m) => m[1]);
  assert.deepEqual(levels, ["h1", "h2", "h3", "h4"]);
  assert.deepEqual([...LEGEND_LEVELS], levels);
});

test("longest streak shows only when longer than the current one", () => {
  assert.deepEqual(streakText(1, 1), { streak: "1 day streak", longest: null });
  assert.deepEqual(streakText(0, 0), { streak: "0 day streak", longest: null });
  assert.deepEqual(streakText(2, 5), { streak: "2 day streak", longest: "Longest 5 days" });
  assert.deepEqual(streakText(0, 1), { streak: "0 day streak", longest: "Longest 1 day" });
});

test("ISO days use the app's medium date format, on the local day", () => {
  const want = new Date(2026, 10, 3).toLocaleDateString("en-US", { dateStyle: "medium" });
  assert.equal(dayText("2026-11-03", "en-US"), want);
  assert.equal(dayText("2026-11-03", "en-US"), "Nov 3, 2026");
  assert.equal(dayText("2026-01-01", "en-US"), "Jan 1, 2026");
  assert.ok(!dayText("2026-11-03").includes("2026-11-03"));
});

test("anything that isn't an ISO day passes through", () => {
  assert.equal(dayText(""), "");
  assert.equal(dayText("yesterday"), "yesterday");
  assert.equal(dayText("2026-11-03T10:00:00Z"), "2026-11-03T10:00:00Z");
});
