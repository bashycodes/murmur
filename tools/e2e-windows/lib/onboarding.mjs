// Scenario b's pure parts (first-run setup), so they can be tested here.
import { similarity } from "./similarity.mjs";

/**
 * Setup step 3's recommended row ("English — fast", D94): the catalogue id
 * the onboarding page's PARAKEET_ID names (apps/murmur-windows/ui/src/onboarding/index.ts).
 */
export const RECOMMENDED_ID = "parakeet-tdt-0.6b-v2-int8";

/** The catalogue entry (`model_catalog`) behind the recommended row, or null. */
export function recommendedEntry(catalog) {
  return (catalog ?? []).find((c) => c.id === RECOMMENDED_ID) ?? null;
}

/**
 * models.ps1's -Files: "name:size,…" for the model's own files (extras live
 * next to the folder and do not count for Installed, like the app's
 * `CatalogEntry::installed`).
 */
export function filesArg(entry) {
  const files = (entry?.files ?? []).filter((f) => !f.extra);
  if (!files.length) throw new Error(`catalogue entry ${entry?.id ?? "?"} lists no files`);
  for (const f of files) {
    if (!/^[\w.-]+$/.test(f.name) || !Number.isSafeInteger(f.size)) throw new Error(`unexpected model file ${JSON.stringify(f)}`);
  }
  return files.map((f) => `${f.name}:${f.size}`).join(",");
}

/** The pill's states while the key is held (overlay.ts: active_ptt, active_locked). */
export const wentActive = (states) => (states ?? []).some((s) => String(s).startsWith("active"));

/**
 * The practice box's verdict. The key worked when the box got text and the
 * pill went active; the words are info only (an STT mishearing is not a
 * setup bug). `text` is the practice box's value.
 */
export function gradePractice({ text, states, expect }) {
  const got = String(text ?? "").trim();
  const active = wentActive(states);
  const words = similarity(expect, got);
  return {
    ok: got !== "" && active,
    text_ok: got !== "",
    overlay_active: active,
    word_score: words.score,
    word_recall: words.recall,
  };
}
