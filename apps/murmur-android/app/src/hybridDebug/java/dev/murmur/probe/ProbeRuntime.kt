package dev.murmur.probe

import android.content.Context
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.io.IOException

/**
 * Process-wide probe state shared by the probe screen and its overlay: the current synthetic
 * clip, whether the probe screen is on top, and the run log. English 1 spike, hybrid debug only.
 */
object ProbeRuntime {
    const val TAG = "EnglishProbe"
    const val LOG_FILE = "probe-english.jsonl"

    data class Clip(val pcm: ShortArray, val source: String, val label: String) {
        val durationMs get() = ProbePcm.durationMs(pcm.size)
    }

    @Volatile var clip: Clip? = null
    /** Probe activity resumed = our app is the top activity (the GenAI foreground-only question). */
    @Volatile var probeOnTop = false
    @Volatile var realTimePacing = true
    @Volatile var stopAtClipEnd = true
    /** Screen settings the overlay reuses. Biasing defaults are synthetic terms. */
    @Volatile var genAiAdvanced = true
    @Volatile var biasing: List<String> = listOf("pytest", "UUID4", "Kubectl")
    @Volatile var formatting = true
    @Volatile var lastGenAiStatus: String? = null
    @Volatile var lastPlatformSupport: String? = null

    fun now() = SystemClock.elapsedRealtime()

    fun logFile(ctx: Context) = File(ctx.filesDir, LOG_FILE)

    @Synchronized fun append(ctx: Context, r: ProbeRecord): String {
        val line = r.toJsonLine()
        runCatching { logFile(ctx).appendText(line + "\n") }.onFailure { Log.w(TAG, "log write failed", it) }
        Log.i(TAG, line)
        return line
    }

    fun build(): String = dev.murmur.data.AppVersion.current.full
}

/**
 * Writes 16 kHz mono PCM16 into [out] (the write end of a pipe an engine reads), in 100 ms
 * chunks, paced like a mic when [realTime]. Closes [out] at the end and reports the close time
 * (= end of audio) to [onEnd]; a reader that went away just ends the feed early.
 */
class ProbeFeeder(
    private val pcm: ShortArray,
    private val realTime: Boolean,
    private val out: ParcelFileDescriptor,
    private val onEnd: (Long) -> Unit,
) {
    @Volatile private var stopped = false
    private val thread = Thread({ feed() }, "probe-feed")

    fun start() = apply { thread.start() }
    fun stop() { stopped = true; thread.interrupt() }

    private fun feed() {
        val chunk = ProbePcm.RATE / 10
        try {
            ParcelFileDescriptor.AutoCloseOutputStream(out).use { os ->
                val t0 = ProbeRuntime.now()
                var off = 0
                while (!stopped && off < pcm.size) {
                    val n = minOf(chunk, pcm.size - off)
                    if (realTime) {
                        val wait = t0 + ProbePcm.durationMs(off + n) - ProbeRuntime.now()
                        if (wait > 0) Thread.sleep(wait)
                    }
                    os.write(ProbePcm.toLeBytes(pcm, off, off + n))
                    off += n
                }
                os.flush()
            }
        } catch (e: IOException) {
            Log.i(ProbeRuntime.TAG, "feed ended early: $e")
        } catch (_: InterruptedException) {
            runCatching { out.close() }
        }
        onEnd(ProbeRuntime.now())
    }
}
