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
  consent_needed: "consent needed: press again and accept, or paste the key",
};

/** The words for `code`; `fallback` (default: the code itself, `?` for none) when unknown. */
export function cloudHint(code: string | null | undefined, fallback?: string): string {
  return code ? (CLOUD_HINTS[code] ?? fallback ?? code) : (fallback ?? "?");
}

/** `code — hint` (+ the backend's message) for a failed OpenRouter call. */
export function failText(code: string | null | undefined, message?: string | null): string {
  const c = code ?? "error";
  const h = CLOUD_HINTS[c];
  return `${c}${h ? ` — ${h}` : ""}${message ? ` (${message})` : ""}`;
}

/**
 * D112: the server locked this address out after too many wrong keys. The
 * same sentence as the backend's (`murmur_router::throttled_text`); the last
 * part tells a user who already fixed the key why it still fails.
 */
export function throttledText(retryAfterS?: number | null): string {
  const lead = "Too many wrong keys from this address.";
  if (retryAfterS == null) return `${lead} Try again later. Your key isn't checked until then.`;
  if (retryAfterS <= 0) return `${lead} The wait is over: try again.`;
  return `${lead} Try again in ${retryAfterS} s. Your key isn't checked until then.`;
}

/** A probe's lockout wait as seen [nowMs]: `retry_after_s` minus the time since `checked_at_ms`. */
export function throttledLeft(
  h: { retry_after_s?: number | null; checked_at_ms?: number | null },
  nowMs: number = Date.now(),
): number | null {
  if (h.retry_after_s == null) return null;
  if (h.checked_at_ms == null) return h.retry_after_s;
  return Math.max(0, h.retry_after_s - Math.floor(Math.max(0, nowMs - h.checked_at_ms) / 1000));
}

type FailedHealth = { error: string | null; message: string | null; retry_after_s?: number | null; checked_at_ms?: number | null };

/**
 * A failed server Test: `code: message — hint`, or the lockout sentence alone
 * for `throttled` (the server is reachable; "is it running?" would mislead),
 * with the wait left now rather than at probe time.
 */
export function serverFailText(h: FailedHealth, hint: (code: string) => string, nowMs: number = Date.now()): string {
  const code = h.error ?? "error";
  if (code === "throttled") return throttledText(throttledLeft(h, nowMs));
  const why = hint(code);
  return `${code}${h.message ? `: ${h.message}` : ""}${why ? ` — ${why}` : ""}`;
}

/** D112: a server route's badge: a lockout is "Locked out", not "Unreachable". */
export function serverBadge(h: { ok: boolean; error?: string | null }): [string, "success" | "warning" | "destructive"] {
  if (h.ok) return ["Connected", "success"];
  return h.error === "throttled" ? ["Locked out", "warning"] : ["Unreachable", "destructive"];
}

/** Money from the backend: preformatted "$0.0234" strings pass through. */
export function money(v: string | number | null | undefined): string {
  if (v == null || v === "") return "$0";
  if (typeof v === "string") return v.startsWith("$") ? v : `$${v}`;
  return `$${v < 1 && v > 0 ? v.toFixed(4) : v.toFixed(2)}`;
}

/**
 * D52: this month's cleanup cost split by service. `month_cost_usd` counts both;
 * OpenRouter's part (what its budget counts) and the custom server's are shown apart.
 */
export function cleanupMonthCost(st: { month_cost_usd: number | null; custom_month_cost_usd?: number | null }): {
  openrouter: number;
  custom: number;
} {
  const custom = st.custom_month_cost_usd ?? 0;
  return { openrouter: Math.max(0, (st.month_cost_usd ?? 0) - custom), custom };
}

export function usd(v: number | null | undefined): string {
  return v == null ? "" : ` · $${v.toFixed(v < 0.01 ? 5 : 4)}`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/**
 * One date format for tables: "11 Nov 2026, 22:51" in local time, today or
 * not, so a column never mixes "12:04" with full dates.
 */
export function dateTime(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}, ${hh}:${mm}`;
}
