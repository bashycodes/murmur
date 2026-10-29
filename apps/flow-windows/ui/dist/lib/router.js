// Which page is on screen. Every page renders its own DOM once at boot
// into its own container; the router only shows one and tells it so.
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
    active() {
        return active;
    },
    /** Called after every navigation (the shell marks its nav). */
    onNavigate(l) {
        navListeners.push(l);
    },
    go(id) {
        const opened = id !== active;
        active = id;
        for (const l of navListeners)
            l(id);
        for (const p of pages.values())
            if ((p.tab ?? p.id) === id)
                p.show?.(opened);
    },
};
