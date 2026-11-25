// The inline ask on the AI cleanup page's provider card: dictation cleanup
// works with the provider alone, but Command Mode and transforms send the
// selected text, which the core counts as text around the cursor (D28
// scope, `murmur_core::command::selection_route`). Instead of sending the
// user to the Advanced fold, the card asks right there, and only while
// the answer matters (cloud cleanup on, that switch off). Pure, so
// `npm test` runs it on the built dist without a page.

import { cloudWhere } from "./provider-names.js";
import type { CloudProvider } from "./types.js";

export interface SelectionAskInput {
  /** Cloud cleanup is switched on (with its consent). */
  cloudOn: boolean;
  /** "Text around the cursor" may go to the provider. */
  sendSurrounding: boolean;
  provider: CloudProvider | undefined;
}

/** The ask's sentence, or null when there is nothing to ask. */
export function selectionAsk(s: SelectionAskInput): string | null {
  if (!s.cloudOn || s.sendSurrounding) return null;
  return `To rewrite selected text ${cloudWhere(s.provider)} (Command Mode, transforms), also send the text around your cursor, which includes the selection. Dictation cleanup doesn't need it.`;
}

interface Checkbox {
  checked: boolean;
}

/** The cleanup form's three "Also send to the cloud" switches, as saved
 *  by `dev_or_set_cleanup` (features/openrouter.ts). */
export function contextSwitches(find: (id: string) => unknown): {
  send_surrounding_text: boolean;
  send_screen_words: boolean;
  send_app_name: boolean;
} {
  const on = (id: string): boolean => (find(id) as Checkbox).checked;
  return {
    send_surrounding_text: on("or_cleanup_send_surrounding_text"),
    send_screen_words: on("or_cleanup_send_screen_words"),
    send_app_name: on("or_cleanup_send_app_name"),
  };
}

/** Wire the ask's button: it sets Advanced → Text around the cursor (the
 *  same switch; the selection needs nothing else, not even "Use text on
 *  screen") and saves the cleanup form with `save`. Returns `show`, for
 *  the page's render to call with the saved state. */
export function wireSelectionAsk(find: (id: string) => unknown, save: () => void): (s: SelectionAskInput) => void {
  (find("or_cleanup_sel_on") as { addEventListener(t: "click", fn: () => void): void }).addEventListener("click", () => {
    (find("or_cleanup_send_surrounding_text") as Checkbox).checked = true;
    save();
  });
  return (s) => {
    const ask = selectionAsk(s);
    (find("or_cleanup_sel_row") as { hidden: boolean }).hidden = ask === null;
    (find("or_cleanup_sel_ask") as { textContent: string }).textContent = ask ?? "";
  };
}
