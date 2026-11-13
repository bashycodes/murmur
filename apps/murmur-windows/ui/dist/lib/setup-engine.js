// Setup's "Where should your voice become text?" step: which card carries
// "Recommended" and which one starts selected (D73, amended by the 1.3.0 UX
// review: step 1 promises "Runs on this PC", so a new install is pointed at
// it). Pure, so `npm test` runs it on the built dist without a page.
/** Each engine in the app's words (setup, Insights, the re-run line). */
export const ENGINE_LABEL = { pc: "This PC", server: "Your server", cloud: "Cloud" };
/**
 * Setup's one order of preference, limited to the build's engines: a saved
 * server, then this PC, then a server still to be set up, then the cloud.
 */
function ranked(f) {
    const order = f.serverSaved ? ["server", "pc", "cloud"] : ["pc", "server", "cloud"];
    return order.filter((e) => f.has(e));
}
/**
 * The engine setup recommends: the top of that order when it is this PC or
 * a saved server. A server nobody set up yet, or the cloud, is never
 * recommended.
 */
export function recommendedEngine(f) {
    const top = ranked(f)[0];
    return top === "pc" || (top === "server" && f.serverSaved) ? top : null;
}
/**
 * The card selected when the step opens. `first` is the first engine the saved
 * settings would use (null: none set up). A server or OpenRouter already in use
 * stays picked, and so does this PC once setup has been finished (re-run from
 * Settings). On a clean install this PC is on by default but was never chosen,
 * so the top of the order is picked.
 */
export function initialEngine(first, onboarded, f) {
    if (first === "server" || first === "cloud")
        return first;
    if (first === "pc" && onboarded)
        return "pc";
    return ranked(f)[0] ?? "cloud";
}
