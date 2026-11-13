// The D36 cleanup order (this PC, your server, the cloud, each with a switch):
// small pure edits shared by Settings → AI cleanup and setup.
/** The order with `route` switched on or off, the rest unchanged. */
export function withEngine(list, route, enabled) {
    return list.map((e) => (e.route === route ? { ...e, enabled } : { ...e }));
}
