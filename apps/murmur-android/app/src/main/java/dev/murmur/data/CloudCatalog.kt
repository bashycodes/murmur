package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject

/**
 * D28 model picker data: the rows `nativeCloudCatalog` returns (flow-cloud `PickerRow`: a
 * trimmed `ModelInfo` + `recommended`, `price_label`, `badges`) and the `nativeRefreshCloudCatalog`
 * status. Rust owns the catalogue (Windows shows the same list); this only decodes and words it.
 */
object CloudCatalog {
    enum class Use(val wire: String) { STT("stt"), CLEANUP("cleanup") }

    data class Row(
        val id: String,
        val name: String,
        val contextLength: Int = 0,
        /** transcriptions | chat_audio | null (cleanup models) */
        val sttPath: String? = null,
        val zdr: Boolean = false,
        val fast: Boolean = false,
        val free: Boolean = false,
        val recommended: Boolean = false,
        /** "$0.04/h audio", "≈ $0.035/h + output", "≈ $0.0002 per dictation", "free", "price: see usage". */
        val priceLabel: String = "",
        val badges: List<String> = emptyList(),
        /** `/models/user`: false = the account's OpenRouter privacy settings exclude it; null = not checked. */
        val allowedByAccount: Boolean? = null,
        val reasoningMandatory: Boolean = false,
    ) {
        val chatAudio: Boolean get() = sttPath == "chat_audio"
    }

    /** Filter chips; `nativeCloudCatalog`'s `filtersJson`. ZDR only is on by default (as in Rust). */
    data class Filters(
        val zdrOnly: Boolean = true,
        val fastOnly: Boolean = false,
        val freeOnly: Boolean = false,
        /** STT only: transcriptions | chat_audio | null = both. */
        val sttPath: String? = null,
    ) {
        fun toJson(): String = JSONObject().apply {
            put("zdr_only", zdrOnly); put("fast_only", fastOnly); put("free_only", freeOnly)
            put("stt_path", sttPath ?: JSONObject.NULL)
        }.toString()
    }

    /** `{source, fetched_at_ms, snapshot_date, models, stt, cleanup, error?}`. */
    data class Status(
        /** Live | Cache | Baked */
        val source: String = "Baked",
        val fetchedAtMs: Long = 0,
        val snapshotDate: String? = null,
        val models: Int = 0,
        val stt: Int = 0,
        val cleanup: Int = 0,
        /** A `Failure` code when the refresh failed (the previous list stays). */
        val error: String? = null,
    )

    fun decodeRows(json: String?): List<Row> = runCatching {
        val a = JSONArray(json ?: "[]")
        (0 until a.length()).mapNotNull { i ->
            val o = a.optJSONObject(i) ?: return@mapNotNull null
            val id = o.optString("id").ifEmpty { return@mapNotNull null }
            val badges = o.optJSONArray("badges")?.let { b -> (0 until b.length()).map { b.optString(it) } }.orEmpty()
            Row(
                id = id,
                name = o.optString("name").ifEmpty { id },
                contextLength = o.optInt("context_length"),
                sttPath = if (o.isNull("stt_path")) null else o.optString("stt_path").ifEmpty { null },
                zdr = o.optBoolean("zdr"),
                fast = (o.optJSONArray("fast_zdr_providers")?.length() ?: 0) > 0,
                free = o.optBoolean("free"),
                recommended = o.optBoolean("recommended"),
                priceLabel = o.optString("price_label"),
                badges = badges,
                allowedByAccount = if (!o.has("allowed_by_account") || o.isNull("allowed_by_account")) null else o.optBoolean("allowed_by_account"),
                reasoningMandatory = o.optBoolean("reasoning_mandatory"),
            )
        }
    }.getOrDefault(emptyList())

    fun decodeStatus(json: String?): Status = runCatching {
        val o = JSONObject(json ?: "{}")
        Status(
            source = o.optString("source", "Baked").ifEmpty { "Baked" },
            fetchedAtMs = o.optLong("fetched_at_ms"),
            snapshotDate = if (o.isNull("snapshot_date")) null else o.optString("snapshot_date").ifEmpty { null },
            models = o.optInt("models"), stt = o.optInt("stt"), cleanup = o.optInt("cleanup"),
            error = if (o.isNull("error")) null else o.optString("error").ifEmpty { null },
        )
    }.getOrDefault(Status(error = "error"))

    /** Row subtitle: "openai/whisper-large-v3-turbo · $0.012/h audio · 131k context". */
    fun meta(r: Row, use: Use): String = listOfNotNull(
        r.id,
        r.priceLabel.ifEmpty { null },
        if (use == Use.CLEANUP && r.contextLength > 0) contextLabel(r.contextLength) else null,
    ).joinToString(" · ")

    fun contextLabel(n: Int): String = when {
        n >= 1_000_000 -> "${n / 1_000_000}M context"
        n >= 1_000 -> "${n / 1_000}k context"
        else -> "$n context"
    }

    /** Short badges on the row's chip ("ZDR · fast · may paraphrase"). */
    fun badgeLine(r: Row): String = r.badges.joinToString(" · ") {
        when (it) {
            "chat model, may paraphrase" -> "may paraphrase"
            "free (20/min, may log)" -> "free, may log"
            "not allowed by your OpenRouter privacy settings" -> "blocked by account"
            else -> it
        }
    }

    /** Where the list came from: "Live list · refreshed 10:42", "Cached list · 3 h old", "Offline list (2026-10-24)". */
    fun sourceLine(s: Status, nowMs: Long, clock: (Long) -> String): String {
        val base = when (s.source) {
            "Live" -> "Live list · refreshed ${clock(s.fetchedAtMs)}"
            "Cache" -> "Cached list · " + age(nowMs - s.fetchedAtMs)
            else -> "Offline list" + (s.snapshotDate?.let { " (snapshot $it)" } ?: "")
        }
        val counts = if (s.models > 0) " · ${s.stt} speech, ${s.cleanup} text models" else ""
        val err = s.error?.let { " · refresh failed (${CloudText.hint(it) ?: it})" } ?: ""
        return base + counts + err
    }

    private fun age(ms: Long): String = when {
        ms < 60_000 -> "just now"
        ms < 3_600_000 -> "${ms / 60_000} min old"
        else -> "${ms / 3_600_000} h old"
    }

    /** The model row on the OpenRouter page. */
    fun modelSummary(id: String, sttPath: String? = null): String =
        id + when (sttPath) { "chat_audio" -> " · chat model (may paraphrase)"; "transcriptions" -> " · transcription"; else -> "" }

    /** The one-time warning when a chat-audio model is picked for speech-to-text (spec §2.4). */
    const val CHAT_AUDIO_WARNING = "This is a chat model, not a transcription model: it may answer, translate or reword " +
        "instead of writing down what you said. Answers that do not look like a transcript are rejected and the next route is used."
}
