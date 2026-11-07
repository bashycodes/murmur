// The cloud cleanup "Try it" row's words (Settings → AI cleanup → Cloud
// server): plain sentences, never a backend code. Pure, so `npm test` runs
// it on the built dist without a page.
import { cloudHint } from "./format.js";
/** The words for a backend code; unknown codes get a generic phrase, never the code. */
export function plainHint(code) {
    return cloudHint(code, "something went wrong");
}
/**
 * Why cloud cleanup is not used, before the circuit and the usage. `null` =
 * nothing blocks it (the caller shows the on line). `cloud_cleanup_not_configured`
 * covers several causes (D28: toggle off, no consent, no key, custom server not
 * set up); this tells them apart from the view. The Cleanup order switch comes
 * first, then the custom server's address and model (its consent needs a host,
 * so it reads as missing until the address is typed), then consent and key.
 */
export function cleanupBlockedLine(st, v) {
    const custom = st.provider === "custom";
    const where = custom ? st.host || "your server" : "OpenRouter";
    if (!st.supported)
        return { text: "Not in this build.", cls: "hint" };
    if (!st.enabled) {
        return { text: `Cloud cleanup is off in Cleanup order: ${where} gets no text.`, cls: "hint", turnOn: true };
    }
    if (!st.blocked)
        return null;
    if (st.blocked !== "cloud_cleanup_not_configured") {
        return { text: `Not used: ${plainHint(st.blocked)}.`, cls: "hint err" };
    }
    let why;
    if (custom && !v?.cleanup.custom.base_url)
        why = "enter the server address";
    else if (custom && !v?.cleanup.custom.model)
        why = "pick a model";
    else if (!st.consent)
        why = "accept the consent first (turn it off and on again in Cleanup order)";
    else if (!custom && v && !v.key.set)
        why = "save an OpenRouter key";
    else
        why = "not set up yet";
    return { text: `Not used: ${why}.`, cls: "hint err" };
}
/** Who cleaned the last dictation, for the row's "last:" part. */
export function cleanupEngineName(engine, provider, host) {
    if (engine === "cloud")
        return provider === "custom" ? host || "your server" : "OpenRouter";
    if (engine === "local")
        return "this PC";
    if (engine === "server")
        return "your server";
    return "another engine";
}
/** A backend detail is shown only when it is a sentence, not a code. */
const CODE_LIKE = /\b[a-z0-9]+(?:_[a-z0-9]+)+\b/;
/**
 * A failed "Test" (`code` = its error, `null` when the call itself failed):
 * the cause in words, never the code. `cloud_not_configured` means no
 * OpenRouter key on OpenRouter; on the custom server it only comes from a
 * build without cloud support.
 */
export function cleanupTestFailText(code, message, provider) {
    let why;
    if (code === "cloud_not_configured") {
        why = provider === "custom" ? "this build can't reach a cleanup server" : "save an OpenRouter key first";
    }
    else {
        why = plainHint(code ?? "error");
    }
    const detail = message && message !== why && !CODE_LIKE.test(message) ? ` (${message})` : "";
    return `Failed: ${why}${detail}`;
}
