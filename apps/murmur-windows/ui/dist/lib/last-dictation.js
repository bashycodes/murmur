// Settings → Developer's "Last dictation" words that need no DOM: the live
// engine word, and the newest history dictation shown before this run of the
// app has dictated. Tested in tests/last-dictation.test.mjs.
/** The live route's engine (`remote` / `local` / `cloud`) as Developer says it. */
export function engineWord(engine) {
    return engine === "remote" ? "server" : engine === "local" ? "client" : (engine ?? "none");
}
/** History's route (`history::route_label`) back to its engine, so both paths use one word. */
const ROUTE_ENGINE = { pc: "local", server: "remote", cloud: "cloud" };
/** How many newest rows to read when looking for a plain dictation. */
export const HISTORY_PEEK = 10;
/** The newest plain dictation (no Command Mode or transform row); rows are newest first. */
export function pickDictation(rows) {
    return rows.find((r) => !r.command && !r.transform) ?? null;
}
export const NO_DICTATION = "No dictation yet";
/** "Nov 3, 2026, 2:14 PM · Notepad · 12 words · server · timings show after the next dictation". */
export function historyLastText(h, locales) {
    if (!h)
        return NO_DICTATION;
    return [
        new Date(h.ts).toLocaleString(locales ?? [], { dateStyle: "medium", timeStyle: "short" }),
        h.app.replace(/\.exe$/i, "") || "unknown app",
        `${h.words.toLocaleString(locales ?? [])} ${h.words === 1 ? "word" : "words"}`,
        h.route ? engineWord(ROUTE_ENGINE[h.route] ?? h.route) : "",
        "timings show after the next dictation",
    ]
        .filter(Boolean)
        .join(" · ");
}
/** Lets only the newest of overlapping async updates write: `next()` on each
 *  update, `isLatest(t)` before writing its result. */
export function latestOnly() {
    let seq = 0;
    return { next: () => ++seq, isLatest: (t) => t === seq };
}
