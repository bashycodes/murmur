// The server's team dictionary as the pages show it (Dictionary: read-only;
// Users & keys: the admin's editor). Pure (no DOM), so `npm test` runs it
// on the built dist. Words are user data: callers use textContent.
/** The matcher's word key, close enough for the page: case and spacing
 *  ignored (`murmur_core::postprocess::word_key`). */
export const wordKey = (w) => w.trim().replace(/\s+/g, " ").toLowerCase();
/** Team entries in the server's order, marked where a personal (non-snippet)
 *  entry with the same word wins. */
export function teamRows(team, personal) {
    const mine = new Set(personal.filter((e) => !e.isSnippet).map((e) => wordKey(e.word)));
    return team.map((e) => ({
        heard: e.replacement ? e.word : "",
        word: e.replacement || e.word,
        key: e.word,
        overridden: mine.has(wordKey(e.word)),
    }));
}
/** The editor's two boxes as the entry the server stores: with "heard as",
 *  what was heard is the word and the word is its replacement (as in the
 *  personal dictionary). A reason string when the word is missing. */
export function teamEntryFrom(word, heard) {
    const w = word.trim();
    const h = heard.trim();
    if (!w)
        return "Fill in the word.";
    return h ? { word: h, replacement: w } : { word: w, replacement: "" };
}
/** The line under the team list. */
export function teamSummary(team, personal) {
    if (!team.length)
        return "Your server's team dictionary is empty.";
    const n = teamRows(team, personal).filter((r) => r.overridden).length;
    const words = team.length === 1 ? "1 word" : `${team.length} words`;
    if (!n)
        return `${words} from your server.`;
    return `${words} from your server. ${n} also in your dictionary: yours ${n === 1 ? "is" : "are"} used.`;
}
/** The Dictionary page's line under the team list: the backend's reason when
 *  the server's list could not be read (it says whether a saved copy is in
 *  use: none after the server refused the key), else the summary. */
export function teamNote(v, personal) {
    return v.error ? { text: v.error, err: true } : { text: teamSummary(v.entries, personal), err: false };
}
