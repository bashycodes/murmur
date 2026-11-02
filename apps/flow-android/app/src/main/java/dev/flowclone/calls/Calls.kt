package dev.flowclone.calls

import dev.flowclone.core.MeetingSegment
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * One call transcript (D46): what a picked recording said, as timestamped segments. Only the
 * text is kept, in app-private storage; the recording stays where it was, untouched.
 */
data class CallTranscript(
    val id: String,
    /** When it was transcribed (epoch ms). */
    val createdAt: Long,
    /** The recording's own time (its last-modified date), else [createdAt]. */
    val recordedAt: Long,
    /** The picked file's display name (call recordings name the contact; shown only here). */
    val sourceName: String,
    val durationMs: Long,
    /** `he` / `en`. */
    val language: String,
    val segments: List<MeetingSegment>,
)

/**
 * Transcripts in `files/calls/<id>.json`, one file each, newest first. Ids are generated here
 * ([newId]) and checked on every call, so nothing leaves [dir]. A file that does not parse is
 * skipped by [list] (never deleted).
 */
class CallsStore(private val dir: File) {
    @Synchronized fun list(): List<CallTranscript> =
        (dir.listFiles() ?: emptyArray())
            .filter { it.isFile && it.name.endsWith(JSON) && isValidId(it.name.removeSuffix(JSON)) }
            .mapNotNull { f -> runCatching { decode(f.readText(Charsets.UTF_8)) }.getOrNull() }
            .sortedWith(compareByDescending<CallTranscript> { it.createdAt }.thenByDescending { it.id })

    @Synchronized fun load(id: String): CallTranscript? {
        if (!isValidId(id)) return null
        val f = File(dir, id + JSON)
        return if (f.isFile) runCatching { decode(f.readText(Charsets.UTF_8)) }.getOrNull() else null
    }

    /** Written to a temp file and renamed, so a crash never leaves half a transcript. */
    @Synchronized fun save(t: CallTranscript) {
        require(isValidId(t.id)) { "bad transcript id" }
        dir.mkdirs()
        val tmp = File(dir, t.id + ".tmp")
        tmp.writeText(encode(t), Charsets.UTF_8)
        if (!tmp.renameTo(File(dir, t.id + JSON))) { tmp.delete(); throw java.io.IOException("could not save the transcript") }
    }

    /** The transcript only; the recording it came from is not ours to delete. */
    @Synchronized fun delete(id: String): Boolean = isValidId(id) && File(dir, id + JSON).delete()

    companion object {
        private const val JSON = ".json"
        private val ID = Regex("[0-9a-z-]{1,40}")

        fun isValidId(id: String) = ID.matches(id)

        /** Time-ordered and unique enough for one phone: `<ms base 36>-<random>`. */
        fun newId(now: Long, random: kotlin.random.Random = kotlin.random.Random.Default): String =
            now.toString(36) + "-" + random.nextInt(0, 36 * 36 * 36).toString(36)

        fun encode(t: CallTranscript): String = JSONObject().apply {
            put("v", 1)
            put("id", t.id); put("created_at", t.createdAt); put("recorded_at", t.recordedAt)
            put("source_name", t.sourceName); put("duration_ms", t.durationMs); put("language", t.language)
            put("segments", JSONArray().apply {
                for (s in t.segments) put(JSONObject().apply {
                    put("start_ms", s.startMs); put("end_ms", s.endMs); put("text", s.text)
                    s.language?.let { put("language", it) }
                    s.engine?.let { put("engine", it) }
                    s.error?.let { put("error", it) }
                })
            })
        }.toString()

        fun decode(json: String): CallTranscript {
            val o = JSONObject(json)
            val a = o.getJSONArray("segments")
            fun JSONObject.str(k: String): String? = if (has(k) && !isNull(k)) getString(k) else null
            return CallTranscript(
                id = o.getString("id"), createdAt = o.getLong("created_at"), recordedAt = o.optLong("recorded_at", o.getLong("created_at")),
                sourceName = o.optString("source_name"), durationMs = o.optLong("duration_ms"), language = o.optString("language"),
                segments = (0 until a.length()).map { i ->
                    val s = a.getJSONObject(i)
                    MeetingSegment(s.getLong("start_ms"), s.getLong("end_ms"), s.optString("text"), s.str("language"), s.str("engine"), s.str("error"))
                },
            )
        }
    }
}

/** Wording of the Calls page, kept pure for the JVM tests. */
object CallsView {
    const val NO_SPEECH = "(no speech)"

    /** The list's title: the first thing said. */
    fun firstLine(t: CallTranscript): String =
        t.segments.firstOrNull { it.error == null && it.text.isNotBlank() }?.text?.trim()?.lineSequence()?.first()?.trim() ?: NO_SPEECH

    /** `4:05`, `1:02:03`. */
    fun clock(ms: Long): String {
        val s = (ms / 1000).coerceAtLeast(0)
        val r = java.util.Locale.ROOT
        return if (s >= 3600) String.format(r, "%d:%02d:%02d", s / 3600, s / 60 % 60, s % 60) else String.format(r, "%d:%02d", s / 60, s % 60)
    }

    /** One line per segment: `[0:04] text`; a chunk that failed on every route says so. */
    fun text(t: CallTranscript): String = t.segments.joinToString("\n") { s ->
        "[${clock(s.startMs)}] " + (s.error?.let { "(not transcribed: $it)" } ?: s.text.trim())
    }.ifEmpty { NO_SPEECH }

    fun languageName(code: String): String = when (code) {
        "he" -> "Hebrew"
        "en" -> "English"
        else -> code
    }

    /** Percent caption while a job runs. */
    fun progressLabel(name: String, fraction: Float?, loading: Boolean): String = when {
        loading -> "Loading the speech model for $name…"
        fraction == null -> "Transcribing $name…"
        else -> "Transcribing $name · ${(fraction * 100).toInt()} %"
    }
}
