// What the Murmur bar's bubble shows for a pill state (src/overlay.ts).
// Pure, so `npm test` checks it on the built dist.
import { fixFor } from "./fix-for.js";
/** The bubble of a pill state; null = none of its own (the live transcript
 *  or a cloud notice may use the bubble then). */
export function bubbleFor(s) {
    const text = s.message ?? "";
    if (!text)
        return null;
    if (s.state === "answer")
        return { text, kind: "answer", fix: null };
    if (s.state === "error" || s.state === "notice")
        return { text, kind: s.state, fix: fixFor(text) };
    return null;
}
