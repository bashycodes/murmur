// The inline ask on the AI cleanup page's provider card: dictation cleanup
// works with the provider alone, but Command Mode and transforms send the
// selected text, which the core counts as text around the cursor (D28
// scope, `murmur_core::command::selection_route`). Instead of sending the
// user to the Advanced fold, the card asks right there, and only while
// the answer matters (cloud cleanup on, that switch off). Pure, so
// `npm test` runs it on the built dist without a page.
import { cloudWhere } from "./provider-names.js";
/** The ask's sentence, or null when there is nothing to ask. */
export function selectionAsk(s) {
    if (!s.cloudOn || s.sendSurrounding)
        return null;
    return `To rewrite selected text ${cloudWhere(s.provider)} (Command Mode, transforms), also send the text around your cursor, which includes the selection. Dictation cleanup doesn't need it.`;
}
/** The cleanup form's three "Also send to the cloud" switches, as saved
 *  by `dev_or_set_cleanup` (features/openrouter.ts). */
export function contextSwitches(find) {
    const on = (id) => find(id).checked;
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
export function wireSelectionAsk(find, save) {
    find("or_cleanup_sel_on").addEventListener("click", () => {
        find("or_cleanup_send_surrounding_text").checked = true;
        save();
    });
    return (s) => {
        const ask = selectionAsk(s);
        find("or_cleanup_sel_row").hidden = ask === null;
        find("or_cleanup_sel_ask").textContent = ask ?? "";
    };
}
