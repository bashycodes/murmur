// Insights words (UX review on 1.3.0): the heatmap legend reads Less → More,
// "Longest" shows only when it beats the current streak, and backend ISO days
// use the app's date format, and every words-bar part is named in its key.
// Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { dayText, legendHtml, LEGEND_LEVELS, splitKey, streakText } from "../dist/lib/insights-text.js";

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

test("impossible days pass through unformatted", () => {
  for (const d of ["2026-13-01", "2026-00-10", "2026-02-30", "2026-11-00", "2026-04-31"]) {
    assert.equal(dayText(d, "en-US"), d);
  }
  assert.equal(dayText("2028-02-29", "en-US"), "Feb 29, 2028");
});

test("the words-bar key names every used route with its share, in bar order", () => {
  assert.deepEqual(splitKey({ pc: 6, server: 3, cloud: 1, unknown: 5 }), [
    { route: "pc", text: "This PC 60%" },
    { route: "server", text: "Your server 30%" },
    { route: "cloud", text: "Cloud 10%" },
  ]);
  assert.deepEqual(splitKey({ pc: 0, server: 1, cloud: 0, unknown: 0 }), [{ route: "server", text: "Your server 100%" }]);
  assert.deepEqual(splitKey({ pc: 0, server: 0, cloud: 0, unknown: 4 }), []);
});
