package dev.flowclone.core

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * JSON wire format between Kotlin and crates/flow-ffi. Field names follow the Rust serde
 * definitions (snake_case; dictionary rows camelCase `isSnippet`; CommitStatus SCREAMING_SNAKE).
 * Uses the framework's org.json (no extra dependency; real impl added for JVM tests).
 */
object CoreJson {
    fun encode(c: ModelConfig): String = JSONObject().apply {
        put("asr_backend", c.mockText?.let { JSONObject().put("mock", JSONObject().put("text", it)) } ?: "whisper")
        put("asr_path", c.asrPath)
        put("llm_path", c.llmPath ?: JSONObject.NULL)
        put("threads", c.threads)
    }.toString()

    fun encode(c: SessionConfig): String = JSONObject().apply {
        put("languages", JSONArray(c.languages))
        put("dictionary", dictionaryArray(c.dictionary))
        put("styles", JSONObject().apply {
            put("personal", c.styles.personal.wire); put("work", c.styles.work.wire)
            put("email", c.styles.email.wire); put("other", c.styles.other.wire)
        })
        put("cleanup_level", c.cleanupLevel.wire)
        put("raw_mode", c.rawMode)
        put("max_samples", c.maxSamples)
        put("platform", "android")
        put("partial_interval_ms", c.partialIntervalMs ?: JSONObject.NULL)
        put("llm_timeout_ms", c.llmTimeoutMs)
    }.toString()

    fun encode(c: FieldContext): String = JSONObject().apply {
        put("app_id", c.appId)
        put("url_host", c.urlHost)
        put("app_category", c.appCategory)
        put("before", c.before)
        put("selected", c.selected)
        put("after", c.after)
        put("screen_words", JSONArray(c.screenWords))
    }.toString()

    private fun JSONObject.optStringOrNull(k: String): String? = if (isNull(k)) null else optString(k)

    fun decodeCommit(json: String): CommitResult = try {
        val o = JSONObject(json)
        val t = o.optJSONObject("timings") ?: JSONObject()
        CommitResult(
            status = runCatching { CommitStatus.valueOf(o.getString("status")) }.getOrDefault(CommitStatus.ERROR),
            asrText = o.optString("asr_text"),
            finalText = o.optString("final_text"),
            language = o.optStringOrNull("language"),
            timings = Timings(
                audioMs = t.optLong("audio_ms"), asrMs = t.optLong("asr_ms"), llmMs = t.optLong("llm_ms"),
                postprocessMs = t.optLong("postprocess_ms"), totalMs = t.optLong("total_ms"),
                droppedSamples = t.optLong("dropped_samples"),
            ),
            pressEnter = o.optBoolean("press_enter"),
            fallbackReason = o.optStringOrNull("fallback_reason"),
            truncated = o.optBoolean("truncated"),
            error = o.optStringOrNull("error"),
        )
    } catch (e: JSONException) {
        CommitResult(CommitStatus.ERROR, error = "bad commit json: ${e.message}")
    }

    fun decodeCandidates(json: String): List<LearnCandidate> = try {
        val a = JSONArray(json)
        (0 until a.length()).map { a.getJSONObject(it) }.map { LearnCandidate(it.getString("original"), it.getString("corrected")) }
    } catch (e: JSONException) {
        emptyList()
    }

    private fun dictionaryArray(entries: List<DictEntry>) = JSONArray().apply {
        for (e in entries) put(JSONObject().apply {
            put("word", e.word)
            if (e.replacement != null) put("replacement", e.replacement)
            put("isSnippet", e.isSnippet)
        })
    }

    /** Shared export format `[{word, replacement, isSnippet}]`. */
    fun encodeDictionary(entries: List<DictEntry>): String = dictionaryArray(entries).toString(2)

    /** @throws JSONException on malformed input (the import UI reports it). */
    fun decodeDictionary(json: String): List<DictEntry> {
        val a = JSONArray(json)
        return (0 until a.length()).mapNotNull { i ->
            val o = a.getJSONObject(i)
            val word = o.optString("word").trim()
            if (word.isEmpty()) return@mapNotNull null
            val repl = o.optStringOrNull("replacement")?.takeIf { it.isNotEmpty() }
            DictEntry(word, repl, o.optBoolean("isSnippet") && repl != null)
        }
    }
}
