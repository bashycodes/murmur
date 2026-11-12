// Which settings page the Murmur bar's "Fix →" link opens for a bubble
// text (src/overlay.ts). Pure, so `npm test` checks it on the built dist.

// Settings pages the Murmur bar may open (src/app.rs OVERLAY_SETTINGS_PAGES).
export type FixPage = "settings-speech" | "settings-cleanup" | "settings-openrouter" | "settings-server";
export interface Fix {
  page: FixPage;
  label: string;
}

/** Which settings page fixes what the bubble says; null = nothing to fix
 *  there ("No speech heard", a microphone or paste error). Cloud first:
 *  "Cloud STT failed (…); used your server" is about OpenRouter. */
export function fixFor(text: string): Fix | null {
  if (/AI cleanup model not found/i.test(text)) return { page: "settings-cleanup", label: "Choose a model →" };
  if (/(speech|whisper|parakeet) model (not found|failed)|model file not found/i.test(text)) {
    return { page: "settings-speech", label: "Choose a model →" };
  }
  // Command Mode / transforms refusing a selection name its destination
  // ("… send the selection to OpenRouter", "… to LiteLLM (10.0.0.5)", "… to
  // the cloud"). The "Text around the cursor" switch that allows it is on the
  // AI cleanup page, whichever the provider; checked before the provider names.
  if (/send the selection to/i.test(text)) return { page: "settings-cleanup", label: "Fix →" };
  if (/cloud stt|openrouter/i.test(text)) return { page: "settings-openrouter", label: "Fix →" };
  if (/cloud cleanup|ai cleanup/i.test(text)) return { page: "settings-cleanup", label: "Fix →" };
  if (/dictation disabled|local model not loaded/i.test(text)) return { page: "settings-speech", label: "Fix →" };
  if (/server/i.test(text)) return { page: "settings-server", label: "Fix →" };
  return null;
}
