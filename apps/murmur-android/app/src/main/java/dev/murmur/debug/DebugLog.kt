package dev.murmur.debug

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
 * Field names are shared with apps/murmur-windows: ts, mode, route, fallback, rtt_ms, audio_ms,
 * endpoint_wait_ms, asr_ms, post_ms, total_ms, perceived_ms, target, insert, raw, text, error,
 * and since D22 wait_ms, segments, pre_asr_ms, client_ms, since D24 connect_ms, local_cold, local_load_ms, since D55 load_wait_ms (+ `kind` on every line, `stack` when there is an exception, `event` on non-dictation lines).
 * Android since 2026-10-23: engine (whisper|parakeet|server|cloud|mock), model (file / folder
 * name, server-reported or cloud model), language (sent to the engine; "auto" = detect), audio
 * (kept WAV file name, Developer "Keep last dictation audio (debug)"; else null).
 * Both since D27 (2026-10-24): style (style sent for the target's bucket), llm ("formatted" |
 * core fallback_reason such as llm_skipped_clean, a normal result | null = LLM off).
 * Since 2026-10-24 (phone-local-only finding): route_order (routes tried, in order), speech_ms
 * (core VAD speech) and peak_dbfs (peak level), so a blank_asr line is diagnosable.
 * D28 (OpenRouter): llm_engine (local | cloud | server; who ran the AI cleanup), llm_model,
 * llm_cost_usd, llm_ttft_ms, llm_generation_id, and stt_cost_usd (cloud speech-to-text cost).
 * D52: llm_provider (openrouter | custom) when llm_engine is cloud.
 * `llm: llm_skipped_clean` stays a normal result; a cloud refusal shows as `llm_error: <code>`.
 * D30 (AI cleanup first): cleanup_route (cloud | local | rules: what produced the text) and
 * cloud_fallback (why the cloud cleanup fell back), only when cloud cleanup was tried.
 * Since 2026-10-27, with cloud_fallback: guard_detail (why, in detail: the timeout, error or empty
 * answer), "<redacted>" for protected targets, like `text`. D59 (2026-11-06) removed the output
 * guard and with it rejected_text (the answer the guard did not use).
 * Since 2026-10-27: source (mic | wav: debug builds can feed a WAV file as the microphone).
 * D37 (2026-10-31): language_source (keyboard | phone | fixed: where `language` came from).
 * 2026-11-02 (local-cleanup-skipped): llm_off (why no AI cleanup was tried when `llm` is null).
 * D58 (2026-11-06): start_language / stop_language (the keyboard read at the start and at the stop;
 * stop_language null = not re-read); `language` is the one used (the stop one).
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
    /** whisper | parakeet | server | cloud | mock ([DebugEngine]); null when no engine ran. */
    val engine: String? = null,
    /** whisper file / Parakeet folder name, the server-reported ASR model, or the cloud model. */
    val model: String? = null,
    /** Language sent to the engine ("auto" = detect). */
    val language: String? = null,
    /** D37: where [language] came from: keyboard | phone | fixed (null = unknown). */
    val languageSource: String? = null,
    /** D58: the language read at the start ("auto" = none forced). */
    val startLanguage: String? = null,
    /** D58: the language read at the stop; null = not re-read (the language does not follow the keyboard). */
    val stopLanguage: String? = null,
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
    /** D22 `timings.wait_ms`: commit waited for a decode in flight. Null without a result. */
    val waitMs: Long? = null,
    /** D22 `timings.segments`: pieces sent to ASR (0 = segmentation off). */
    val segments: Long? = null,
    /** D22 `timings.pre_asr_ms`: ASR done while recording. */
    val preAsrMs: Long? = null,
    /** D22 `route.client_ms`: release → result on the phone, network included (router only). */
    val clientMs: Long? = null,
    /** D24: time the start spent connecting to the server (null = not tried: off, circuit open). */
    val connectMs: Long? = null,
    /** D24: the phone model was loaded on demand for this dictation (server unreachable, keep warm off). */
    val localCold: Boolean? = null,
    /** D24: how long that load took. */
    val localLoadMs: Long? = null,
    /** D55: recording started while the phone's speech model loaded; how long the dictation waited for it. */
    val loadWaitMs: Long? = null,
    /** Package of the field the dictation started in; redacted by [DebugRedact]. */
    val target: String? = null,
    val insert: InsertOutcome,
    val raw: String? = null,
    val text: String? = null,
    val error: String? = null,
    val stack: String? = null,
    /** "Keep last dictation audio (debug)": the WAV in `files/debug/audio/` ([DebugAudioStore]); null = none kept. */
    val audio: String? = null,
    /** D27: style sent for the target's bucket (formal | casual | very_casual | excited); null before a session. */
    val style: String? = null,
    /** D27: "formatted" (LLM text used), the core `fallback_reason` (llm_skipped_clean = nothing to do, llm_timeout, …), or null = LLM off. */
    val llm: String? = null,
    /**
     * 2026-11-02: why no AI cleanup was tried when [llm] is null and there was a transcript
     * ([dev.murmur.data.LlmOutcome.offReason]: ai_cleanup_off | no_phone_model |
     * phone_model_failed | no_engine_available); set by the app. Android only.
     */
    val llmOff: String? = null,
    /** 2026-10-24: routes the router may take, in try order ("client,server" = phone first); set by the app. */
    val routeOrder: String? = null,
    /** 2026-10-24: core VAD speech in the dictation (`timings.speech_ms`); null = VAD off / no result. */
    val speechMs: Long? = null,
    /** 2026-10-24: peak level of the dictation audio in dBFS (`timings.peak_dbfs`). */
    val peakDbfs: Int? = null,
    /** D28 `route.llm_engine`: local | cloud (OpenRouter) | server; null = no LLM step. */
    val llmEngine: String? = null,
    /** D52: the cloud cleanup route's service when [llmEngine] is cloud ("openrouter" | "custom"); set by the app. */
    val llmProvider: String? = null,
    val llmModel: String? = null,
    val llmCostUsd: Double? = null,
    /** D30 `route.cleanup_route` (cloud | local | rules) and `route.cloud_fallback`; null = AI cleanup not tried. */
    val cleanupRoute: String? = null,
    val cloudFallback: String? = null,
    /** 2026-10-27 `route.guard_detail`: why AI cleanup fell back. Redacted like [text]. */
    val fallbackDetail: String? = null,
    /** D28 `llm.ttft_ms` / `llm.generation_id` (OpenRouter's id, for a later cost lookup). */
    val llmTtftMs: Long? = null,
    val llmGenerationId: String? = null,
    /** D28: what the cloud speech-to-text of this dictation cost (`route.cost_usd`). */
    val sttCostUsd: Double? = null,
    /** Where the audio came from: "mic", or "wav" (debug builds: "WAV as microphone", tools/phone/play-wav.sh). */
    val source: String? = null,
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
        /** Command Mode: the AI gave no text (no AI, timeout, refused selection, …); nothing changed. */
        const val COMMAND_FAILED = "command_failed"
    }
}

/** The `engine` / `model` fields of a dictation line, from the route that served it. */
object DebugEngine {
    /**
     * [asr]: the on-device ASR that loaded; [serverModel]: the ASR model the server reported in
     * its last health probe; [routeModel]: the model in the commit's route (cloud).
     */
    fun of(route: String?, mock: Boolean, asr: dev.murmur.data.LoadedAsr?, serverModel: String?, routeModel: String?): Pair<String?, String?> = when {
        mock -> "mock" to null
        route == "local" -> asr?.engine?.wire to asr?.model
        route == "remote" -> "server" to serverModel
        route == "cloud" -> "cloud" to routeModel
        else -> null to null
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
            fallbackDetail = t.fallbackDetail?.let { REDACTED },
            insert = t.insert.copy(steps = emptyList()),
            audio = null,
        )
    }

    /** "Keep last dictation audio": never for the targets whose text is redacted. */
    fun keepsAudio(t: DictationTrace, protectedField: Boolean, protectedApp: Boolean) =
        !(protectedField || protectedApp || t.insert.outcome == InsertOutcome.TARGET_PROTECTED)

    /** Password-type input, a password node, or (API 34+) an accessibility-data-sensitive field. */
    fun isProtectedField(inputType: Int, nodePassword: Boolean, dataSensitive: Boolean) =
        nodePassword || dataSensitive || dev.murmur.policy.EditorInputType.isPassword(inputType)

    /** Restricted banking apps and the user's denylist (the apps the bubble stays hidden in). */
    fun isProtectedApp(pkg: String?, userDenylist: Set<String>) =
        dev.murmur.policy.BubbleVisibilityPolicy.isRestricted(pkg) || (pkg != null && pkg in userDenylist)

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

    /** D26: every line names the APK that wrote it (`app_version` 0.2.0, `build` abc1234 / abc1234-dirty). */
    @Volatile var build: dev.murmur.data.AppBuild = dev.murmur.data.AppVersion.current

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
        "app_version" to build.version,
        "build" to build.build,
        "mode" to t.mode,
        "route" to t.route,
        "fallback" to t.fallback,
        "engine" to t.engine,
        "model" to t.model,
        "language" to t.language,
        "rtt_ms" to t.rttMs,
        "audio_ms" to t.audioMs,
        "endpoint_wait_ms" to t.endpointWaitMs,
        "asr_ms" to t.asrMs,
        "post_ms" to t.postMs,
        "total_ms" to t.totalMs,
        "perceived_ms" to t.perceivedMs,
        "wait_ms" to t.waitMs,
        "segments" to t.segments,
        "pre_asr_ms" to t.preAsrMs,
        "client_ms" to t.clientMs,
        "connect_ms" to t.connectMs,
        "local_cold" to t.localCold,
        "local_load_ms" to t.localLoadMs,
        "load_wait_ms" to t.loadWaitMs,
        "style" to t.style,
        "llm" to t.llm,
        "llm_off" to t.llmOff,
        "route_order" to t.routeOrder,
        "speech_ms" to t.speechMs,
        "peak_dbfs" to t.peakDbfs,
        "llm_engine" to t.llmEngine,
        "llm_provider" to t.llmProvider,
        "llm_model" to t.llmModel,
        "llm_cost_usd" to t.llmCostUsd,
        "cleanup_route" to t.cleanupRoute,
        "cloud_fallback" to t.cloudFallback,
        dev.murmur.core.CoreJson.FALLBACK_DETAIL_KEY to t.fallbackDetail,
        "llm_ttft_ms" to t.llmTtftMs,
        "llm_generation_id" to t.llmGenerationId,
        "stt_cost_usd" to t.sttCostUsd,
        "source" to t.source,
        "language_source" to t.languageSource,
        "start_language" to t.startLanguage,
        "stop_language" to t.stopLanguage,
        "target" to t.target,
        "insert" to Raw(obj("outcome" to t.insert.outcome, "steps" to t.insert.steps)),
        "raw" to t.raw,
        "text" to t.text,
        "audio" to t.audio,
        "error" to t.error,
        "stack" to t.stack,
    )

    fun event(e: DebugEvent): String = obj(
        "kind" to "event",
        "ts" to ts(e.tsMs),
        "app_version" to build.version,
        "build" to build.build,
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
        val route = listOfNotNull(s("route") ?: "-", s("fallback")?.let { "($it)" }, s("engine"), s("model")).joinToString(" ") +
            (s("language")?.let { " · lang $it" + (s("language_source")?.let { src -> " ($src)" } ?: "") } ?: "") + (s("style")?.let { " · $it" } ?: "") +
            (dev.murmur.data.LlmOutcome.label(s("llm"))?.let { " · $it" } ?: "") +
            (dev.murmur.data.LlmOutcome.offLabel(s("llm_off"))?.let { " · $it" } ?: "") +
            (s("llm_engine")?.let { e -> " · AI " + (if (e == "cloud") (if (s("llm_provider") == "custom") "LiteLLM" else "OpenRouter") else e) + (s("llm_model")?.let { " ${it.substringAfter('/')}" } ?: "") } ?: "") +
            (s("cleanup_route")?.let { c -> " · text by $c" + (s("cloud_fallback")?.let { " (cloud: $it)" } ?: "") } ?: "") +
            (listOfNotNull(s("stt_cost_usd"), s("llm_cost_usd")).takeIf { it.isNotEmpty() }?.let { " · $" + it.sumOf { c -> c.toDoubleOrNull() ?: 0.0 }.let { v -> String.format(java.util.Locale.US, "%.4f", v) } } ?: "")
        val ms = listOfNotNull(
            s("perceived_ms")?.let { "wait $it ms" }, s("client_ms")?.let { "client $it" }, s("asr_ms")?.let { "asr $it" },
            s("pre_asr_ms")?.takeIf { it != "0" }?.let { "(+$it early, ${s("segments")} seg)" }, s("post_ms")?.let { "post $it" },
            s("endpoint_wait_ms")?.let { "endpoint $it" }, s("rtt_ms")?.let { "rtt $it" },
            s("connect_ms")?.let { "connect $it" }, s("local_load_ms")?.let { "phone model loaded in $it" },
            s("load_wait_ms")?.let { "recorded while the model loaded ($it ms)" },
            s("speech_ms")?.let { "speech $it ms" }, s("peak_dbfs")?.let { "peak $it dBFS" },
        ).joinToString(" · ")
        val text = s("text") ?: s("raw") ?: ""
        val err = s("error")?.let { "\n  error: $it" } ?: ""
        val rejected = s(dev.murmur.core.CoreJson.FALLBACK_DETAIL_KEY)?.let { "\n  AI cleanup fell back: $it" } ?: ""
        val src = s("source")?.takeIf { it != "mic" }?.let { " · source=$it" } ?: ""
        "$time  ${s("mode")}$src · $route · $outcome · $ms · ${s("target") ?: "-"}\n  “$text”$err$rejected"
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
        const val TAG = "MurmurDebug"
        /** logcat drops anything past ~4 KB per line; the file keeps the full entry. */
        const val LOGCAT_MAX = 3_800

        fun defaultRunner(): (Runnable) -> Unit {
            val ex = Executors.newSingleThreadExecutor { r -> Thread(r, "murmur-debug-log").apply { isDaemon = true; priority = Thread.MIN_PRIORITY } }
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
 * MurmurService's always-on `commit …` logcat line (tag MurmurService): timings, status and route
 * only. Never any text, with the debug log on or off (D21 keeps text in the MurmurDebug sink).
 */
object CommitLine {
    fun of(r: dev.murmur.core.CommitResult): String {
        val t = r.timings
        return "commit ${r.status} audio=${t.audioMs}ms asr=${t.asrMs}ms llm=${t.llmMs}ms total=${t.totalMs}ms dropped=${t.droppedSamples} " +
            "wait=${t.waitMs}ms segments=${t.segments} pre_asr=${t.preAsrMs}ms client=${r.route?.clientMs?.let { "${it}ms" } ?: "-"} " +
            "${r.fallbackReason ?: ""} ${r.emptyReason ?: ""} route=${r.route?.engine ?: "-"}/${r.route?.fallback ?: "-"}"
    }
}
