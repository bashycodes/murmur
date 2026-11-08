// Setup's "Now try it" step (D75): the line under the practice box. The
// step used to show something only after a dictation finished, so a press
// that could not start (no speech model, no server) left it silent while
// the reason went to the pill alone. Pure, so `npm test` runs it on the
// built dist without a page.
/** A backend sentence without its final full stop, to put inside ours. */
const bare = (m) => m.trim().replace(/\.+$/, "");
/** Before the user talks: `null` = ready, nothing to say. `downloading`: a
 *  speech model download started in the model step is still running. */
export function readinessLine(r, downloading = false) {
    if (r.ready)
        return null;
    if (downloading) {
        return { text: "The speech model is still downloading. Hold the shortcut once it's done.", bad: false };
    }
    return {
        text: `Dictation can't work yet. ${r.message ?? "No speech engine is set up."} You can finish setup now and fix it later in Settings → Speech to text.`,
        bad: true,
    };
}
/** A failed readiness call (e.g. the app is still starting). */
export function readinessErrorLine(e) {
    return { text: `Couldn't check whether dictation is ready: ${String(e)}`, bad: true };
}
/** While and after the user holds the shortcut. `null` = keep the current line
 *  (resting, or a finished dictation, which `dev:route` / history report). */
export function pillLine(s) {
    switch (s.state) {
        case "active_ptt":
            return { text: "Listening… let go when you're done.", bad: false };
        case "active_locked":
            return { text: "Listening…", bad: false };
        case "processing":
            return { text: "Turning your words into text…", bad: false };
        case "error":
            return {
                text: `That didn't work: ${bare(s.message ?? "unknown error")}. Try again, or check Settings → Speech to text after setup.`,
                bad: true,
            };
        case "notice":
            return { text: `${bare(s.message ?? "Nothing came through")}. Hold the shortcut, speak, then let go.`, bad: true };
        default:
            return null;
    }
}
