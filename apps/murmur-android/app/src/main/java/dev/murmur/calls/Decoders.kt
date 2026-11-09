package dev.murmur.calls

import android.content.Context
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.net.Uri
import java.nio.ByteOrder

/**
 * Opens a picked recording for reading only (D46: the file is never written, moved or deleted).
 * WAV goes through [WavSource]; everything else (Samsung's m4a/AAC call recordings, 3gp/AMR,
 * mp3, ogg…) through the phone's own decoders ([MediaCodecSource]).
 */
object Decoders {
    /** @throws DecodeException */
    fun open(ctx: Context, uri: Uri): AudioSource {
        val head = try {
            ctx.contentResolver.openInputStream(uri)?.use { s -> ByteArray(12).also { b -> var n = 0; while (n < 12) { val r = s.read(b, n, 12 - n); if (r < 0) break; n += r } } }
        } catch (e: Exception) {
            throw DecodeException("Can't open the recording: ${e.message}", e)
        } ?: throw DecodeException("Can't open the recording")
        return if (WavSource.isWav(head)) {
            WavSource(ctx.contentResolver.openInputStream(uri)?.buffered(1 shl 16) ?: throw DecodeException("Can't open the recording"))
        } else {
            MediaCodecSource(ctx, uri)
        }
    }
}

/** The first audio track of [uri], decoded by MediaCodec to PCM (16-bit or float, as the codec gives it). */
class MediaCodecSource(ctx: Context, uri: Uri) : AudioSource {
    private val extractor = MediaExtractor()
    private val codec: MediaCodec
    override val durationMs: Long?
    private var rate: Int
    private var channels: Int
    private var floatPcm = false
    private var inputDone = false
    private var outputDone = false
    private val info = MediaCodec.BufferInfo()

    init {
        try {
            extractor.setDataSource(ctx, uri, null)
            val track = (0 until extractor.trackCount).firstOrNull { i ->
                extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true
            } ?: throw DecodeException("The file has no audio track")
            extractor.selectTrack(track)
            val format = extractor.getTrackFormat(track)
            val mime = format.getString(MediaFormat.KEY_MIME)!!
            durationMs = if (format.containsKey(MediaFormat.KEY_DURATION)) format.getLong(MediaFormat.KEY_DURATION) / 1000 else null
            rate = format.getInteger(MediaFormat.KEY_SAMPLE_RATE)
            channels = format.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
            codec = try {
                MediaCodec.createDecoderByType(mime)
            } catch (e: Exception) {
                throw DecodeException("This phone can't decode $mime audio", e)
            }
            codec.configure(format, null, null, 0)
            codec.start()
        } catch (e: DecodeException) {
            extractor.release(); throw e
        } catch (e: Exception) {
            extractor.release(); throw DecodeException("Can't read this file as audio: ${e.message}", e)
        }
    }

    override fun next(): AudioBlock? {
        try {
            while (!outputDone) {
                if (!inputDone) feedInput()
                val idx = codec.dequeueOutputBuffer(info, TIMEOUT_US)
                when {
                    idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> readFormat(codec.outputFormat)
                    idx >= 0 -> {
                        val block = readOutput(idx)
                        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) outputDone = true
                        if (block != null) return block
                    }
                    else -> Unit // try again later
                }
            }
            return null
        } catch (e: IllegalStateException) {
            throw DecodeException("Decoding failed: ${e.message}", e)
        }
    }

    private fun feedInput() {
        val idx = codec.dequeueInputBuffer(TIMEOUT_US)
        if (idx < 0) return
        val buf = codec.getInputBuffer(idx) ?: return
        val n = extractor.readSampleData(buf, 0)
        if (n < 0) {
            codec.queueInputBuffer(idx, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
            inputDone = true
        } else {
            codec.queueInputBuffer(idx, 0, n, extractor.sampleTime.coerceAtLeast(0), 0)
            extractor.advance()
        }
    }

    private fun readFormat(f: MediaFormat) {
        if (f.containsKey(MediaFormat.KEY_SAMPLE_RATE)) rate = f.getInteger(MediaFormat.KEY_SAMPLE_RATE)
        if (f.containsKey(MediaFormat.KEY_CHANNEL_COUNT)) channels = f.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
        floatPcm = f.containsKey(MediaFormat.KEY_PCM_ENCODING) && f.getInteger(MediaFormat.KEY_PCM_ENCODING) == AudioFormat.ENCODING_PCM_FLOAT
    }

    private fun readOutput(idx: Int): AudioBlock? {
        val buf = codec.getOutputBuffer(idx)
        val block = if (buf == null || info.size <= 0) null else {
            buf.position(info.offset); buf.limit(info.offset + info.size)
            val b = buf.slice().order(ByteOrder.nativeOrder())
            val samples = if (floatPcm) {
                FloatArray(info.size / 4).also { b.asFloatBuffer().get(it) }
            } else {
                val s = ShortArray(info.size / 2).also { b.asShortBuffer().get(it) }
                FloatArray(s.size) { i -> s[i] / 32768f }
            }
            AudioBlock(samples, channels, rate)
        }
        codec.releaseOutputBuffer(idx, false)
        return block
    }

    override fun close() {
        runCatching { codec.stop() }
        runCatching { codec.release() }
        runCatching { extractor.release() }
    }

    private companion object {
        const val TIMEOUT_US = 10_000L
    }
}
