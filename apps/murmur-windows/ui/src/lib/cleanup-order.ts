// The D36 cleanup order (this PC, your server, the cloud, each with a switch):
// small pure edits shared by Settings → AI cleanup and setup.

import type { CleanupEngine, CleanupRoute } from "./types.js";

/** The order with `route` switched on or off, the rest unchanged. */
export function withEngine(list: CleanupEngine[], route: CleanupRoute, enabled: boolean): CleanupEngine[] {
  return list.map((e) => (e.route === route ? { ...e, enabled } : { ...e }));
}
