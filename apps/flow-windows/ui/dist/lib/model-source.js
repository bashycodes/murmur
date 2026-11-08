// "Download from: Your server | Hugging Face" (D79): what setup shows for
// it, whether the server has a model, and setup's Skip / Continue on the
// model step. Where downloads really come from is the backend's call
// (server_library::Origin, seen as the catalogue's from_server); this only
// lays the user's pick over it. Pure, so `npm test` runs it on the built
// dist without a page.
/**
 * The server's library holds a usable copy of catalogue model `id` (tagged with
 * its id, pins matching, engine in this build). undefined = not known: not read
 * yet or unreadable, so Download stays offered and its error says why.
 */
export function onServer(id, lib) {
    if (!lib || lib.error)
        return undefined;
    return lib.models.some((m) => m.catalogue_id === id && !!m.slot);
}
/** Setup's "Download from" switch over the backend's source. */
export function sourceView(s) {
    const shown = s.remote ? (s.pick ?? s.effective) : "huggingface";
    return {
        shown,
        serverButton: s.remote,
        serverForm: shown === "server" && !s.serverSet,
        downloads: shown === s.effective,
        hfLinks: shown === "huggingface",
    };
}
/** The line next to the "Download from" switch. */
export function sourceHint(v, host) {
    if (v.shown === "huggingface")
        return "huggingface.co · needs the internet";
    if (v.serverForm)
        return "Add your server below. No internet needed.";
    return `${host || "Your server"} · no internet needed`;
}
/**
 * Setup's model step footer: "Skip — use my server only" while speech runs on
 * the server and no model is ready (it goes to the next step, the counter is
 * unchanged); Continue once a model is ready or downloading, or always when the
 * model is optional, else shown disabled.
 */
export function modelStepFoot(s) {
    const started = s.ready || s.busy;
    if (s.serverEngine && !s.ready)
        return { skip: true, next: started ? "enabled" : "hidden" };
    return { skip: false, next: started || !s.needed ? "enabled" : "disabled" };
}
