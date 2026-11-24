// Settings → General → Languages: the warning when auto-detect has a Hebrew
// model but no language-ID model (D34: without `ggml-tiny-q5_1.bin`
// auto-detect never routes to the Hebrew model, so it stays English).
// Pure, so tests/lid-warning.test.mjs can run it.

export interface LidInput {
  /** "Keyboard language" (D37) is picked: auto-detect only for unknown layouts, no warning. */
  keyboard: boolean;
  /** `prefs.languages` (empty = Detect automatically). */
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

/** Auto-detect picks the language: no fixed language, or several of them including Hebrew (D34). */
export function autoDetects(keyboard: boolean, languages: string[]): boolean {
  if (keyboard) return false;
  const codes = languages.map((c) => c.trim()).filter(Boolean);
  return codes.length === 0 || (codes.length > 1 && codes.some(isHebrewCode));
}

export function lidWarning(i: LidInput): LidWarning {
  if (!autoDetects(i.keyboard, i.languages) || !i.hebrewModel || i.lidPresent !== false) return { show: false };
  const size = i.target && i.target.missing > 0 ? Math.max(1, Math.round(i.target.missing / 1e6)) : LID_MB;
  const why = `Auto-detect can't recognise Hebrew yet, so every dictation is treated as English. It needs a small language-detection model (${size} MB).`;
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
