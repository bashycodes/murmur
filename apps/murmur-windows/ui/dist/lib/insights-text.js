// Insights page words that need no DOM: the heatmap legend, the streak
// pair, and backend ISO days in the app's date format. Tested in
// tests/insights-text.test.mjs.
/** Heat levels in legend order, lightest first: "Less ■■■■ More". */
export const LEGEND_LEVELS = ["h1", "h2", "h3", "h4"];
/** The heatmap legend's markup, reading Less → More. */
export function legendHtml() {
    return `Less${LEGEND_LEVELS.map((h) => `<i class="${h}"></i>`).join("")}More`;
}
function days(n) {
    return n === 1 ? "1 day" : `${n} days`;
}
/** The streak title, and the "Longest" caption only when it says something new. */
export function streakText(current, longest) {
    return {
        streak: `${current} day streak`,
        longest: longest > current ? `Longest ${days(longest)}` : null,
    };
}
/** A backend day ("2026-11-03") in the app's date format ("Nov 3, 2026");
 *  read as a local day, so no timezone shifts it. Anything else passes through. */
export function dayText(iso, locales) {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
    if (!m)
        return iso;
    const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
    return d.toLocaleDateString(locales ?? [], { dateStyle: "medium" });
}
