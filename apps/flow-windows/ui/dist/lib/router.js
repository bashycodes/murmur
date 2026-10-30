// Which page is on screen. Every page renders its own static template once
// at boot into an empty container the shell made for it; the router only
// shows one page and tells it so.
const pages = new Map();
let active = "";
const navListeners = [];
export const router = {
    register(p) {
        pages.set(p.id, p);
    },
    pages() {
        return [...pages.values()];
    },
    has(id) {
        return pages.has(id);
    },
    active() {
        return active;
    },
    /** Called after every navigation (the shell shows the page, marks its nav). */
    onNavigate(l) {
        navListeners.push(l);
    },
    go(id) {
        if (!pages.has(id))
            return;
        const prev = active;
        const opened = id !== prev;
        active = id;
        for (const l of navListeners)
            l(id, prev);
        pages.get(id)?.show?.(opened);
    },
};
