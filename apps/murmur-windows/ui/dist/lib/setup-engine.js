// Setup's "Where should your voice become text?" step: which card carries
// "Recommended" and which one starts selected (D73). Pure, so `npm test`
// runs it on the built dist without a page.
/** The engine setup recommends: your server when the build has it, else this PC. */
export function recommendedEngine(has) {
    if (has("server"))
        return "server";
    if (has("pc"))
        return "pc";
    return null;
}
/**
 * The card selected when the step opens. `first` is the first engine the saved
 * settings would use (null: none set up). A server or OpenRouter already in use
 * stays picked, and so does this PC once setup has been finished (re-run from
 * Settings). On a clean install this PC is on by default but was never chosen,
 * so the recommended engine is picked.
 */
export function initialEngine(first, onboarded, has) {
    if (first === "server" || first === "cloud")
        return first;
    if (first === "pc" && onboarded)
        return "pc";
    return recommendedEngine(has) ?? "cloud";
}
