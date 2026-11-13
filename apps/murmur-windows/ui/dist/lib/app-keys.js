// Keys that work only while the Murmur window has focus: the page listens
// for them, not the keyboard hook, so they never take a key from another
// app (the global shortcuts are in Settings → General, src/hotkeys/).
/** Shown in Settings → General, as keycaps. */
export const OPEN_SETTINGS_KEYS = ["Ctrl", ","];
/** Ctrl+, and nothing else. The comma key by its place too, so a layout
 *  whose comma key types another letter (Hebrew: ת) still matches. */
export function isOpenSettings(e) {
    if (!e.ctrlKey || e.altKey || e.shiftKey || e.metaKey)
        return false;
    return e.key === "," || e.code === "Comma";
}
/** Open Settings on Ctrl+, unless it is already open or something sits
 *  over the shell (setup, a dialog) that the key must not leave. */
export function shouldOpenSettings(e, w) {
    return isOpenSettings(e) && !w.settingsOpen && !w.onboardingOpen && !w.dialogOpen;
}
