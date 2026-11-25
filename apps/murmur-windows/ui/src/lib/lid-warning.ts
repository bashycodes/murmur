// Settings → General → Languages: the warning when auto-detect has a Hebrew
// model but no language-ID model (D34: without `ggml-tiny-q5_1.bin`
// auto-detect never routes to the Hebrew model, so it stays English).
// Pure, so tests/lid-warning.test.mjs can run it.

export interface LidInput {
  /** "Keyboard language" (D37) is picked: auto-detect only for unknown layouts, no warning. */
  keyboard: boolean;
  /** `prefs.languages` (empty = Automatic). */
  languages: string[];
  /** A Hebrew model is chosen and on disk (catalogue or the server's library). */
  hebrewModel: boolean;
  /** The language-ID model is on disk (`lid_model_present`); undefined = not known yet. */
  lidPresent: boolean | undefined;
  /** The Hebrew model came from the server's library (D41), not the catalogue. */
  serverHebrew: boolean;
  /**
   * The installed catalogue Hebrew model: downloading it again fetches only
   * its missing files, i.e. the language-ID model (D31 extras). Absent
   * when the Hebrew model is not a catalogue one.
   */
  target?: { canStart: boolean; busy: boolean; missing: number };
}

export type LidWarning =
  | { show: false }
  | {
      show: true;
      text: string;
      /** Label of the one-click download (target's Download), absent when it can't be offered. */
      download?: string;
      /** Where to fix it instead: a Settings page id and the link's text. */
      go?: { page: string; label: string };
    };

/** Size the language-ID model is shown with when the catalogue doesn't say. */
const LID_MB = 32;

/** `he`, `he-IL`, `iw` or `hebrew` (any case). */
export function isHebrewCode(code: string): boolean {
  const p = code.trim().toLowerCase().split(/[-_]/)[0];
  return p === "he" || p === "iw" || p === "hebrew";
}

/**
 * The session's forced language, by murmur-core's rule
 * (`SessionConfig::forced_language`): exactly one entry, not "auto".
 */
export function forcedLanguage(languages: string[]): string | undefined {
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
export function autoDetects(keyboard: boolean, languages: string[]): boolean {
  if (keyboard || forcedLanguage(languages) !== undefined) return false;
  const listed = languages.filter((l) => {
    const p = l.trim().toLowerCase().split(/[-_]/)[0];
    return p !== "" && p !== "auto";
  });
  return listed.length < 2 || listed.some(isHebrewCode);
}

export function lidWarning(i: LidInput): LidWarning {
  if (!autoDetects(i.keyboard, i.languages) || !i.hebrewModel || i.lidPresent !== false) return { show: false };
  const size = i.target && i.target.missing > 0 ? Math.max(1, Math.round(i.target.missing / 1e6)) : LID_MB;
  const why = `Automatic can't recognise Hebrew yet, so every dictation is treated as English. It needs a small language-detection model (${size} MB).`;
  if (i.target?.busy) return { show: true, text: "Downloading the language-detection model…" };
  if (i.target && i.target.canStart && i.target.missing > 0) return { show: true, text: why, download: `Download ${size} MB…` };
  return {
    show: true,
    text: why,
    go: i.serverHebrew
      ? { page: "settings-server", label: "Get it from your server →" }
      : { page: "settings-speech", label: "Get it under Speech to text →" },
  };
}
