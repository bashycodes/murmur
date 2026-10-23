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

/**
 * Wispr `HeadSilenceTracker` (07 §7): a frame is signal when mean(s²) > 33856 (RMS ≈ 184,
 * ≈ −45 dBFS). Tracks the leading silence so the shell can say "can't hear you".
 *
 * The trailing silence (D21 `endpoint_wait_ms`) uses a different, adaptive bar: a phone mic's
 * room noise sits above −45 dBFS, so the fixed bar never saw the silence after the last word
 * (findings 2026-10-23: 0 ms on every device line). As in core vad.rs, the noise floor is the
 * 10th-percentile frame RMS (last 30 s) and a frame is speech when its RMS is at least
 * clamp(3 × floor, −48 dBFS, −36 dBFS); the cap keeps clear speech as speech when there is no
 * quiet frame to learn the floor from (fail open: a loud room reads 0, as before).
 */
class SilenceTracker {
    companion object {
        const val SIGNAL_MEAN_SQUARE = 33_856.0
        /** vad.rs ABS_MIN / ABS_CAP (0.004 / 0.015 full scale) in PCM16 units. */
        const val SPEECH_RMS_MIN = 0.004 * 32768
        const val SPEECH_RMS_CAP = 0.015 * 32768
        const val FLOOR_FACTOR = 3.0
        /** Noise-floor window in frames (100 ms each: 30 s). */
        const val FLOOR_FRAMES = 300
    }

    @Volatile var heardSpeech = false; private set
    @Volatile var leadingSilenceMs = 0L; private set
    /** Silence since the last speech frame (D21 debug log `endpoint_wait_ms`); 0 before any speech. */
    @Volatile var trailingSilenceMs = 0L; private set
    private var leadingSamples = 0L
    private var trailingSamples = 0L
    private var heardRelativeSpeech = false
    private val levels = DoubleArray(FLOOR_FRAMES)
    private var levelCount = 0
    private var levelNext = 0

    fun reset() {
        heardSpeech = false; leadingSilenceMs = 0; leadingSamples = 0; trailingSilenceMs = 0; trailingSamples = 0
        heardRelativeSpeech = false; levelCount = 0; levelNext = 0
    }

    /** Current speech bar (RMS, PCM16 units) from the frames seen so far. */
    private fun speechBar(): Double {
        val sorted = levels.copyOf(levelCount).also { it.sort() }
        val floor = sorted[levelCount / 10]
        return (floor * FLOOR_FACTOR).coerceIn(SPEECH_RMS_MIN, SPEECH_RMS_CAP)
    }

    fun update(pcm: ShortArray, length: Int) {
        if (length <= 0) return
        var sum = 0.0
        for (i in 0 until length) { val v = pcm[i].toDouble(); sum += v * v }
        val meanSquare = sum / length
        val rms = kotlin.math.sqrt(meanSquare)
        levels[levelNext] = rms
        levelNext = (levelNext + 1) % FLOOR_FRAMES
        if (levelCount < FLOOR_FRAMES) levelCount++

        val signal = meanSquare > SIGNAL_MEAN_SQUARE
        if (signal) heardSpeech = true else if (!heardSpeech) {
            leadingSamples += length
            leadingSilenceMs = leadingSamples * 1000 / SAMPLE_RATE
        }
        if (signal && rms >= speechBar()) {
            heardRelativeSpeech = true
            trailingSamples = 0; trailingSilenceMs = 0
        } else if (heardRelativeSpeech) {
            trailingSamples += length
            trailingSilenceMs = trailingSamples * 1000 / SAMPLE_RATE
        }
    }
}

/**
 * "Keep last dictation audio (debug)": a copy of every mic frame of one dictation, up to
 * [maxSamples] (the session cap). Frames arrive on the capture thread; [take] runs after stop.
 */
class PcmCapture(private val maxSamples: Int) {
    private var buf = ShortArray(SAMPLE_RATE * 10)
    private var size = 0

    @Synchronized fun append(pcm: ShortArray, length: Int) {
        val n = minOf(length, maxSamples - size)
        if (n <= 0) return
        if (size + n > buf.size) buf = buf.copyOf(minOf(maxSamples, maxOf(buf.size * 2, size + n)))
        System.arraycopy(pcm, 0, buf, size, n)
        size += n
    }

    @Synchronized fun take(): ShortArray = buf.copyOf(size)
}

/**
 * Review C2: the session start runs off Main and can block (server route: TCP + TLS + hello).
 * The mic starts at once; frames captured meanwhile are held here and replayed in order when
 * the session is up, so the first words are not lost. Thread-safe (AudioRecord thread + start).
 */
class StartGate(private val maxHeldSamples: Int = SAMPLE_RATE * 30) {
    private val lock = Any()
    private var sink: ((ShortArray, Int) -> Unit)? = null
    private val held = ArrayDeque<ShortArray>()
    private var heldSamples = 0

    /** Frames that did not fit while waiting (more than [maxHeldSamples]). */
    var droppedSamples = 0; private set

    val isOpen: Boolean get() = synchronized(lock) { sink != null }

    fun push(pcm: ShortArray, length: Int) = synchronized(lock) {
        val s = sink
        when {
            s != null -> s(pcm, length)
            heldSamples + length > maxHeldSamples -> droppedSamples += length
            else -> { held.addLast(pcm.copyOf(length)); heldSamples += length }
        }
    }

    /** Replay the held frames into [to], then pass every later frame straight through. */
    fun open(to: (ShortArray, Int) -> Unit) = synchronized(lock) {
        while (held.isNotEmpty()) held.removeFirst().let { to(it, it.size) }
        heldSamples = 0
        sink = to
    }
}
