// Settings → Users & keys: what the table shows for each key. Pure (no DOM),
// so `npm test` runs it on the built dist.

import { dateTime } from "./format.js";
import type { KeyView } from "./types.js";

/** Active keys in the server's order (oldest first); revoked ones, latest revoked first. */
export function splitKeys(keys: KeyView[]): { active: KeyView[]; revoked: KeyView[] } {
  return {
    active: keys.filter((k) => !k.revoked),
    revoked: keys.filter((k) => k.revoked).sort((a, b) => (b.revoked ?? 0) - (a.revoked ?? 0)),
  };
}

/** A server timestamp (seconds) in the table's one date format. */
export const keyTime = (secs?: number | null): string => (secs ? dateTime(secs * 1000) : "Never");

/** Used before the server tracked last use: counts but no time. */
const usedBeforeTracking = (k: KeyView): boolean => !k.last_used && !!(k.dictations || k.downloads);

/** The Last used cell: "—" rather than a made-up date for a key used before tracking. */
export function lastUsed(k: KeyView): string {
  if (usedBeforeTracking(k)) return "—";
  return keyTime(k.last_used);
}

/** Hover text on a key's name: the build it last connected with, then what the table leaves out. */
export function keyDetails(k: KeyView): string {
  const lines = [k.client ?? "Not used yet", `Created ${keyTime(k.created)}`];
  // Older servers send only `last_seen`, which is last connected.
  lines.push(`Last connected ${keyTime(k.last_connected ?? k.last_seen)}`);
  if (usedBeforeTracking(k)) lines.push("Last used before the server tracked it");
  lines.push(`Audio ${(k.audio_ms / 60000).toFixed(1)} min`);
  const n = k.downloads ?? 0;
  if (n) lines.push(`Model downloads ${n} (${((k.download_bytes ?? 0) / 1e6).toFixed(1)} MB)`);
  if (k.revoked) lines.push(`Revoked ${keyTime(k.revoked)}`);
  return lines.join("\n");
}
