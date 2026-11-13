// The D36 cleanup order (this PC, your server, the cloud, each with a switch):
// small pure edits shared by Settings → AI cleanup and setup.

import type { CleanupEngine, CleanupRoute } from "./types.js";

/** The order with `route` switched on or off, the rest unchanged. */
export function withEngine(list: CleanupEngine[], route: CleanupRoute, enabled: boolean): CleanupEngine[] {
  return list.map((e) => (e.route === route ? { ...e, enabled } : { ...e }));
}

/** D96: your server cleans text: in the order with its switch on, a server set, not known to lack an AI model. */
export function serverCleanupOn(engines: CleanupEngine[], host: string, remote: boolean, serverLlm: boolean | null | undefined): boolean {
  return remote && host !== "" && serverLlm !== false && engines.some((e) => e.route === "server" && e.enabled);
}
