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
