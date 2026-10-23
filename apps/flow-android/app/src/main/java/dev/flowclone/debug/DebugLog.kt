package dev.flowclone.debug

import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/**
 * D21: opt-in, device-local debug log (Developer settings → "Debug log (includes transcripts)",
 * default off). One JSON line per dictation (and per notable non-dictation error) goes to logcat
 * tag [DebugLogger.TAG] and to `files/debug/dictations.jsonl` (last [DebugLogFile.MAX_ENTRIES]).
 *
 * Field names are shared with apps/flow-windows: ts, mode, route, fallback, rtt_ms, audio_ms,
 * endpoint_wait_ms, asr_ms, post_ms, total_ms, perceived_ms, target, insert, raw, text, error
 * (+ `kind` on every line, `stack` when there is an exception, `event` on non-dictation lines).
 *
 * Logging must never affect dictation: entries are queued (bounded, drop-oldest) and written
 * on one background thread; every failure is swallowed and counted.
 */

/** What one dictation did, as the platform-free controller saw it (before redaction). */
data class DictationTrace(
    /** Wall clock at record start (epoch ms). */
    val tsMs: Long,
    /** "tap" | "hold" */
    val mode: String,
    /** "local" | "remote" | "cloud"; null when no engine ran (start failed, cancelled early). */
    val route: String? = null,
    /** Router fallback reason (connect_failed, disconnected, circuit_open, …) or the start error kind. */
    val fallback: String? = null,
    val rttMs: Long? = null,
    val audioMs: Long? = null,
    /** Last speech → stop (shell-side energy tracker); null when no speech was heard. */
    val endpointWaitMs: Long? = null,
    val asrMs: Long? = null,
    /** llm_ms + postprocess_ms */
    val postMs: Long? = null,
    val totalMs: Long? = null,
    /** Stop / release → final outcome (text inserted, or the error shown): what the user waited. */
    val perceivedMs: Long? = null,
    /** Package of the field the dictation started in; redacted by [DebugRedact]. */
    val target: String? = null,
    val insert: InsertOutcome,
    val raw: String? = null,
    val text: String? = null,
    val error: String? = null,
    val stack: String? = null,
)

/** `insert` field: {"outcome": …, "steps": […]}. */
data class InsertOutcome(val outcome: String, val steps: List<String> = emptyList()) {
    companion object {
        const val INSERTED = "inserted"
        const val PASTE_FAILED = "paste_failed"
        const val TARGET_CHANGED = "target_changed"
        const val TARGET_PROTECTED = "target_protected"
        const val EMPTY = "empty"
        const val ERROR = "error"
        const val TIMEOUT = "timeout"
        const val START_FAILED = "start_failed"
        const val CANCELLED = "cancelled"
    }
}

/** A non-dictation entry: engine load / import / server / accessibility health. */
data class DebugEvent(val tsMs: Long, val event: String, val message: String?, val stack: String? = null)

object DebugRedact {
    const val REDACTED = "<redacted>"

    /**
     * Password / sensitive fields, banking apps and user-denylisted apps: the target and the
     * text are replaced by "<redacted>" (the timings stay; they are what the log is for).
     */
    fun apply(t: DictationTrace, protectedField: Boolean, protectedApp: Boolean): DictationTrace {
        val hide = protectedField || protectedApp || t.insert.outcome == InsertOutcome.TARGET_PROTECTED
        if (!hide) return t
        return t.copy(
            target = REDACTED,
            raw = t.raw?.let { REDACTED },
            text = t.text?.let { REDACTED },
            insert = t.insert.copy(steps = emptyList()),
        )
    }

    /** Password-type input, a password node, or (API 34+) an accessibility-data-sensitive field. */
    fun isProtectedField(inputType: Int, nodePassword: Boolean, dataSensitive: Boolean) =
        nodePassword || dataSensitive || dev.flowclone.policy.EditorInputType.isPassword(inputType)

    /** Restricted banking apps and the user's denylist (the apps the bubble stays hidden in). */
    fun isProtectedApp(pkg: String?, userDenylist: Set<String>) =
        dev.flowclone.policy.BubbleVisibilityPolicy.isBanking(pkg) || (pkg != null && pkg in userDenylist)

    /** Defence in depth: no secret (server token, cloud key) survives into a line, whatever path it took. */
    fun scrub(line: String, secrets: Collection<String>): String {
        var out = line
        for (s in secrets) if (s.length >= 8) out = out.replace(s, REDACTED)
        return out
    }
}

/** Ordered JSON lines (org.json's JSONObject has no key order; a log reads better with one). */
object DebugFormat {
    private val iso = ThreadLocal.withInitial {
        SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", Locale.US).apply { timeZone = TimeZone.getTimeZone("UTC") }
    }

    fun ts(ms: Long): String = iso.get()!!.format(Date(ms))

    private fun v(x: Any?): String = when (x) {
        null -> "null"
        is String -> JSONObject.quote(x)
        is Number, is Boolean -> x.toString()
        is List<*> -> x.joinToString(",", "[", "]") { v(it) }
        is Raw -> x.json
        else -> JSONObject.quote(x.toString())
    }

    private class Raw(val json: String)

    private fun obj(vararg kv: Pair<String, Any?>) = kv.joinToString(",", "{", "}") { (k, x) -> JSONObject.quote(k) + ":" + v(x) }

    fun dictation(t: DictationTrace): String = obj(
        "kind" to "dictation",
        "ts" to ts(t.tsMs),
        "mode" to t.mode,
        "route" to t.route,
        "fallback" to t.fallback,
        "rtt_ms" to t.rttMs,
        "audio_ms" to t.audioMs,
        "endpoint_wait_ms" to t.endpointWaitMs,
        "asr_ms" to t.asrMs,
        "post_ms" to t.postMs,
        "total_ms" to t.totalMs,
        "perceived_ms" to t.perceivedMs,
        "target" to t.target,
        "insert" to Raw(obj("outcome" to t.insert.outcome, "steps" to t.insert.steps)),
        "raw" to t.raw,
        "text" to t.text,
        "error" to t.error,
        "stack" to t.stack,
    )

    fun event(e: DebugEvent): String = obj(
        "kind" to "event",
        "ts" to ts(e.tsMs),
        "event" to e.event,
        "error" to e.message,
        "stack" to e.stack,
    )

    /** One row for the in-app viewer (newest first). Unparseable lines are shown as they are. */
    fun summary(line: String): String = runCatching {
        val o = JSONObject(line)
        fun s(k: String) = if (o.isNull(k)) null else o.opt(k)?.toString()
        val time = s("ts")?.substringAfter('T')?.removeSuffix("Z") ?: "?"
        if (o.optString("kind") == "event") return@runCatching "$time  ${s("event")}: ${s("error") ?: ""}"
        val outcome = o.optJSONObject("insert")?.optString("outcome") ?: "?"
        val route = listOfNotNull(s("route") ?: "-", s("fallback")?.let { "($it)" }).joinToString(" ")
        val ms = listOfNotNull(
            s("perceived_ms")?.let { "wait $it ms" }, s("asr_ms")?.let { "asr $it" }, s("post_ms")?.let { "post $it" },
            s("endpoint_wait_ms")?.let { "endpoint $it" }, s("rtt_ms")?.let { "rtt $it" },
        ).joinToString(" · ")
        val text = s("text") ?: s("raw") ?: ""
        val err = s("error")?.let { "\n  error: $it" } ?: ""
        "$time  ${s("mode")} · $route · $outcome · $ms · ${s("target") ?: "-"}\n  “$text”$err"
    }.getOrElse { line }
}

/** Where formatted lines go. Implementations may throw: [DebugLogger] swallows and counts. */
fun interface DebugSink {
    fun write(line: String)
}

/**
 * Ring buffer file: append one line per entry; when it holds more than [maxEntries], keep the
 * newest [maxEntries] (rewritten through a temp file + rename).
 */
class DebugLogFile(val file: File, private val maxEntries: Int = MAX_ENTRIES) : DebugSink {
    companion object {
        const val MAX_ENTRIES = 500
        fun inFilesDir(filesDir: File) = DebugLogFile(File(File(filesDir, "debug"), "dictations.jsonl"))
    }

    private var count = -1

    @Synchronized override fun write(line: String) {
        if (count < 0) count = readLines().size
        file.parentFile?.mkdirs()
        file.appendText(line.replace('\n', ' ') + "\n")
        count++
        if (count > maxEntries) {
            val keep = readLines().takeLast(maxEntries)
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeText(keep.joinToString("") { it + "\n" })
            if (!tmp.renameTo(file)) { file.writeText(tmp.readText()); tmp.delete() }
            count = keep.size
        }
    }

    @Synchronized fun readLines(): List<String> =
        if (file.isFile) file.readLines().filter { it.isNotBlank() } else emptyList()

    @Synchronized fun newestFirst(): List<String> = readLines().asReversed()

    @Synchronized fun size(): Int = readLines().size

    @Synchronized fun clear() {
        file.delete()
        count = 0
    }
}

/**
 * The single entry point. [enabled] is read per entry: when it is false nothing is formatted,
 * queued or written (the caller's timings-only lines are unaffected). Calls never block and
 * never throw: entries go into a bounded queue ([capacity], oldest dropped when full) drained
 * by [runner] (one background thread in the app, inline in tests).
 */
class DebugLogger(
    private val enabled: () -> Boolean,
    private val sink: DebugSink,
    private val logcat: (String) -> Unit,
    private val secrets: () -> Collection<String> = { emptyList() },
    private val runner: (Runnable) -> Unit = defaultRunner(),
    private val capacity: Int = 64,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    companion object {
        const val TAG = "FlowDebug"
        /** logcat drops anything past ~4 KB per line; the file keeps the full entry. */
        const val LOGCAT_MAX = 3_800

        fun defaultRunner(): (Runnable) -> Unit {
            val ex = Executors.newSingleThreadExecutor { r -> Thread(r, "flow-debug-log").apply { isDaemon = true; priority = Thread.MIN_PRIORITY } }
            return { r -> ex.execute(r) }
        }

        fun stackOf(t: Throwable?): String? = t?.let { e ->
            runCatching { e.stackTraceToString().lineSequence().take(25).joinToString("\n") }.getOrNull()
        }
    }

    private val queue = ArrayDeque<() -> String>()
    private var draining = false

    /** Entries dropped because the queue was full. */
    val dropped = AtomicInteger()
    /** Writes that failed (disk full, IO, serialisation, logcat). */
    val errors = AtomicInteger()

    fun isEnabled(): Boolean = try { enabled() } catch (_: Throwable) { false }

    fun dictation(t: DictationTrace) {
        if (!isEnabled()) return
        offer { DebugFormat.dictation(t) }
    }

    fun event(event: String, message: String?, error: Throwable? = null) {
        if (!isEnabled()) return
        try {
            val e = DebugEvent(clock(), event, message ?: error?.message, stackOf(error))
            offer { DebugFormat.event(e) }
        } catch (_: Throwable) { errors.incrementAndGet() }
    }

    private fun offer(build: () -> String) {
        try {
            val schedule: Boolean
            synchronized(queue) {
                if (queue.size >= capacity) { queue.removeFirst(); dropped.incrementAndGet() }
                queue.addLast(build)
                schedule = !draining
                draining = true
            }
            if (schedule) runner(Runnable { drain() })
        } catch (_: Throwable) {
            errors.incrementAndGet()
            synchronized(queue) { draining = false }
        }
    }

    private fun drain() {
        while (true) {
            val next = synchronized(queue) {
                queue.removeFirstOrNull() ?: run { draining = false; null }
            } ?: return
            write(next)
        }
    }

    private fun write(build: () -> String) {
        val line = try {
            DebugRedact.scrub(build(), secrets())
        } catch (_: Throwable) { errors.incrementAndGet(); return }
        try {
            logcat(if (line.length <= LOGCAT_MAX) line else line.take(LOGCAT_MAX) + " …(truncated; full entry in dictations.jsonl)")
        } catch (_: Throwable) { errors.incrementAndGet() }
        try { sink.write(line) } catch (_: Throwable) { errors.incrementAndGet() }
    }
}

/**
 * FlowService's always-on `commit …` logcat line (tag FlowService): timings, status and route
 * only. Never any text, with the debug log on or off (D21 keeps text in the FlowDebug sink).
 */
object CommitLine {
    fun of(r: dev.flowclone.core.CommitResult): String {
        val t = r.timings
        return "commit ${r.status} audio=${t.audioMs}ms asr=${t.asrMs}ms llm=${t.llmMs}ms total=${t.totalMs}ms dropped=${t.droppedSamples} " +
            "${r.fallbackReason ?: ""} ${r.emptyReason ?: ""} route=${r.route?.engine ?: "-"}/${r.route?.fallback ?: "-"}"
    }
}
