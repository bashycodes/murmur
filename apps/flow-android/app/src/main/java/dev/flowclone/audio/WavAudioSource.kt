package dev.flowclone.audio

import dev.flowclone.session.AudioBatcher
import dev.flowclone.session.AudioSource
import dev.flowclone.session.SAMPLE_RATE

/**
 * Debug "WAV as microphone" (debug builds only reach it: see src/debug WavMicReceiver): plays
 * 16 kHz mono [pcm] into the dictation pipeline in place of the mic. Frames go through the same
 * [AudioBatcher] as [MicRecorder] (100 ms frames, tail flushed), so the controller, the core and
 * the debug log see exactly what the mic would have delivered for that audio.
 *
 * [realTime]: one frame per 100 ms of audio, each once its audio has "been spoken" (as from a
 * mic); false = as fast as the pipeline takes them. [onEnd] runs once on the feeder thread after
 * the last frame (the app then releases the dictation), never when [stop] came first.
 */
class WavAudioSource(
    private val pcm: ShortArray,
    private val realTime: Boolean = true,
    private val onEnd: () -> Unit = {},
    private val clockMs: () -> Long = { System.nanoTime() / 1_000_000 },
    private val sleepMs: (Long) -> Unit = Thread::sleep,
) : AudioSource {
    companion object {
        const val SOURCE = "wav"
        private const val FRAME = SAMPLE_RATE / 10
    }

    override val source: String get() = SOURCE

    @Volatile private var playing = false
    private var thread: Thread? = null

    override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean = start(onFrame) {}

    override fun start(onFrame: (ShortArray, Int) -> Unit, onError: () -> Unit): Boolean {
        if (playing) return true
        playing = true
        thread = Thread({ feed(onFrame) }, "flow-wav").apply { start() }
        return true
    }

    private fun feed(onFrame: (ShortArray, Int) -> Unit) {
        val batcher = AudioBatcher(FRAME) { f, n -> if (playing) onFrame(f, n) }
        val t0 = clockMs()
        var off = 0
        try {
            while (playing && off < pcm.size) {
                val n = minOf(FRAME, pcm.size - off)
                if (realTime) {
                    val due = t0 + ((off + n).toLong() * 1000 + SAMPLE_RATE - 1) / SAMPLE_RATE
                    val wait = due - clockMs()
                    if (wait > 0) sleepMs(wait)
                    if (!playing) break
                }
                batcher.feed(pcm.copyOfRange(off, off + n), n)
                off += n
            }
        } catch (_: InterruptedException) {
            return
        }
        batcher.flush()
        if (playing && off >= pcm.size) onEnd()
    }

    override fun stop() {
        playing = false
        thread?.let { t ->
            if (t !== Thread.currentThread()) {
                t.interrupt()
                t.join(1000)
            }
        }
        thread = null
    }
}

/**
 * The controller's audio source: the mic, or for exactly one dictation a source [arm]ed by the
 * debug "WAV as microphone" receiver. [source] names the one in use (debug log `source`).
 */
class AudioSwitch(private val mic: AudioSource) : AudioSource {
    @Volatile private var next: AudioSource? = null
    @Volatile private var active: AudioSource? = null

    override val source: String get() = (active ?: mic).source

    /** The next [start] uses [s] instead of the mic (once). */
    fun arm(s: AudioSource) { next = s }

    /** Drops an armed source the controller did not start. True when there was one (it was not used). */
    fun disarm(): Boolean {
        val unused = next != null
        next = null
        return unused
    }

    override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean = start(onFrame) {}

    override fun start(onFrame: (ShortArray, Int) -> Unit, onError: () -> Unit): Boolean {
        val s = next ?: mic
        next = null
        active = s
        return s.start(onFrame, onError).also { if (!it) active = null }
    }

    override fun stop() {
        active?.stop()
    }
}
