// Settings → Developer's "Last dictation" words that need no DOM: the live
// engine word, and the newest history dictation shown before this run of the
// app has dictated. Tested in tests/last-dictation.test.mjs.

/** The live route's engine (`remote` / `local` / `cloud`) as Developer says it. */
export function engineWord(engine: string | null | undefined): string {
  return engine === "remote" ? "server" : engine === "local" ? "client" : (engine ?? "none");
}

/** History's route (`history::route_label`) back to its engine, so both paths use one word. */
const ROUTE_ENGINE: Record<string, string> = { pc: "local", server: "remote", cloud: "cloud" };

/** The fields of a `history_list` row this needs. */
export interface HistoryHead {
  ts: number;
  app: string;
  words: number;
  route: string | null;
  /** A Command Mode or transform-shortcut row. */
  command?: boolean;
  /** The transform's name, when one ran. */
  transform?: string | null;
}

/** How many newest rows to read when looking for a plain dictation. */
export const HISTORY_PEEK = 10;

/** The newest plain dictation (no Command Mode or transform row); rows are newest first. */
export function pickDictation<T extends HistoryHead>(rows: T[]): T | null {
  return rows.find((r) => !r.command && !r.transform) ?? null;
}

export const NO_DICTATION = "No dictation yet";

/** "Nov 3, 2026, 2:14 PM · Notepad · 12 words · server · timings show after the next dictation". */
export function historyLastText(h: HistoryHead | null, locales?: string | string[]): string {
  if (!h) return NO_DICTATION;
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
export function latestOnly(): { next(): number; isLatest(t: number): boolean } {
  let seq = 0;
  return { next: () => ++seq, isLatest: (t) => t === seq };
}
