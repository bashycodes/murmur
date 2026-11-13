// D96: the Transforms banner's engine sentence, from the live cleanup order.
// It mirrors how the router picks a transform's engine (murmur_router
// `Router::command_cleaner` / `legacy_plan`, router.rs): the server never
// runs transforms; the cloud route is "wanted" when it is on and listed,
// ready or not (a cloud that can't answer then falls back or fails); with
// the cloud wanted and first, this PC follows only when it is on; with
// this PC first and its model loaded, it alone answers; without the cloud
// wanted, this PC's loaded model answers even with its switch off. Pure,
// tested in tests/transforms-line.test.mjs against the router's cases.
import { cloudRouteInSentence } from "./provider-names.js";
/** `allow_cloud_cleanup` in a build with cloud cleanup. */
export function cloudCleanupAllowed(or) {
    return !!or?.supported && !!or.cleanup.allow;
}
/** The engines a transform runs on, in order (router `command_cleaner`). */
export function transformEngines(t) {
    // legacy_plan's order: the cleanup order without the server and switched-off engines.
    const order = t.engines.filter((e) => e.route !== "server" && e.enabled).map((e) => e.route);
    const wanted = t.cloudAllowed && order.includes("cloud");
    if (!wanted)
        return t.local ? ["local"] : [];
    if (order[0] === "cloud")
        return t.local && order.includes("local") ? ["cloud", "local"] : ["cloud"];
    return t.local ? ["local"] : ["cloud"];
}
/** The banner sentence. */
export function transformsEngineLine(t) {
    const names = transformEngines(t).map((r) => (r === "cloud" ? cloudRouteInSentence(t.provider) : "this PC"));
    const serverNote = t.server ? " Your server doesn't run transforms." : "";
    if (names.length === 0) {
        return `Needs an AI model on this PC or ${cloudRouteInSentence(t.provider)}: turn one on under AI cleanup.${serverNote}`;
    }
    const list = names.join(", then ");
    return `Uses ${list}.${serverNote}`;
}
