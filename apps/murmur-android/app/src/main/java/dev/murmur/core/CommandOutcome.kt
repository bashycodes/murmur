package dev.murmur.core

import org.json.JSONException
import org.json.JSONObject

/**
 * Command Mode (2026-10-28): what `nativeCommand` answered, i.e. murmur-core
 * `command::result_json` (`{"ok":true,"text"}` / `{"ok":false,"kind","error"}`). The wire
 * shape is pinned by crates/murmur-core/fixtures/command/command_results.json.
 */
sealed interface CommandOutcome {
    /** The rewrite (or generated text) to put in place of the selection / at the cursor. */
    data class Text(val text: String) : CommandOutcome

    /** [kind] = `CommandError::kind()`; [message] = its pill text. Nothing is written. */
    data class Failed(val kind: String, val message: String) : CommandOutcome

    companion object {
        const val NO_INSTRUCTION = "no_instruction"
        const val SELECTION_TOO_LONG = "selection_too_long"
        const val NO_LLM = "no_llm"
        const val SELECTION_NOT_ALLOWED = "selection_not_allowed"
        const val TIMEOUT = "timeout"
        const val EMPTY = "empty"
        /** A Hebrew (non-Latin) selection came back in another script, retry included. */
        const val LANGUAGE_CHANGED = "language_changed"
        const val BACKEND = "backend"

        /** Same wording as the core's `CommandError::NoLlm` (engines / libraries without Command Mode). */
        const val NO_LLM_MESSAGE = "Command Mode needs AI: turn on AI cleanup (local model) or OpenRouter text cleanup"

        val noLlm: Failed get() = Failed(NO_LLM, NO_LLM_MESSAGE)

        /** Never throws: an unreadable answer is a [BACKEND] failure (nothing gets written). */
        fun decode(json: String?): CommandOutcome = try {
            val o = JSONObject(json ?: "{}")
            val text = o.optString("text", "")
            if (o.optBoolean("ok") && text.isNotEmpty()) {
                Text(text)
            } else if (o.optBoolean("ok")) {
                Failed(EMPTY, "AI returned no text")
            } else {
                Failed(o.optString("kind", BACKEND).ifEmpty { BACKEND }, o.optString("error", "").ifEmpty { "AI failed" })
            }
        } catch (e: JSONException) {
            Failed(BACKEND, "AI failed: unreadable answer")
        }
    }
}
