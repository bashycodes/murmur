// The cloud cleanup "Try it" row's words (Settings → AI cleanup → Cloud
// server): plain sentences, never a backend code. Pure, so `npm test` runs
// it on the built dist without a page.

import { cloudHint } from "./format.js";
import type { CleanupStatus, OrView } from "./types.js";

/** What the row says; `turnOn` = offer a button that switches cloud cleanup on in Cleanup order. */
export interface CleanupStateLine {
  text: string;
  cls: "hint" | "hint ok" | "hint err";
  turnOn?: boolean;
}

/**
 * Why cloud cleanup is not used, before the circuit and the usage. `null` =
 * nothing blocks it (the caller shows the on line). `cloud_cleanup_not_configured`
 * covers several causes (D28: toggle off, no consent, no key, custom server not
 * set up); this tells them apart from the view, the switch in Cleanup order first.
 */
export function cleanupBlockedLine(st: CleanupStatus, v: OrView | null): CleanupStateLine | null {
  const custom = st.provider === "custom";
  const where = custom ? st.host || "your server" : "OpenRouter";
  if (!st.supported) return { text: "Not in this build.", cls: "hint" };
  if (!st.enabled) {
    return { text: `Cloud cleanup is off in Cleanup order: ${where} gets no text.`, cls: "hint", turnOn: true };
  }
  if (!st.blocked) return null;
  if (st.blocked !== "cloud_cleanup_not_configured") {
    return { text: `Not used: ${plainHint(st.blocked)}.`, cls: "hint err" };
  }
  let why: string;
  if (!st.consent) why = "accept the consent first (turn it off and on again in Cleanup order)";
  else if (custom && !v?.cleanup.custom.base_url) why = "enter the server address";
  else if (custom && !v?.cleanup.custom.model) why = "pick a model";
  else if (!custom && v && !v.key.set) why = "save an OpenRouter key";
  else why = "not set up yet";
  return { text: `Not used: ${why}.`, cls: "hint err" };
}

/** A failed "Test": the hint for a known code, never the code itself. */
export function cleanupTestFailText(code: string | null | undefined, message?: string | null): string {
  const c = code ?? "error";
  const why = c === "cloud_not_configured" ? "save an OpenRouter key first" : plainHint(c);
  return `Failed: ${why}${message && message !== why ? ` (${message})` : ""}`;
}

/** `cloudHint` without the raw-code fallback. */
function plainHint(code: string): string {
  const h = cloudHint(code);
  return h === code ? "something went wrong" : h;
}
