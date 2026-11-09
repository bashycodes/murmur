package dev.flowclone.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.SystemClock
import android.util.Log
import dev.flowclone.session.AudioBatcher
import dev.flowclone.session.AudioSource
import dev.flowclone.session.SAMPLE_RATE

/**
 * Recorder config: MIC source, 16 kHz mono PCM16, buffer =
 * getMinBufferSize (no multiplier), plain thread with blocking reads, regrouped into
 * 100 ms frames that go straight to `push_audio`. No WAV, no effects, no VAD.
 */
class MicRecorder : AudioSource {
    companion object {
        private const val TAG = "FlowMic"
        private const val TAIL_DRAIN_MS = 120L
        private const val ZERO_READ_LOG_THRESHOLD = 50
    }

    @Volatile private var recording = false
    private var thread: Thread? = null
    private var record: AudioRecord? = null

    override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean = start(onFrame) {}

    /** [onError]: a read error ended the capture (review C15); the controller then stops + submits. */
    @SuppressLint("MissingPermission") // caller checks RECORD_AUDIO
    override fun start(onFrame: (ShortArray, Int) -> Unit, onError: () -> Unit): Boolean {
        if (recording) return true
        val minBuf = AudioRecord.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        if (minBuf <= 0) return false
        val r = try {
            AudioRecord(MediaRecorder.AudioSource.MIC, SAMPLE_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, minBuf)
        } catch (e: Exception) {
            Log.w(TAG, "AudioRecord ctor failed", e); return false
        }
        if (r.state != AudioRecord.STATE_INITIALIZED) { r.release(); return false }
        try {
            r.startRecording()
        } catch (e: IllegalStateException) {
            Log.w(TAG, "startRecording failed", e); r.release(); return false
        }
        if (r.recordingState != AudioRecord.RECORDSTATE_RECORDING) { r.release(); return false }
        record = r
        recording = true
        val batcher = AudioBatcher(SAMPLE_RATE / 10, onFrame)
        val buf = ShortArray(minBuf / 2)
        thread = Thread({
            var zeroReads = 0
            var failed = false
            while (recording) {
                val n = r.read(buf, 0, buf.size)
                when {
                    n > 0 -> { zeroReads = 0; batcher.feed(buf, n) }
                    n == 0 -> if (++zeroReads == ZERO_READ_LOG_THRESHOLD) Log.w(TAG, "$zeroReads consecutive empty reads")
                    else -> { Log.w(TAG, "read error $n"); failed = true; break }
                }
            }
            if (failed) {
                batcher.flush()
                onError()
                return@Thread
            }
            // Tail drain: whatever is already buffered, non-blocking, ≤ 120 ms.
            val until = SystemClock.elapsedRealtime() + TAIL_DRAIN_MS
            while (SystemClock.elapsedRealtime() < until) {
                val n = r.read(buf, 0, buf.size, AudioRecord.READ_NON_BLOCKING)
                if (n <= 0) break
                batcher.feed(buf, n)
            }
            batcher.flush()
        }, "flow-mic").apply { start() }
        return true
    }

    override fun stop() {
        if (!recording && thread == null) return
        recording = false
        thread?.let { t ->
            t.join(2000)
            if (t.isAlive) { t.interrupt(); t.join(500) }
        }
        thread = null
        record?.let { runCatching { it.stop() }; it.release() }
        record = null
    }
}
