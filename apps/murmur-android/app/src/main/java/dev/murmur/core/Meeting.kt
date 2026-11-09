package dev.flowclone.core

import org.json.JSONArray
import org.json.JSONObject

/**
 * Long-recording transcription (D44 `flow_core::meeting`, D46 Calls): a recording of any length
 * cut into 8-30 s chunks at pauses, each chunk one ordinary dictation on an engine the job owns.
 * Kotlin mirror of JNI_CONTRACT.md "Meeting transcription".
 */
data class MeetingSegment(
    /** Speech start / end, ms from the start of the recording. */
    val startMs: Long,
    val endMs: Long,
    val text: String,
    val language: String? = null,
    val engine: String? = null,
    /** Every route failed for this chunk (its text is empty). */
    val error: String? = null,
)

data class MeetingProgress(
    /** Audio handled (transcribed, or skipped as silence), ms. */
    val doneMs: Long = 0,
    val receivedMs: Long = 0,
    val totalMs: Long? = null,
    val segments: Int = 0,
) {
    /** 0..1 when the length is known. */
    val fraction: Float? get() = totalMs?.let { t -> if (t <= 0) 1f else (doneMs.toFloat() / t).coerceIn(0f, 1f) }
}

/** [kind]: `cancelled`, `failed` (chunks kept failing on every route), `bad_config`. */
class MeetingException(message: String, val kind: String?) : Exception(message) {
    val cancelled: Boolean get() = kind == CANCELLED

    companion object {
        const val CANCELLED = "cancelled"
    }
}

/**
 * One job. [push] and [finish] block while a chunk transcribes (a worker thread); [progress]
 * and [cancel] never wait and may come from any thread. [close] frees the job and its engine.
 */
interface MeetingJob {
    /** Length of the recording in 16 kHz samples (for [MeetingProgress.totalMs]). */
    fun setTotalSamples(samples: Long)

    /** 16 kHz mono PCM16, in order. The segments of the chunks this closed. @throws MeetingException */
    fun push(pcm: ShortArray, length: Int = pcm.size): List<MeetingSegment>

    /** End of the recording: the last segments. @throws MeetingException */
    fun finish(): List<MeetingSegment>

    fun progress(): MeetingProgress

    fun cancel()

    fun close()
}

object MeetingJson {
    fun decodeSegments(json: String): List<MeetingSegment> {
        val a = JSONArray(json)
        return (0 until a.length()).map { i ->
            val o = a.getJSONObject(i)
            MeetingSegment(
                startMs = o.optLong("start_ms"), endMs = o.optLong("end_ms"), text = o.optString("text"),
                language = o.optStringOrNull("language"), engine = o.optStringOrNull("engine"), error = o.optStringOrNull("error"),
            )
        }
    }

    fun decodeProgress(json: String): MeetingProgress {
        val o = runCatching { JSONObject(json) }.getOrDefault(JSONObject())
        return MeetingProgress(
            doneMs = o.optLong("done_ms"), receivedMs = o.optLong("received_ms"),
            totalMs = if (o.has("total_ms") && !o.isNull("total_ms")) o.optLong("total_ms") else null,
            segments = o.optInt("segments"),
        )
    }

    /** `MeetingParams` with the core defaults (pause 800 ms, chunks 8-30 s). */
    const val DEFAULT_PARAMS = "{}"

    private fun JSONObject.optStringOrNull(k: String): String? = if (has(k) && !isNull(k)) getString(k) else null
}

/** `nativeMeeting*` over a job handle (review C1's [HandleLease]: [close] never frees under a running call). */
class NativeMeeting internal constructor(handle: Long) : MeetingJob {
    private val lease = HandleLease(handle, { NativeFlowCore.nativeMeetingClose(it) })

    private fun closed(): Nothing = throw MeetingException("cancelled", MeetingException.CANCELLED)

    override fun setTotalSamples(samples: Long) = lease.use({}) { h -> NativeFlowCore.nativeMeetingSetTotal(h, samples) }

    override fun push(pcm: ShortArray, length: Int): List<MeetingSegment> = lease.use(::closed) { h ->
        BlockingCalls.check("nativeMeetingPush")
        segments(NativeFlowCore.nativeMeetingPush(h, pcm, length))
    }

    override fun finish(): List<MeetingSegment> = lease.use(::closed) { h ->
        BlockingCalls.check("nativeMeetingFinish")
        segments(NativeFlowCore.nativeMeetingFinish(h))
    }

    override fun progress(): MeetingProgress = lease.use({ MeetingProgress() }) { h -> MeetingJson.decodeProgress(NativeFlowCore.nativeMeetingProgress(h)) }

    override fun cancel() = lease.use({}) { h -> NativeFlowCore.nativeMeetingCancel(h) }

    override fun close() = lease.close()

    private fun segments(json: String?): List<MeetingSegment> {
        if (json != null) return MeetingJson.decodeSegments(json)
        val kind = runCatching { NativeFlowCore.nativeLastErrorKind() }.getOrNull()
        throw MeetingException(NativeFlowCore.nativeLastError() ?: "transcription failed", kind)
    }

    companion object {
        /**
         * A job over [engine] (a bare core from `loadModels`: on-device only). The engine's handle
         * moves to the job whatever happens; [engine] is unusable afterwards.
         * @throws CoreException
         */
        fun open(engine: NativeFlowCore, session: SessionConfig, paramsJson: String = MeetingJson.DEFAULT_PARAMS): MeetingJob {
            val h = engine.release()
            if (h == 0L) throw CoreException("Speech engine was unloaded, try again in a moment.", "local_not_ready")
            val m = try {
                NativeFlowCore.nativeMeetingOpen(h, CoreJson.encode(session), paramsJson)
            } catch (e: UnsatisfiedLinkError) {
                // A library from before D46 (the handle is lost with it; the process frees it on exit).
                throw CoreException("This build's speech library cannot transcribe recordings (rebuild the native libraries).", "bad_config")
            }
            if (m == 0L) throw NativeFlowCore.lastError("meeting_open failed")
            return NativeMeeting(m)
        }
    }
}
