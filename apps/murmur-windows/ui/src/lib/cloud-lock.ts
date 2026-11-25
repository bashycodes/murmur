// The provider cards (Speech → Cloud, AI cleanup → Cloud) are locked while
// their "Use the cloud" switch is off: the shared opt-out lock
// (lib/opt-out-lock.ts) disables the card's controls and shows a banner
// whose button runs the switch's own consent flow. Before, the cards were
// only dimmed and stayed editable. The switches themselves sit in the
// engine lists above, outside the locked region. Pure: tests run it in node.

import { optOutMarkup, wireOptOut } from "./opt-out-lock.js";

const KEPT = "Nothing is sent there; these settings are kept.";
export const STT_LOCK = optOutMarkup("or_stt_lock", "Cloud speech", KEPT);
export const CLEANUP_LOCK = optOutMarkup("or_cleanup_lock", "Cloud cleanup", KEPT);

/** The switch each card's banner turns on (and then focuses). */
export const STT_SWITCH = "or_stt_allow";
export const CLEANUP_SWITCH = "or_cleanup_allow";

export interface CloudLocks {
  stt: (on: boolean) => void;
  cleanup: (on: boolean) => void;
}

/** Wire both banners; `turnOn` runs a switch's usual flow (consent first)
 *  and resolves to whether the switch ended up on. */
export function wireCloudLocks(
  find: (id: string) => unknown,
  turnOn: { stt: () => Promise<boolean>; cleanup: () => Promise<boolean> },
): CloudLocks {
  return {
    stt: wireOptOut("or_stt_lock", find, { turnOn: turnOn.stt, switchId: STT_SWITCH }),
    cleanup: wireOptOut("or_cleanup_lock", find, { turnOn: turnOn.cleanup, switchId: CLEANUP_SWITCH }),
  };
}
