// Text for backend codes and amounts, shared by the engine and cloud pages.

const CLOUD_HINTS: Record<string, string> = {
  auth_failed: "key rejected",
  quota_exceeded: "out of credits",
  connect_failed: "can't reach openrouter.ai",
  budget_exceeded: "monthly budget reached",
  cloud_not_configured: "save a key and accept the consent",
  cloud_cleanup_not_configured: "turn on cleanup, accept its consent and save a key",
  no_private_route:
    "no zero-retention provider serves this model: pick a model marked ZDR, or allow other providers",
  provider_error: "the provider returned an error",
  timeout: "took too long",
  busy: "rate limited",
  circuit_open: "paused after recent failures",
};

export function cloudHint(code: string | null | undefined): string {
  return code ? (CLOUD_HINTS[code] ?? code) : "?";
}

/** `code — hint` (+ the backend's message) for a failed OpenRouter call. */
export function failText(code: string | null | undefined, message?: string | null): string {
  const c = code ?? "error";
  const h = CLOUD_HINTS[c];
  return `${c}${h ? ` — ${h}` : ""}${message ? ` (${message})` : ""}`;
}

/** Money from the backend: preformatted "$0.0234" strings pass through. */
export function money(v: string | number | null | undefined): string {
  if (v == null || v === "") return "$0";
  if (typeof v === "string") return v.startsWith("$") ? v : `$${v}`;
  return `$${v < 1 && v > 0 ? v.toFixed(4) : v.toFixed(2)}`;
}

export function usd(v: number | null | undefined): string {
  return v == null ? "" : ` · $${v.toFixed(v < 0.01 ? 5 : 4)}`;
}
