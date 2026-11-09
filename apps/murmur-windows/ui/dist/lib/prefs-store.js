// The single owner of prefs.json on this page: the ONLY caller of
// `set_prefs`. Pages save a patch of the fields they own; the store lays
// it over the latest saved prefs, so one page's save can never roll back
// another page's earlier save (the old single form read every field on
// every Save). Field names mirror `src/prefs.rs` (snake_case).
import { invoke } from "./ipc.js";
let current = null;
const listeners = [];
function publish(p) {
    current = p;
    for (const l of listeners)
        l(p);
}
export const prefsStore = {
    /** The last saved (or loaded) prefs; null before `load()`. */
    get() {
        return current;
    },
    /** Called with the new prefs after every load, save or adopt. */
    subscribe(l) {
        listeners.push(l);
    },
    async load() {
        const p = await invoke("get_prefs");
        publish(p);
        return p;
    },
    /** Save `patch` over the latest prefs. Throws the backend's error. */
    async save(patch) {
        if (!current)
            throw new Error("prefs not loaded yet");
        const next = { ...current, ...patch };
        await invoke("set_prefs", { prefs: next });
        publish(next);
        return next;
    },
    /** Take prefs the backend changed itself (e.g. `suggestion_accept`). */
    adopt(p) {
        publish(p);
    },
};
