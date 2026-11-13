package dev.murmur.core

import org.json.JSONException
import org.json.JSONObject

/**
 * D90 translation: what `nativeTranslate` answered, i.e. murmur-core `translate::result_json`
 * (`{"ok":true,"text","target","on_device"}` / `{"ok":false,"kind","error"}`). The
 * wire shape is pinned by crates/murmur-core/fixtures/translate/translate_results.json.
 */
sealed interface TranslateOutcome {
    /**
     * The cleaned, translated text to type instead of the dictation; [target] = the language it
     * is in ("" from an older library); [onDevice] = the phone's own model answered (lower quality).
     */
    data class Text(val text: String, val target: String = "", val onDevice: Boolean = false) : TranslateOutcome

    /** [kind] = `TranslateError::kind()`; [message] = its user-facing text. The dictation is typed as said. */
    data class Failed(val kind: String, val message: String) : TranslateOutcome

    companion object {
        const val NO_TEXT = "no_text"
        const val TEXT_TOO_LONG = "text_too_long"
        const val NO_TARGET = "no_target"
        const val NO_LLM = "no_llm"
        const val TIMEOUT = "timeout"
        const val EMPTY = "empty"
        /** The answer kept the dictation's script although the target is written in another one. */
        const val NOT_TRANSLATED = "not_translated"
        const val BACKEND = "backend"

        /** Same wording as the core's `TranslateError::NoLlm` (engines / libraries without translation). */
        const val NO_LLM_MESSAGE = "Translation needs AI: turn on AI cleanup (your server, the cloud or a local model)"

        val noLlm: Failed get() = Failed(NO_LLM, NO_LLM_MESSAGE)

        /** Never throws: an unreadable answer is a [BACKEND] failure (the dictation is typed as said). */
        fun decode(json: String?): TranslateOutcome = try {
            val o = JSONObject(json ?: "{}")
            val text = o.optString("text", "")
            if (o.optBoolean("ok") && text.isNotEmpty()) {
                Text(text, o.optString("target", ""), o.optBoolean("on_device"))
            } else if (o.optBoolean("ok")) {
                Failed(EMPTY, "AI returned no translation")
            } else {
                Failed(o.optString("kind", BACKEND).ifEmpty { BACKEND }, o.optString("error", "").ifEmpty { "Translation failed" })
            }
        } catch (e: JSONException) {
            Failed(BACKEND, "Translation failed: unreadable answer")
        }

        /**
         * The `nativeTranslate` request: [target] null or blank = automatic (the first of
         * [installed], the keyboard languages, that is not the dictation's; D58).
         */
        fun request(text: String, source: String?, target: String?, installed: List<String>, dictionary: List<DictEntry>): String =
            JSONObject().apply {
                put("text", text)
                if (!source.isNullOrBlank()) put("source", source)
                if (!target.isNullOrBlank()) put("target", target)
                put("installed", org.json.JSONArray(installed))
                put("dictionary", org.json.JSONArray(CoreJson.encodeDictionary(dictionary)))
            }.toString()
    }
}
