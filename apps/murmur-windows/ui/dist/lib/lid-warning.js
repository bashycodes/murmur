// Settings → General → Languages: the warning when auto-detect has a Hebrew
// model but no language-ID model (D34: without `ggml-tiny-q5_1.bin`
// auto-detect never routes to the Hebrew model, so it stays English).
// Pure, so tests/lid-warning.test.mjs can run it.
/** Size the language-ID model is shown with when the catalogue doesn't say. */
const LID_MB = 32;
/** `he`, `he-IL`, `iw` or `hebrew` (any case). */
export function isHebrewCode(code) {
    const p = code.trim().toLowerCase().split(/[-_]/)[0];
    return p === "he" || p === "iw" || p === "hebrew";
}
/**
 * The session's forced language, by murmur-core's rule
 * (`SessionConfig::forced_language`): exactly one entry, not "auto".
 */
export function forcedLanguage(languages) {
    return languages.length === 1 && languages[0] !== "auto" ? languages[0] : undefined;
}
/**
 * Language ID (D34) would choose between English and Hebrew: no forced
 * language (murmur-core `needs_lid`), and Hebrew among the candidates
 * (`lid_candidates`: the session's own list when it names two or more,
 * else English plus the routes, i.e. Hebrew). "Keyboard language" (D37)
 * is left out on purpose: it falls back to this only for a layout it
 * doesn't know, which depends on the app dictated into, not on Settings.
 */
export function autoDetects(keyboard, languages) {
    if (keyboard || forcedLanguage(languages) !== undefined)
        return false;
    const listed = languages.filter((l) => {
        const p = l.trim().toLowerCase().split(/[-_]/)[0];
        return p !== "" && p !== "auto";
    });
    return listed.length < 2 || listed.some(isHebrewCode);
}
export function lidWarning(i) {
    if (!autoDetects(i.keyboard, i.languages) || !i.hebrewModel || i.lidPresent !== false)
        return { show: false };
    const size = i.target && i.target.missing > 0 ? Math.max(1, Math.round(i.target.missing / 1e6)) : LID_MB;
    const why = `Auto-detect can't recognise Hebrew yet, so every dictation is treated as English. It needs a small language-detection model (${size} MB).`;
    if (i.target?.busy)
        return { show: true, text: "Downloading the language-detection model…" };
    if (i.target && i.target.canStart && i.target.missing > 0)
        return { show: true, text: why, download: `Download ${size} MB…` };
    return {
        show: true,
        text: why,
        go: i.serverHebrew
            ? { page: "settings-server", label: "Get it from your server →" }
            : { page: "settings-speech", label: "Get it under Speech to text →" },
    };
}
