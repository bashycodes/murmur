// Settings → Users & keys: what the table shows for each key. Pure (no DOM),
// so `npm test` runs it on the built dist.
import { dateTime } from "./format.js";
/** Active keys in the server's order (oldest first); revoked ones, latest revoked first. */
export function splitKeys(keys) {
    return {
        active: keys.filter((k) => !k.revoked),
        revoked: keys.filter((k) => k.revoked).sort((a, b) => (b.revoked ?? 0) - (a.revoked ?? 0)),
    };
}
/** A server timestamp (seconds) in the table's one date format. */
export const at = (secs) => (secs ? dateTime(secs * 1000) : "Never");
/**
 * The Last used cell. A key used before the server tracked last use has
 * counts but no time: "—" rather than a made-up date.
 */
export function lastUsed(k) {
    if (k.last_used)
        return at(k.last_used);
    return k.dictations || k.downloads ? "—" : "Never";
}
/** Hover text on a key's name: the build it last connected with, then what the table leaves out. */
export function keyDetails(k) {
    const lines = [k.client ?? "Not used yet", `Created ${at(k.created)}`];
    // Older servers send only `last_seen`, which is last connected.
    lines.push(`Last connected ${at(k.last_connected ?? k.last_seen)}`);
    if (!k.last_used && (k.dictations || k.downloads))
        lines.push("Last used before the server tracked it");
    lines.push(`Audio ${(k.audio_ms / 60000).toFixed(1)} min`);
    const n = k.downloads ?? 0;
    if (n)
        lines.push(`Model downloads ${n} (${((k.download_bytes ?? 0) / 1e6).toFixed(1)} MB)`);
    if (k.revoked)
        lines.push(`Revoked ${at(k.revoked)}`);
    return lines.join("\n");
}
