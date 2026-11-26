// The server's team dictionary (src/team_dictionary.rs): the last list the
// backend has, fetched again (with its ETag, so usually a 304) when the
// Settings window loads or shows and when a page asks. Dictionary shows it
// read-only; Users & keys edits it.
import { invoke } from "./ipc.js";
const listeners = [];
let view = { configured: false, entries: [] };
let asking = 0;
let lastAsked = 0;
function take(v) {
    view = v;
    for (const l of listeners)
        l(v);
    return v;
}
export const teamStore = {
    get: () => view,
    onChange(l) {
        listeners.push(l);
    },
    /** Fetch it again if it changed. Only the latest answer counts. */
    async refresh() {
        const n = ++asking;
        lastAsked = Date.now();
        let v;
        try {
            v = await invoke("team_dictionary_refresh");
        }
        catch (e) {
            v = { ...view, error: String(e) };
        }
        return n === asking ? take(v) : view;
    },
    /** `refresh`, unless one ran in the last `ms`. */
    refreshIfOlder(ms) {
        if (Date.now() - lastAsked >= ms)
            void teamStore.refresh();
    },
    /** Admin: add an entry or replace the one with that word. Throws the reason. */
    async add(word, replacement) {
        asking++;
        return take(await invoke("team_dictionary_add", { word, replacement }));
    },
    /** Admin: remove an entry. Throws the reason. */
    async remove(word) {
        asking++;
        return take(await invoke("team_dictionary_remove", { word }));
    },
};
