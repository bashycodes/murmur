package dev.murmur.calls

import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** One call recording as the phone's media index lists it (D49). Only read, never changed. */
data class Recording(
    /** MediaStore `_id`. */
    val id: Long,
    /** `content://media/…/audio/media/<id>`. */
    val uri: String,
    val name: String,
    /** When it was recorded (epoch ms). */
    val recordedAt: Long,
    /** MediaStore `generation_added`: rises with every new file on the volume. */
    val generation: Long,
)

/**
 * Where new call recordings come from (D49). The app's is [MediaStoreRecordings]; tests use a fake.
 * Both calls are one query each: nothing watches the disk.
 */
interface RecordingSource {
    /** The volume's generation now: everything added later has a larger `generation_added`. */
    fun generation(): Long

    /** Call recordings (finished, non-empty) added after generation [after], oldest first. @throws SecurityException without read access */
    fun since(after: Long): List<Recording>
}

/**
 * D49 auto-transcribe: the owner's opt-in and the queue of new call recordings, in one JSON file
 * (app-private, so the opt-in survives a reboot). Turning it on stores the media generation of
 * that moment ([floor]); [scan] queues only recordings added after it, so the backlog from
 * before is never transcribed. Every recording seen is remembered by its MediaStore id with its
 * outcome, so nothing is transcribed twice (deleting the transcript does not bring it back).
 * The recordings themselves are only read.
 */
class AutoQueue(private val file: File, private val source: RecordingSource, private val now: () -> Long = System::currentTimeMillis) {
    /** RUNNING: handed to a job; read back as QUEUED after a restart (the job died with the process). */
    enum class Status { QUEUED, RUNNING, DONE, FAILED, DISMISSED }

    data class Item(
        val rec: Recording,
        val status: Status,
        /** Times a job started on it; a job that never finishes (crash, killed) counts. */
        val attempts: Int = 0,
        val transcriptId: String? = null,
        val error: String? = null,
        val at: Long = 0,
    )

    private var enabled = false
    private var floor = 0L
    private var scanError: String? = null
    private val items = LinkedHashMap<Long, Item>()

    init { load() }

    @get:Synchronized val isEnabled: Boolean get() = enabled
    /** Why the last [scan] could not read the recordings (no permission), or null. */
    @get:Synchronized val lastScanError: String? get() = scanError

    /** On from now: recordings added after this moment are queued; already on keeps its floor. */
    @Synchronized
    fun enable() {
        if (enabled) return
        floor = source.generation()
        enabled = true
        items.clear(); scanError = null
        save()
    }

    /** Off: nothing more is queued, and what was waiting is dropped. */
    @Synchronized
    fun disable() {
        enabled = false
        items.clear(); scanError = null
        save()
    }

    /** Queues recordings that are new since the opt-in and not seen before; returns how many. */
    @Synchronized
    fun scan(): Int {
        if (!enabled) return 0
        val found = try {
            source.since(floor)
        } catch (e: SecurityException) {
            if (scanError == null) { scanError = NO_ACCESS; save() }
            return 0
        }
        var added = 0
        for (r in found) {
            if (r.generation <= floor || items.containsKey(r.id)) continue
            items[r.id] = Item(r, Status.QUEUED, at = now())
            added++
        }
        if (added > 0 || scanError != null) { scanError = null; prune(); save() }
        return added
    }

    /** The oldest queued recording as a job request (counts an attempt), or null. */
    @Synchronized
    fun next(language: String): CallRequest? {
        while (true) {
            val it = items.values.filter { it.status == Status.QUEUED }.minByOrNull { it.rec.generation } ?: return null
            if (it.attempts >= MAX_ATTEMPTS) {
                items[it.rec.id] = it.copy(status = Status.FAILED, error = "Stopped $MAX_ATTEMPTS times before it finished", at = now())
                save()
                continue
            }
            items[it.rec.id] = it.copy(status = Status.RUNNING, attempts = it.attempts + 1)
            save()
            return CallRequest(it.rec.uri, it.rec.name, it.rec.recordedAt, language, autoId = it.rec.id)
        }
    }

    /** A job on an auto recording ended ([CallJobs] onEnd). */
    @Synchronized
    fun finished(id: Long, state: CallJobState, stop: CallStop?) {
        val it = items[id] ?: return
        items[id] = when (state) {
            is CallJobState.Done -> it.copy(status = Status.DONE, transcriptId = state.id, error = null, at = now())
            is CallJobState.Failed -> it.copy(status = Status.FAILED, error = state.message, at = now())
            is CallJobState.Cancelled -> when (stop) {
                // A manual job took its turn: back in the queue, the attempt not counted.
                CallStop.PREEMPTED -> it.copy(status = Status.QUEUED, attempts = (it.attempts - 1).coerceAtLeast(0))
                // Android stopped it: try again later (counted, so a file that never fits gives up).
                CallStop.SYSTEM -> it.copy(status = Status.QUEUED)
                else -> it.copy(status = Status.FAILED, error = "Cancelled", at = now())
            }
            else -> return
        }
        save()
    }

    @Synchronized fun pending(): Int = items.values.count { it.status == Status.QUEUED }

    /** Failed recordings, newest first (shown on the Calls page). */
    @Synchronized fun failures(): List<Item> = items.values.filter { it.status == Status.FAILED }.sortedByDescending { it.rec.generation }

    @Synchronized
    fun retry(id: Long) {
        val it = items[id]?.takeIf { it.status == Status.FAILED } ?: return
        items[id] = it.copy(status = Status.QUEUED, attempts = 0, error = null)
        save()
    }

    @Synchronized
    fun dismiss(id: Long) {
        val it = items[id]?.takeIf { it.status == Status.FAILED } ?: return
        items[id] = it.copy(status = Status.DISMISSED)
        save()
    }

    /**
     * Keeps the file small: past [KEEP] finished recordings the oldest are forgotten and the
     * floor moves above them, so they still are never queued again.
     */
    private fun prune() {
        val finished = items.values.filter { it.status == Status.DONE || it.status == Status.DISMISSED }.sortedBy { it.rec.generation }
        val drop = finished.size - KEEP
        if (drop <= 0) return
        for (g in finished.take(drop)) {
            items.remove(g.rec.id)
            floor = maxOf(floor, g.rec.generation)
        }
    }

    private fun load() {
        if (!file.isFile) return
        runCatching {
            val o = JSONObject(file.readText(Charsets.UTF_8))
            enabled = o.optBoolean("enabled")
            floor = o.optLong("floor")
            scanError = if (o.has("scan_error") && !o.isNull("scan_error")) o.getString("scan_error") else null
            val a = o.optJSONArray("items") ?: JSONArray()
            for (i in 0 until a.length()) {
                val x = a.getJSONObject(i)
                val r = Recording(x.getLong("id"), x.getString("uri"), x.optString("name"), x.optLong("recorded_at"), x.getLong("generation"))
                val st = Status.valueOf(x.getString("status")).let { s -> if (s == Status.RUNNING) Status.QUEUED else s }
                items[r.id] = Item(
                    r, st, x.optInt("attempts"),
                    x.optString("transcript").ifEmpty { null }, x.optString("error").ifEmpty { null }, x.optLong("at"),
                )
            }
        }
    }

    /** Temp file + rename, like [CallsStore]. */
    private fun save() {
        val o = JSONObject().apply {
            put("v", 1); put("enabled", enabled); put("floor", floor)
            scanError?.let { put("scan_error", it) }
            put("items", JSONArray().apply {
                for (it in items.values) put(JSONObject().apply {
                    put("id", it.rec.id); put("uri", it.rec.uri); put("name", it.rec.name)
                    put("recorded_at", it.rec.recordedAt); put("generation", it.rec.generation)
                    put("status", it.status.name); put("attempts", it.attempts); put("at", it.at)
                    it.transcriptId?.let { t -> put("transcript", t) }
                    it.error?.let { e -> put("error", e) }
                })
            })
        }
        file.parentFile?.mkdirs()
        val tmp = File(file.path + ".tmp")
        tmp.writeText(o.toString(), Charsets.UTF_8)
        if (!tmp.renameTo(file)) tmp.delete()
    }

    companion object {
        const val MAX_ATTEMPTS = 3
        const val KEEP = 500
        const val NO_ACCESS = "Murmur can't read the call recordings: allow Music and audio for it."
    }
}
