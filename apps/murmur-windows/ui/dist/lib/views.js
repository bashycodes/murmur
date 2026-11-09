// The last DevView (`dev_get` / `dev_set`) and OrView (`dev_or_*`) the page
// received, shared by every page that shows engine or cloud state. The
// page that fetched a view publishes it; others subscribe instead of
// reaching into each other.
const devListeners = [];
const orListeners = [];
export const views = {
    dev: null,
    or: null,
    onDev(l) {
        devListeners.push(l);
    },
    onOr(l) {
        orListeners.push(l);
    },
    setDev(v) {
        views.dev = v;
        for (const l of devListeners)
            l(v);
    },
    /** null = a build without OpenRouter. */
    setOr(v) {
        const prev = views.or;
        views.or = v;
        for (const l of orListeners)
            l(v, prev);
    },
};
