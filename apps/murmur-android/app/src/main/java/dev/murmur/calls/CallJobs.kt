package dev.murmur.calls

import dev.murmur.core.CoreException
import dev.murmur.core.MeetingException
import dev.murmur.core.MeetingJob
import dev.murmur.core.MeetingSegment
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * What the Calls page asks for: one picked recording and the language it is in. [autoId]: the
 * MediaStore id of a new call recording queued by auto-transcribe (D49); null for a picked file.
 */
data class CallRequest(val uri: String, val name: String, val recordedAt: Long, val language: String, val autoId: Long? = null) {
    val auto: Boolean get() = autoId != null
}

/** Why a job was cancelled (D49): the owner asked, a manual job took its place, or Android stopped us. */
enum class CallStop { OWNER, PREEMPTED, SYSTEM }

sealed interface CallJobState {
    data object Idle : CallJobState
    /** [loading]: the job's own speech model is loading; [fraction] null while the length is unknown. */
    data class Running(val name: String, val loading: Boolean, val fraction: Float?) : CallJobState
    data class Done(val id: String, val name: String) : CallJobState
    data class Failed(val name: String, val message: String) : CallJobState
    data class Cancelled(val name: String) : CallJobState

    val busy: Boolean get() = this is Running
}

/**
 * The one call transcription that may run at a time (D46). [start] runs it on [runner] (a
 * worker thread): decode the picked file ([decoder]), open the job's own on-device engine
 * ([engine]: a model loaded for this job only, so a dictation meanwhile is not disturbed and the
 * audio never leaves the phone), feed 1 s blocks of 16 kHz mono through the meeting
 * transcriber, save the transcript. [cancel] stops it from any thread; nothing is saved then.
 * [CallJobService] holds the foreground + wake lock and the notification while [state] is running.
 * Dictation always comes first: see [yieldTo] and [backgroundThread].
 *
 * D49 queue: [startNext] runs the next auto-transcribe recording ([nextAuto]) when nothing runs.
 * A manual [start] while an auto job runs cancels that job ([CallStop.PREEMPTED], it goes back
 * in the queue) and runs right after it, so auto jobs always wait behind manual ones. [onEnd]
 * hears every job's outcome (on the worker thread, before [state] shows it).
 */
class CallJobs(
    private val store: CallsStore,
    private val decoder: (CallRequest) -> AudioSource,
    private val engine: (language: String) -> MeetingJob,
    private val now: () -> Long = System::currentTimeMillis,
    /**
     * Dictation comes first (D46): true while a dictation records or is processed. The job then
     * starts no new push (so no new chunk): at most the one chunk already in flight overlaps it.
     */
    private val yieldTo: () -> Boolean = { false },
    private val pause: (Long) -> Unit = { ms -> Thread.sleep(ms) },
    private val runner: (Runnable) -> Unit = ::backgroundThread,
    private val nextAuto: () -> CallRequest? = { null },
    private val onEnd: (CallRequest, CallJobState, CallStop?) -> Unit = { _, _, _ -> },
) {
    private val _state = MutableStateFlow<CallJobState>(CallJobState.Idle)
    val state: StateFlow<CallJobState> = _state

    @Volatile private var cancelled = false
    @Volatile private var stop: CallStop? = null
    @Volatile private var job: MeetingJob? = null
    /** The request running now (null when idle). */
    @Volatile var current: CallRequest? = null
        private set
    /** A manual request waiting for the auto job it preempted to stop. */
    private var pendingManual: CallRequest? = null

    /**
     * False while another manual transcription runs. A manual [req] while an auto job runs stops
     * that job (it is queued again) and runs right after it.
     */
    @Synchronized
    fun start(req: CallRequest): Boolean {
        if (_state.value.busy) {
            val cur = current
            if (req.auto || cur == null || !cur.auto || pendingManual != null) return false
            pendingManual = req
            cancel(CallStop.PREEMPTED)
            return true
        }
        launch(req)
        return true
    }

    /** D49: runs the next queued auto job when nothing runs; false when busy or the queue is empty. */
    @Synchronized
    fun startNext(): Boolean {
        if (_state.value.busy) return false
        val req = nextAuto() ?: return false
        launch(req)
        return true
    }

    private fun launch(req: CallRequest) {
        cancelled = false
        stop = null
        current = req
        _state.value = CallJobState.Running(req.name, loading = true, fraction = null)
        runner(Runnable { run(req) })
    }

    fun cancel(why: CallStop = CallStop.OWNER) {
        if (stop == null || why == CallStop.PREEMPTED) stop = why
        cancelled = true
        job?.cancel()
    }

    /** The page showed a finished / failed state: back to idle (a running job is left alone). */
    @Synchronized
    fun acknowledge() {
        if (!_state.value.busy) _state.value = CallJobState.Idle
    }

    private fun run(req: CallRequest) {
        var source: AudioSource? = null
        var j: MeetingJob? = null
        val end = try {
            val src = decoder(req).also { source = it }
            if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
            j = engine(req.language).also { job = it }
            if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
            val segments = transcribe(req, src, j)
            val t = CallTranscript(
                id = CallsStore.newId(now()), createdAt = now(), recordedAt = req.recordedAt.takeIf { it > 0 } ?: now(),
                sourceName = req.name, durationMs = j.progress().receivedMs, language = req.language, segments = segments,
            )
            store.save(t)
            CallJobState.Done(t.id, req.name)
        } catch (e: MeetingException) {
            if (e.cancelled || cancelled) CallJobState.Cancelled(req.name) else CallJobState.Failed(req.name, e.message ?: "Transcription failed")
        } catch (e: DecodeException) {
            CallJobState.Failed(req.name, e.message ?: "Can't read this recording")
        } catch (e: CoreException) {
            CallJobState.Failed(req.name, e.message ?: "The speech model failed to load")
        } catch (e: Exception) {
            if (cancelled) CallJobState.Cancelled(req.name) else CallJobState.Failed(req.name, e.message ?: e.javaClass.simpleName)
        } finally {
            job = null
            runCatching { j?.close() }
            runCatching { source?.close() }
        }
        val why = if (end is CallJobState.Cancelled) stop ?: CallStop.OWNER else null
        runCatching { onEnd(req, end, why) }
        synchronized(this) {
            current = null
            val next = pendingManual
            pendingManual = null
            if (next != null) launch(next) else _state.value = end
        }
    }

    private fun transcribe(req: CallRequest, src: AudioSource, j: MeetingJob): List<MeetingSegment> {
        src.durationMs?.let { j.setTotalSamples(it * 16) }
        val out = ArrayList<MeetingSegment>()
        val conv = To16kMono()
        val block = ShortArray(BLOCK)
        var fill = 0
        fun push(pcm: ShortArray) {
            var i = 0
            while (i < pcm.size) {
                val n = minOf(BLOCK - fill, pcm.size - i)
                System.arraycopy(pcm, i, block, fill, n)
                fill += n; i += n
                if (fill == BLOCK) { yieldToDictation(); out += j.push(block, BLOCK); fill = 0; report(req, j) }
            }
        }
        _state.value = CallJobState.Running(req.name, loading = false, fraction = j.progress().fraction ?: src.durationMs?.let { 0f })
        while (true) {
            if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
            val b = src.next() ?: break
            push(conv.feed(b))
        }
        push(conv.flush())
        yieldToDictation()
        if (fill > 0) out += j.push(block, fill)
        yieldToDictation()
        out += j.finish()
        return out
    }

    /** Waits (between chunks) while a dictation runs; a cancel still ends the wait. */
    private fun yieldToDictation() {
        while (yieldTo()) {
            if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
            pause(YIELD_POLL_MS)
        }
        if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
    }

    private fun report(req: CallRequest, j: MeetingJob) {
        _state.value = CallJobState.Running(req.name, loading = false, fraction = j.progress().fraction)
    }

    companion object {
        /** 1 s of 16 kHz audio per push. */
        const val BLOCK = 16_000
        const val YIELD_POLL_MS = 50L

        /**
         * The job's worker: lowest Java priority and, on Android, background nice (10). Native
         * worker threads the engine creates from it (model load and decoding run on it) inherit
         * that nice value, so the dictation engine wins the CPU.
         */
        fun backgroundThread(r: Runnable) {
            Thread({
                runCatching { android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_BACKGROUND) }
                r.run()
            }, "murmur-call-job").apply { priority = Thread.MIN_PRIORITY; isDaemon = true }.start()
        }
    }
}
