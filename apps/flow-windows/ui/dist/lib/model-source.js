// "Download from: Your server | Hugging Face" (D79): where catalogue model
// downloads come from, whether the server has a model, and setup's
// Skip / Continue on the model step. Pure, so `npm test` runs it on the
// built dist without a page.
/** Where downloads come from: the choice (prefs.model_source, default the server) counts only with a server set. */
export function effectiveSource(choice, hasServer) {
    return hasServer && choice !== "huggingface" ? "server" : "huggingface";
}
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
/** The line next to the "Download from" switch. */
export function sourceHint(source, hasServer, host) {
    if (!hasServer)
        return "No server set up, so models come from huggingface.co";
    return source === "server" ? `${host || "Your server"} · no internet needed` : "huggingface.co · needs the internet";
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
