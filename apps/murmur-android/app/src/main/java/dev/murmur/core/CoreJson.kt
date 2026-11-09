package dev.murmur.core

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * JSON wire format between Kotlin and crates/murmur-ffi. Field names follow the Rust serde
 * definitions (snake_case; dictionary rows camelCase `isSnippet`; CommitStatus SCREAMING_SNAKE).
 * Uses the framework's org.json (no extra dependency; real impl added for JVM tests).
 */
object CoreJson {
    /** JSON key of [RouteInfo.fallbackDetail]: kept from before D59 (the output guard's detail), so old payloads still parse. */
    const val FALLBACK_DETAIL_KEY = "guard_detail"

    fun encode(c: ModelConfig): String = JSONObject().apply {
        put("asr_backend", c.mockText?.let { JSONObject().put("mock", JSONObject().put("text", it)) } ?: c.asrEngine.wire)
        put("asr_path", c.asrPath)
        put("llm_path", c.llmPath ?: JSONObject.NULL)
        put("llm_lib", c.llmLib ?: JSONObject.NULL)
        put("threads", c.threads)
        // Omitted when auto, like the core's serde (`skip_serializing_if = is_auto`).
        if (c.parakeetDecoding != ParakeetDecoding.AUTO) put("parakeet_decoding", c.parakeetDecoding.wire)
        // D33: omitted when empty, like the core's serde (old libraries see the same config).
        if (c.languages.isNotEmpty()) put("languages", JSONObject().apply {
            for ((code, m) in c.languages) put(code, JSONObject().apply {
                put("asr_backend", m.asrEngine.wire)
                put("asr_path", m.asrPath)
                if (m.parakeetDecoding != ParakeetDecoding.AUTO) put("parakeet_decoding", m.parakeetDecoding.wire)
            })
        })
        c.loadLanguage?.let { put("load_language", it) }
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
        put("vad", c.vad)
        put("hallucination_guard", c.hallucinationGuard)
        put("spoken_cleanup", c.spokenCleanup)
        put("llm_prefill", c.llmPrefill)
        // Omitted (not null) when unset, like the core's serde: old servers see the same config.
        c.endpointSilenceMs?.let { put("endpoint_silence_ms", it) }
        // D22: omitted when off; segment_min_ms / segment_soft_max_ms stay at the core defaults.
        c.segmentPauseMs?.let { put("segment_pause_ms", it) }
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

    internal fun JSONObject.optStringOrNull(k: String): String? = if (isNull(k)) null else optString(k)
    internal fun JSONObject.optLongOrNull(k: String): Long? = if (isNull(k)) null else optLong(k)
    internal fun JSONObject.optDoubleOrNull(k: String): Double? = if (isNull(k)) null else optDouble(k).takeIf { !it.isNaN() }

    /** Unknown keys / malformed rows are skipped (a newer core may add segment kinds). */
    private fun decodeSegments(a: JSONArray?): List<PasteSegment> {
        if (a == null) return emptyList()
        return (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            when {
                o.has("text") -> PasteSegment.Text(o.optString("text"))
                o.has("key") -> PasteKey.entries.firstOrNull { it.wire == o.optString("key") }?.let { PasteSegment.Key(it) }
                else -> null
            }
        }
    }

    fun decodeRoute(o: JSONObject) = RouteInfo(
        engine = o.optString("engine"),
        fallback = o.optStringOrNull("fallback"),
        server = o.optStringOrNull("server"),
        rttMs = o.optLongOrNull("rtt_ms"),
        queueMs = o.optLongOrNull("queue_ms"),
        serverCore = o.optStringOrNull("server_core"),
        provider = o.optStringOrNull("provider"),
        model = o.optStringOrNull("model"),
        billedAudioMs = o.optLongOrNull("billed_audio_ms"),
        costUsd = o.optDoubleOrNull("cost_usd"),
        costEstimated = if (o.isNull("cost_estimated")) null else o.optBoolean("cost_estimated"),
        clientMs = o.optLongOrNull("client_ms"),
        llmEngine = o.optStringOrNull("llm_engine"),
        llmModel = o.optStringOrNull("llm_model"),
        llmCostUsd = o.optDoubleOrNull("llm_cost_usd"),
        cleanupRoute = o.optStringOrNull("cleanup_route"),
        cloudFallback = o.optStringOrNull("cloud_fallback"),
        fallbackDetail = o.optStringOrNull(FALLBACK_DETAIL_KEY),
    )

    fun decodeCleanUsage(o: JSONObject?): CleanUsage? = o?.let {
        CleanUsage(
            engine = it.optString("engine", "local").ifEmpty { "local" }, model = it.optStringOrNull("model"),
            costUsd = it.optDoubleOrNull("cost_usd"), costEstimated = it.optBoolean("cost_estimated"),
            generationId = it.optStringOrNull("generation_id"), ttftMs = it.optLongOrNull("ttft_ms"),
            latencyMs = it.optLongOrNull("latency_ms"),
        )
    }

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
                // D22: omitted by the core when 0.
                waitMs = t.optLong("wait_ms"), segments = t.optLong("segments"), preAsrMs = t.optLong("pre_asr_ms"),
                // 2026-10-24: blank_asr diagnostics; omitted when unknown.
                speechMs = t.optLongOrNull("speech_ms"), peakDbfs = t.optLongOrNull("peak_dbfs")?.toInt(),
            ),
            pressEnter = o.optBoolean("press_enter"),
            fallbackReason = o.optStringOrNull("fallback_reason"),
            truncated = o.optBoolean("truncated"),
            error = o.optStringOrNull("error"),
            segments = decodeSegments(o.optJSONArray("segments")),
            plainText = o.optString("plain_text"),
            emptyReason = o.optStringOrNull("empty_reason"),
            route = o.optJSONObject("route")?.let(::decodeRoute),
            llm = decodeCleanUsage(o.optJSONObject("llm")),
        )
    } catch (e: JSONException) {
        CommitResult(CommitStatus.ERROR, error = "bad commit json: ${e.message}")
    }

    fun decodeCandidates(json: String): List<LearnCandidate> = try {
        val a = JSONArray(json)
        (0 until a.length()).map { a.getJSONObject(it) }.map { LearnCandidate(it.getString("original"), it.getString("corrected"), it.optBoolean("looks_like_name")) }
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
