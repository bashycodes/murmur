// D105: Settings → General → Send to webhook. What the line under the two
// fields says. Pure, so tests/webhook.test.mjs can run it. https:// and
// http:// both go to any address (D116); the backend (src/webhook.rs)
// checks the URL's shape, and a save it refuses shows its reason here.
/** The URL without its query string: never shown in full (it may hold a token). */
export function redactUrl(url) {
    const u = url.trim();
    const cut = u.search(/[?#]/);
    return cut < 0 ? u : `${u.slice(0, cut)}?…`;
}
/** A URL the backend will at least try to check (https:// or http://). */
export function looksLikeUrl(url) {
    return /^https?:\/\/\S+$/i.test(url.trim());
}
/** The status line under the fields: on only when both are set. */
export function webhookStatus(url, shortcut) {
    const u = url.trim();
    const k = shortcut.trim();
    if (u && !looksLikeUrl(u)) {
        return { on: false, text: "The URL must start with https:// or http://." };
    }
    if (!u && !k)
        return { on: false, text: "Off. Set a URL and a shortcut to turn it on." };
    if (!u)
        return { on: false, text: "Off until you set a URL." };
    if (!k)
        return { on: false, text: "Off until you set a shortcut." };
    return {
        on: true,
        text: `On: hold ${k}, speak and let go. The text is sent to ${redactUrl(u)} and not pasted.`,
    };
}
