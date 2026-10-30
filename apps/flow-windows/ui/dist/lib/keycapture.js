// "Press the keys" for a shortcut field: the keyboard hook records the
// chord the user holds and lets go (`hotkey_capture_start` in src/app.rs,
// src/hotkeys/capture.rs) and answers with a `hotkey:capture` event. The
// hook, not the page, listens: the webview can't tell Right Alt from AltGr
// (Ctrl+Alt) and never sees Win or keys the hook swallows. Hotkeys don't
// fire while recording; it ends on release, Esc, 10 s, or the window
// losing focus.
import { invoke, onEvent } from "./ipc.js";
let pending = null;
let listening = false;
function finish(r) {
    const p = pending;
    pending = null;
    p?.(r);
}
function onBlur() {
    if (!pending)
        return;
    void invoke("hotkey_capture_stop").catch(() => undefined);
    finish({ cancelled: true });
}
/** Record one chord. A recording already running ends as cancelled. */
export async function captureChord() {
    if (!listening) {
        listening = true;
        await onEvent("hotkey:capture", (e) => {
            const { state, chord } = e.payload;
            if (state === "done" && chord)
                finish({ chord });
            else
                finish({ cancelled: true, message: state === "timeout" ? "No keys pressed: kept the old shortcut." : undefined });
        });
        window.addEventListener("blur", onBlur);
    }
    finish({ cancelled: true });
    const result = new Promise((resolve) => (pending = resolve));
    try {
        await invoke("hotkey_capture_start");
    }
    catch (e) {
        finish({ cancelled: true, message: String(e) });
    }
    return result;
}
/** Leave recording without a chord (Cancel / Type instead). */
export function stopCapture() {
    if (!pending)
        return;
    void invoke("hotkey_capture_stop").catch(() => undefined);
    finish({ cancelled: true });
}
/** Is a recording running? */
export function capturing() {
    return pending !== null;
}
export const RECORDING_TEXT = "Press keys… Esc to cancel";
