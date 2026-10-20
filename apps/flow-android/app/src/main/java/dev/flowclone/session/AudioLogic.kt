package dev.flowclone.session

import kotlin.math.log10
import kotlin.math.sqrt

const val SAMPLE_RATE = 16_000

/**
 * Wispr `VoiceRecorder` meter (07 §7): dBFS with an adaptive floor that only moves down
 * (starts at 0 dB, never below −50), mapped over a 30 dB range to 0..1.
 */
class LevelMeter {
    private var bg = 0.0

    fun reset() { bg = 0.0 }

    fun update(pcm: ShortArray, length: Int): Float {
        if (length <= 0) return 0f
        var sum = 0.0
        for (i in 0 until length) { val s = pcm[i] / 32767.0; sum += s * s }
        val db = 20 * log10(sqrt(sum / length) + 1e-9)
        if (db < bg) bg = maxOf(-50.0, db)
        return ((db - bg) / 30.0).coerceIn(0.0, 1.0).toFloat()
    }
}

/** `AudioBatcher`: regroups arbitrary AudioRecord reads into fixed 100 ms (1600-sample) frames. */
class AudioBatcher(private val frameSamples: Int = SAMPLE_RATE / 10, private val emit: (ShortArray, Int) -> Unit) {
    private var buf = ShortArray(frameSamples)
    private var fill = 0

    fun feed(pcm: ShortArray, length: Int) {
        var off = 0
        while (off < length) {
            val n = minOf(frameSamples - fill, length - off)
            System.arraycopy(pcm, off, buf, fill, n)
            fill += n; off += n
            if (fill == frameSamples) {
                emit(buf, fill)
                buf = ShortArray(frameSamples) // receiver may retain the frame
                fill = 0
            }
        }
    }

    fun flush() {
        if (fill == 0) return
        emit(buf, fill)
        buf = ShortArray(frameSamples)
        fill = 0
    }
}

/**
 * Session length policy (shell-owned per §9): Wispr warns at 239 s and stops at 299 s
 * of a 5-minute cap, i.e. 61 s / 1 s before the cap. Configurable cap.
 */
data class SessionCap(val capMs: Long = 300_000) {
    val warnAtMs: Long get() = capMs - 61_000
    val stopAtMs: Long get() = capMs - 1_000
    /** Hard core-side bound handed to `start_session`. */
    val maxSamples: Long get() = capMs / 1000 * SAMPLE_RATE
}
