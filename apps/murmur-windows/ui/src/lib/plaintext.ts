// D100: "Allow an unencrypted connection" is per server. These rules keep it
// from following the user to another address, and spot the refusal that
// offers it. Pure, so tests/plaintext.test.mjs can run them.

/** Same server address? Trimmed, case-insensitive, IPv6 brackets ignored
 *  (the backend stores IPv6 without them). */
export function sameHost(a: string, b: string): boolean {
  const norm = (h: string): string => h.trim().replace(/^\[(.*)\]$/, "$1").toLowerCase();
  return norm(a) === norm(b);
}

/** The opt-in to save with `host`: the saved one only while the address is
 *  unchanged; a new address starts without it. */
export function optInFor(savedHost: string, savedOptIn: boolean, host: string): boolean {
  return savedOptIn && sameHost(savedHost, host);
}

/** murmur-remote refused plain ws:// to a non-private address (review S6). */
export function isPlaintextRefusal(message: string | null | undefined): boolean {
  return /plaintext (refused|to public address)/.test(message ?? "");
}

/** What setup and Test connection show for that refusal. */
export const PLAINTEXT_REFUSED_TEXT =
  "Server needs TLS — its address is not private (or resolves to a public one). If your server has a certificate, turn on TLS in Settings → Your server. Only on a network you trust, allow an unencrypted connection there instead.";
