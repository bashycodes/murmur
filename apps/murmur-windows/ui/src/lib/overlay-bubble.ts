// What the Murmur bar's bubble shows for a pill state (src/overlay.ts).
// Pure, so `npm test` checks it on the built dist.

import { fixFor, type Fix } from "./fix-for.js";

/** error / notice: a dot + one message; answer: Command Mode's answer to
 *  a question about the selection, a multi-line card (nothing was pasted). */
export type BubbleKind = "error" | "notice" | "answer";

export interface Bubble {
  text: string;
  kind: BubbleKind;
  /** The settings page that fixes it; never on an answer (its text is the
   *  AI's, not ours, so "server" in it means nothing). */
  fix: Fix | null;
}

/** The bubble of a pill state; null = none of its own (the live transcript
 *  or a cloud notice may use the bubble then). */
export function bubbleFor(s: { state: string; message?: string }): Bubble | null {
  const text = s.message ?? "";
  if (!text) return null;
  if (s.state === "answer") return { text, kind: "answer", fix: null };
  if (s.state === "error" || s.state === "notice") return { text, kind: s.state, fix: fixFor(text) };
  return null;
}
