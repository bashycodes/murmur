// D96: what the engines are called. "Cloud" means OpenRouter only; the
// OpenAI-compatible path (D52, wire "custom", e.g. a LiteLLM proxy) is
// "Your own AI server"; "Your server" stays the Murmur server. Settings →
// AI provider is where one of the two is picked. Also the Transforms line
// and the Data and privacy rows, built from the live engines. Pure, so
// `npm test` runs it on the built dist without a page.
/** The sidebar tab and heading of the page that picks the provider. */
export const PROVIDER_PAGE = "AI provider";
export const OWN_AI_SERVER = "Your own AI server";
export const OWN_AI_SERVER_FULL = "Your own AI server (OpenAI-compatible)";
const isOwn = (p) => p === "custom";
/** The provider's name on its own: "OpenRouter" / "Your own AI server (OpenAI-compatible)". */
export function providerName(p) {
    return isOwn(p) ? OWN_AI_SERVER_FULL : "OpenRouter";
}
/** The cloud route's title in an engine list or a card heading. */
export function cloudRouteTitle(p) {
    return isOwn(p) ? OWN_AI_SERVER : "Cloud · OpenRouter";
}
/** The cloud route inside a sentence: "OpenRouter in the cloud" / "your own AI server". */
export function cloudRouteInSentence(p) {
    return isOwn(p) ? "your own AI server" : "OpenRouter in the cloud";
}
/** The sidebar Engines card's live label: "Cloud · <model>" / "Your own AI server · <host>". */
export function cloudLiveLabel(p, detail) {
    const head = isOwn(p) ? OWN_AI_SERVER : "Cloud";
    return detail ? `${head} · ${detail}` : head;
}
/** The cloud cleanup route is on and could answer: consent, and a key (OpenRouter) or a model (your own AI server). */
export function cloudCleanupUsable(or) {
    const c = or?.cleanup;
    if (!or?.supported || !c?.allow)
        return false;
    return c.provider === "custom" ? c.custom.consent && c.custom.model !== "" : c.consent && or.key.set;
}
/**
 * The Transforms banner's engine sentence, from the live cleanup order.
 * Transforms run on the cloud route and this PC in that order
 * (murmur_router `command_cleaner`); your server never runs them, which
 * the line says when it is the dictation cleaner.
 */
export function transformsEngineLine(t) {
    const names = [];
    for (const e of t.engines) {
        if (e.route === "cloud" && t.cloud)
            names.push(cloudRouteInSentence(t.provider));
        // Without the cloud route a loaded model on this PC answers even with its switch off.
        if (e.route === "local" && t.local && (e.enabled || !t.cloud))
            names.push("this PC");
    }
    const serverNote = t.server ? " Your server doesn't run transforms." : "";
    if (names.length === 0) {
        return `Needs an AI model on this PC or ${cloudRouteInSentence(t.provider)}: turn one on under AI cleanup.${serverNote}`;
    }
    const list = names.length === 1 ? names[0] : `${names[0]}, then ${names.slice(1).join(", then ")}`;
    return `Uses your AI cleanup engines in their order: ${list}.${serverNote}`;
}
/** Server cleanup is on: in the order with its switch on, a server set, not known to lack an AI model. */
export function serverCleanupOn(engines, host, remote, serverLlm) {
    return remote && host !== "" && serverLlm !== false && engines.some((e) => e.route === "server" && e.enabled);
}
/** Data and privacy → "Text to your server": the dictated text your server cleans. */
export function serverTextLeave(on, host, tls) {
    return {
        title: "Text to your server",
        on,
        desc: on
            ? `For AI cleanup on ${host}${tls ? ", encrypted" : ""}. Your own machine; text goes nowhere else.`
            : "Off: your server doesn't clean text.",
    };
}
/** Data and privacy → the audio and text rows of the picked provider. */
export function cloudLeaves(c) {
    const own = isOwn(c.provider);
    const where = own ? "your own AI server" : "the cloud";
    const at = own ? ` at ${c.host || "the address you set"}` : "";
    const zdr = !own && c.zdr ? " Zero-retention providers only." : "";
    const off = "Off. Turning it on asks first.";
    return {
        audio: {
            title: `Audio to ${where}`,
            on: c.sttAllow,
            desc: c.sttAllow ? `For speech to text${at} with ${c.sttModel}.` : off,
        },
        text: {
            title: `Text to ${where}`,
            on: c.cleanupAllow,
            desc: c.cleanupAllow ? `For AI cleanup${at} with ${c.cleanupModel || "no model picked yet"}.${zdr}` : off,
        },
    };
}
