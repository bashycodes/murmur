package dev.flowclone.probe

import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder

/*
 * English 1 spike (docs/findings/2026-11-02-pixel-offline-dictation.md, PQ-a / PQ-d): the pure
 * parts of the probe screen, kept free of Android and ML Kit types so they unit-test on the JVM.
 * Throwaway: hybrid debug builds only (src/hybridDebug); normal dictation never reaches it.
 */

/** Names for the numeric codes ML Kit GenAI and the platform recognizer report. */
object ProbeCodes {
    /** ML Kit `FeatureStatus` (genai-common): checkStatus() result. */
    fun featureStatus(s: Int): String = when (s) {
        0 -> "UNAVAILABLE"
        1 -> "DOWNLOADABLE"
        2 -> "DOWNLOADING"
        3 -> "AVAILABLE"
        else -> "UNKNOWN($s)"
    }

    /** ML Kit `GenAiException.ErrorCode` (genai-common 1.0.0-beta3). */
    fun genAiError(code: Int): String = when (code) {
        0 -> "UNKNOWN"
        4 -> "REQUEST_PROCESSING_ERROR"
        7 -> "CANCELLED"
        8 -> "NOT_AVAILABLE"
        9 -> "BUSY"
        11 -> "RESPONSE_PROCESSING_ERROR"
        12 -> "REQUEST_TOO_LARGE"
        -100 -> "REQUEST_TOO_SMALL"
        15 -> "RESPONSE_GENERATION_ERROR"
        27 -> "PER_APP_BATTERY_USE_QUOTA_EXCEEDED"
        30 -> "BACKGROUND_USE_BLOCKED"
        501 -> "NOT_ENOUGH_DISK_SPACE"
        604 -> "NEEDS_SYSTEM_UPDATE"
        -101 -> "AICORE_INCOMPATIBLE"
        -102 -> "INVALID_INPUT_IMAGE"
        -103 -> "CACHE_PROCESSING_ERROR"
        else -> "CODE_$code"
    }

    /** `android.speech.SpeechRecognizer.ERROR_*` (API 31–35). */
    fun platformError(code: Int): String = when (code) {
        1 -> "ERROR_NETWORK_TIMEOUT"
        2 -> "ERROR_NETWORK"
        3 -> "ERROR_AUDIO"
        4 -> "ERROR_SERVER"
        5 -> "ERROR_CLIENT"
        6 -> "ERROR_SPEECH_TIMEOUT"
        7 -> "ERROR_NO_MATCH"
        8 -> "ERROR_RECOGNIZER_BUSY"
        9 -> "ERROR_INSUFFICIENT_PERMISSIONS"
        10 -> "ERROR_TOO_MANY_REQUESTS"
        11 -> "ERROR_SERVER_DISCONNECTED"
        12 -> "ERROR_LANGUAGE_NOT_SUPPORTED"
        13 -> "ERROR_LANGUAGE_UNAVAILABLE"
        14 -> "ERROR_CANNOT_CHECK_SUPPORT"
        15 -> "ERROR_CANNOT_LISTEN_TO_DOWNLOAD_EVENTS"
        else -> "ERROR_$code"
    }

    fun mode(advanced: Boolean) = if (advanced) "advanced" else "basic"

    /** A recognizer's best non-blank hypothesis ("" if none): some lists lead with an empty string. */
    fun firstNonBlank(hypotheses: List<String?>): String = hypotheses.firstOrNull { !it.isNullOrBlank() }?.trim().orEmpty()

    /**
     * `checkRecognitionSupport` → one word for [lang]: installed, pending (download queued),
     * supported (on-device, not installed), online (only online), or unsupported.
     * Tags compare case-insensitively with `_` = `-` (services report both "en-US" and "en_us").
     */
    fun platformSupport(
        lang: String,
        installed: List<String>,
        pending: List<String>,
        supported: List<String>,
        online: List<String>,
    ): String {
        fun norm(t: String) = t.trim().replace('_', '-').lowercase()
        val l = norm(lang)
        fun has(xs: List<String>) = xs.any { norm(it) == l }
        return when {
            has(installed) -> "installed"
            has(pending) -> "pending"
            has(supported) -> "supported"
            has(online) -> "online"
            else -> "unsupported"
        }
    }

    /** Download line: "started 120.0 MB", "45 % (54.0 / 120.0 MB)", or bytes when the total is unknown. */
    fun downloadProgress(done: Long, total: Long?): String {
        fun mb(b: Long) = String.format(java.util.Locale.US, "%.1f MB", b / 1_000_000.0)
        return if (total != null && total > 0) {
            val pct = (done * 100 / total).coerceIn(0, 100)
            "$pct % (${mb(done).removeSuffix(" MB")} / ${mb(total)})"
        } else mb(done)
    }
}

/** Run timing: every value is ms on one monotonic clock; null when the event never happened. */
data class ProbeTiming(
    val startAt: Long,
    val firstPartialAt: Long? = null,
    val audioEndAt: Long? = null,
    val lastFinalAt: Long? = null,
    val completedAt: Long? = null,
) {
    /** End of audio → final text. Negative when the final text came before the audio ended (streaming). */
    val latencyMs: Long? get() = if (audioEndAt != null && lastFinalAt != null) lastFinalAt - audioEndAt else null
    /** End of audio → the engine said it is done (Completed / onResults). */
    val completedMs: Long? get() = if (audioEndAt != null && completedAt != null) completedAt - audioEndAt else null
    val firstPartialMs: Long? get() = firstPartialAt?.let { it - startAt }
}

/** One probe run, one line of `files/probe-english.jsonl` (and logcat tag EnglishProbe). */
data class ProbeRecord(
    val tsMs: Long,
    val appBuild: String,
    /** "genai" or "platform". */
    val engine: String,
    /** genai: "advanced" / "basic"; platform: null. */
    val mode: String?,
    /** "mic", "clip_tts", "clip_file". */
    val source: String,
    /** "screen" (probe activity buttons) or "overlay" (probe overlay over another app). */
    val trigger: String,
    /** Was the probe activity resumed (our app on top) when the run started? */
    val probeOnTop: Boolean,
    /** genai: FeatureStatus name; platform: en-US support word (or null if not checked). */
    val status: String?,
    /** "ok", "empty", "error", "cancelled". */
    val outcome: String,
    val text: String,
    val partials: Int,
    val timing: ProbeTiming,
    val audioMs: Long?,
    val errorCode: Int? = null,
    val error: String? = null,
    val extras: Map<String, Any?> = emptyMap(),
) {
    fun toJsonLine(): String {
        val o = JSONObject()
        o.put("ts_ms", tsMs)
        o.put("build", appBuild)
        o.put("engine", engine)
        o.put("mode", mode ?: JSONObject.NULL)
        o.put("source", source)
        o.put("trigger", trigger)
        o.put("probe_on_top", probeOnTop)
        o.put("status", status ?: JSONObject.NULL)
        o.put("outcome", outcome)
        o.put("text", text)
        o.put("partials", partials)
        o.put("first_partial_ms", timing.firstPartialMs ?: JSONObject.NULL)
        o.put("latency_ms", timing.latencyMs ?: JSONObject.NULL)
        o.put("completed_ms", timing.completedMs ?: JSONObject.NULL)
        o.put("audio_ms", audioMs ?: JSONObject.NULL)
        o.put("error_code", errorCode ?: JSONObject.NULL)
        o.put("error", error ?: JSONObject.NULL)
        for ((k, v) in extras.toSortedMap()) o.put(k, jsonValue(v))
        return o.toString()
    }

    private fun jsonValue(v: Any?): Any = when (v) {
        null -> JSONObject.NULL
        is List<*> -> org.json.JSONArray(v.map { jsonValue(it) })
        else -> v
    }

    companion object {
        fun outcome(text: String, errorCode: Int?, cancelled: Boolean): String = when {
            cancelled -> "cancelled"
            errorCode != null && text.isBlank() -> "error"
            text.isBlank() -> "empty"
            else -> "ok"
        }
    }
}

/** Probe audio: any PCM16 WAV → 16 kHz mono PCM16, the format both engines' PFD inputs take. */
object ProbePcm {
    const val RATE = 16_000

    data class Wav(val pcm: ShortArray, val rate: Int, val channels: Int)

    /** Reads a PCM16 WAV of any rate and channel count (interleaved). Throws IllegalArgumentException saying why not. */
    fun readWav(bytes: ByteArray): Wav {
        val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        fun tag(at: Int) = String(bytes, at, 4, Charsets.US_ASCII)
        require(bytes.size >= 12 && tag(0) == "RIFF" && tag(8) == "WAVE") { "not a RIFF/WAVE file" }
        var rate = 0; var channels = 0
        var p = 12
        while (p + 8 <= bytes.size) {
            val id = tag(p)
            val size = b.getInt(p + 4).toLong() and 0xffffffffL
            val body = p + 8
            when (id) {
                "fmt " -> {
                    require(size >= 16) { "fmt chunk too short" }
                    val format = b.getShort(body).toInt() and 0xffff
                    channels = b.getShort(body + 2).toInt()
                    rate = b.getInt(body + 4)
                    val bits = b.getShort(body + 14).toInt()
                    require(format == 1 || format == 0xFFFE) { "not PCM (format $format)" }
                    require(bits == 16) { "$bits-bit; needs 16-bit PCM" }
                    require(channels >= 1 && rate > 0) { "bad fmt ($channels ch, $rate Hz)" }
                }
                "data" -> {
                    require(rate > 0) { "no fmt chunk before the data" }
                    // TTS engines sometimes write a 0 / 0xFFFFFFFF data size while streaming: take the rest.
                    val avail = (bytes.size - body).toLong()
                    val len = if (size == 0L || size > avail) avail else size
                    val n = (len / 2).toInt()
                    return Wav(ShortArray(n) { b.getShort(body + 2 * it) }, rate, channels)
                }
            }
            p = (body + size + (size and 1)).toInt()
            if (p < 0) break
        }
        throw IllegalArgumentException(if (rate > 0) "no data chunk" else "no fmt chunk")
    }

    fun downmix(pcm: ShortArray, channels: Int): ShortArray {
        if (channels <= 1) return pcm
        val frames = pcm.size / channels
        return ShortArray(frames) { f ->
            var sum = 0
            for (c in 0 until channels) sum += pcm[f * channels + c]
            (sum / channels).toShort()
        }
    }

    /** Linear-interpolation resample (good enough for speech into an ASR probe). */
    fun resample(pcm: ShortArray, from: Int, to: Int = RATE): ShortArray {
        if (from == to || pcm.isEmpty()) return pcm
        val outN = ((pcm.size.toLong() * to) / from).toInt().coerceAtLeast(1)
        val step = from.toDouble() / to
        return ShortArray(outN) { i ->
            val x = i * step
            val i0 = x.toInt().coerceAtMost(pcm.size - 1)
            val i1 = (i0 + 1).coerceAtMost(pcm.size - 1)
            val f = x - i0
            (pcm[i0] * (1 - f) + pcm[i1] * f).toInt().coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
        }
    }

    fun to16kMono(w: Wav): ShortArray = resample(downmix(w.pcm, w.channels), w.rate)

    fun toLeBytes(pcm: ShortArray, from: Int = 0, to: Int = pcm.size): ByteArray {
        val b = ByteBuffer.allocate((to - from) * 2).order(ByteOrder.LITTLE_ENDIAN)
        for (i in from until to) b.putShort(pcm[i])
        return b.array()
    }

    fun durationMs(samples: Int, rate: Int = RATE): Long = samples * 1000L / rate
}

/**
 * Folder batch (parity scorecard): which files of a set folder run, their clip ids, and the one
 * JSON line each (file, engine) run appends to the batch results file. Pure, so it unit-tests.
 */
object ProbeBatchPlan {
    const val DIR = "nano-batch"
    const val RESULTS = "results"

    /** The WAVs of a set folder, by name (the clip ids sort like the eval's refs). */
    fun wavs(names: List<String>): List<String> =
        names.filter { it.endsWith(".wav", ignoreCase = true) && !it.startsWith(".") }.sorted()

    /** Clip id = file name without ".wav" (the id score.py's refs use). */
    fun clipId(fileName: String): String =
        if (fileName.endsWith(".wav", ignoreCase = true)) fileName.dropLast(4) else fileName

    /** Set folders to run: [only] if given (and present), else every folder but the results one. */
    fun sets(dirs: List<String>, only: String): List<String> {
        val all = dirs.filter { it != RESULTS && !it.startsWith(".") }.sorted()
        val want = only.split(',').map { it.trim() }.filter { it.isNotEmpty() }
        return if (want.isEmpty()) all else want.filter { it in all }
    }

    /** Results file name for one batch session; never overwrites an earlier session's file. */
    fun resultsName(startedAtUtc: String): String = "batch-$startedAtUtc.jsonl"

    /** No result this long after a clip's audio should have ended → cancel that run. */
    fun watchdogMs(audioMs: Long, realTime: Boolean): Long = (if (realTime) audioMs else 0L) + 60_000L

    fun line(set: String, id: String, r: ProbeRecord): String {
        val o = JSONObject()
        o.put("set", set)
        o.put("id", id)
        o.put("engine", r.engine)
        o.put("mode", r.mode ?: JSONObject.NULL)
        o.put("text", r.text)
        o.put("first_partial_ms", r.timing.firstPartialMs ?: JSONObject.NULL)
        o.put("final_ms_after_audio_end", r.timing.latencyMs ?: JSONObject.NULL)
        o.put("completed_ms_after_audio_end", r.timing.completedMs ?: JSONObject.NULL)
        o.put("outcome", r.outcome)
        o.put("error", r.error ?: JSONObject.NULL)
        o.put("error_code", r.errorCode ?: JSONObject.NULL)
        o.put("audio_ms", r.audioMs ?: JSONObject.NULL)
        o.put("partials", r.partials)
        o.put("status", r.status ?: JSONObject.NULL)
        o.put("build", r.appBuild)
        o.put("ts_ms", r.tsMs)
        for ((k, v) in r.extras.toSortedMap()) o.put(k, if (v is List<*>) org.json.JSONArray(v) else v ?: JSONObject.NULL)
        return o.toString()
    }
}
