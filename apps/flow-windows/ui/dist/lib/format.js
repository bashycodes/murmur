// Text for backend codes and amounts, shared by the engine and cloud pages.
const CLOUD_HINTS = {
    auth_failed: "key rejected",
    quota_exceeded: "out of credits",
    connect_failed: "can't reach openrouter.ai",
    budget_exceeded: "monthly budget reached",
    cloud_not_configured: "save a key and accept the consent",
    cloud_cleanup_not_configured: "turn on cleanup, accept its consent and save a key",
    no_private_route: "no zero-retention provider serves this model: pick a model marked ZDR, or allow other providers",
    provider_error: "the provider returned an error",
    timeout: "took too long",
    busy: "rate limited",
    circuit_open: "paused after recent failures",
};
export function cloudHint(code) {
    return code ? (CLOUD_HINTS[code] ?? code) : "?";
}
/** `code — hint` (+ the backend's message) for a failed OpenRouter call. */
export function failText(code, message) {
    const c = code ?? "error";
    const h = CLOUD_HINTS[c];
    return `${c}${h ? ` — ${h}` : ""}${message ? ` (${message})` : ""}`;
}
/** Money from the backend: preformatted "$0.0234" strings pass through. */
export function money(v) {
    if (v == null || v === "")
        return "$0";
    if (typeof v === "string")
        return v.startsWith("$") ? v : `$${v}`;
    return `$${v < 1 && v > 0 ? v.toFixed(4) : v.toFixed(2)}`;
}
/**
 * D52: this month's cleanup cost split by service. `month_cost_usd` counts both;
 * OpenRouter's part (what its budget counts) and the custom server's are shown apart.
 */
export function cleanupMonthCost(st) {
    const custom = st.custom_month_cost_usd ?? 0;
    return { openrouter: Math.max(0, (st.month_cost_usd ?? 0) - custom), custom };
}
export function usd(v) {
    return v == null ? "" : ` · $${v.toFixed(v < 0.01 ? 5 : 4)}`;
}
