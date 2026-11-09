package dev.murmur.calls

import java.io.Closeable
import java.io.EOFException
import java.io.InputStream
import kotlin.math.PI
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.roundToInt
import kotlin.math.sin

/** Decoded audio: interleaved frames in [-1, 1] at [sampleRate] with [channels] channels. */
class AudioBlock(val samples: FloatArray, val channels: Int, val sampleRate: Int) {
    val frames: Int get() = if (channels <= 0) 0 else samples.size / channels
}

/** A decoder: blocks in order, null at the end. */
interface AudioSource : Closeable {
    /** Length of the recording when the container says (progress), else null. */
    val durationMs: Long?

    /** @throws DecodeException */
    fun next(): AudioBlock?
}

class DecodeException(message: String, cause: Throwable? = null) : Exception(message, cause)

/**
 * Streaming resampler for one channel: windowed-sinc interpolation (Blackman window,
 * [ZERO_CROSSINGS] each side) with the cut-off below the lower Nyquist, so 44.1 / 48 kHz call
 * audio does not alias into the 0-8 kHz band speech recognition sees. Equal rates pass through.
 */
class Resampler(private val inRate: Int, private val outRate: Int = TARGET_RATE) {
    private val step = inRate.toDouble() / outRate
    /** Cut-off as a fraction of the input rate's Nyquist. */
    private val cutoff = minOf(1.0, outRate.toDouble() / inRate) * 0.94
    private val half = ceil(ZERO_CROSSINGS / cutoff).toInt()
    private var buf = FloatArray(4096)
    private var len = 0
    /** Absolute input index of buf[0]. */
    private var bufStart = 0L
    private var fed = 0L
    /** Absolute input position of the next output sample. */
    private var t = 0.0
    private var produced = 0L
    private val pass = inRate == outRate

    init {
        require(inRate > 0 && outRate > 0) { "bad sample rate $inRate -> $outRate" }
    }

    fun feed(x: FloatArray, n: Int = x.size): FloatArray {
        if (pass) { fed += n; return x.copyOf(n) }
        if (len + n > buf.size) buf = buf.copyOf(maxOf(buf.size * 2, len + n))
        System.arraycopy(x, 0, buf, len, n)
        len += n; fed += n
        return produce(fed - half)
    }

    /** End of input: the rest, as if followed by silence; the output length is round(in × out / in). */
    fun flush(): FloatArray {
        if (pass) return FloatArray(0)
        val total = (fed * outRate.toDouble() / inRate).roundToInt().toLong()
        val out = FloatArray((total - produced).coerceAtLeast(0).toInt())
        for (i in out.indices) { out[i] = at(t); t += step }
        produced += out.size
        return out
    }

    /** Outputs whose kernel ends before input index [limit]. */
    private fun produce(limit: Long): FloatArray {
        val n = if (t >= limit) 0 else ceil((limit - t) / step).toInt().coerceAtLeast(0)
        val out = FloatArray(n)
        for (i in 0 until n) { out[i] = at(t); t += step }
        produced += n
        // Keep what the next kernels still need.
        val keepFrom = (floor(t).toLong() - half - 1).coerceAtLeast(bufStart)
        val drop = (keepFrom - bufStart).toInt().coerceAtMost(len)
        if (drop > 0) {
            System.arraycopy(buf, drop, buf, 0, len - drop)
            len -= drop; bufStart += drop
        }
        return out
    }

    private fun sample(i: Long): Float {
        val k = i - bufStart
        return if (k < 0 || k >= len) 0f else buf[k.toInt()]
    }

    private fun at(pos: Double): Float {
        val c = floor(pos).toLong()
        var acc = 0.0
        for (i in c - half + 1..c + half) {
            val x = pos - i
            acc += sample(i) * kernel(x)
        }
        return acc.toFloat()
    }

    /** The kernel sampled [TABLE_RES] times per input sample (no trig per tap). */
    private val table = DoubleArray(half * TABLE_RES + 2) { j -> kernelExact(j.toDouble() / TABLE_RES) }

    private fun kernel(x: Double): Double {
        val a = kotlin.math.abs(x) * TABLE_RES
        val j = a.toInt()
        if (j >= half * TABLE_RES) return 0.0
        val f = a - j
        return table[j] + (table[j + 1] - table[j]) * f
    }

    private fun kernelExact(x: Double): Double {
        if (x >= half) return 0.0
        val u = x * cutoff
        val sinc = if (u == 0.0) 1.0 else sin(PI * u) / (PI * u)
        val w = (x / half + 1) / 2 // 0.5..1 across the window's right half
        val blackman = 0.42 - 0.5 * cos(2 * PI * w) + 0.08 * cos(4 * PI * w)
        return cutoff * sinc * blackman
    }

    companion object {
        const val TARGET_RATE = 16_000
        const val ZERO_CROSSINGS = 12
        private const val TABLE_RES = 512
    }
}

/**
 * Any decoded audio → 16 kHz mono PCM16 (D46): channels averaged (a call recording that keeps
 * the two sides on two channels becomes one mix), resampled with [Resampler], clipped. A change
 * of format mid-stream (a decoder's output format change) restarts the resampler.
 */
class To16kMono {
    private var rate = 0
    private var resampler: Resampler? = null

    fun feed(b: AudioBlock): ShortArray {
        val pre = if (b.sampleRate != rate) flushResampler().also { rate = b.sampleRate; resampler = Resampler(rate) } else ShortArray(0)
        val mono = downmix(b)
        val out = toPcm16(resampler!!.feed(mono))
        return if (pre.isEmpty()) out else pre + out
    }

    fun flush(): ShortArray = flushResampler()

    private fun flushResampler(): ShortArray = resampler?.let { toPcm16(it.flush()) } ?: ShortArray(0)

    companion object {
        fun downmix(b: AudioBlock): FloatArray {
            val ch = b.channels.coerceAtLeast(1)
            if (ch == 1) return b.samples
            val n = b.samples.size / ch
            return FloatArray(n) { i ->
                var s = 0f
                for (c in 0 until ch) s += b.samples[i * ch + c]
                s / ch
            }
        }

        fun toPcm16(x: FloatArray): ShortArray = ShortArray(x.size) { i -> (x[i] * 32767f).roundToInt().coerceIn(-32768, 32767).toShort() }
    }
}

/**
 * RIFF/WAVE reader for the plain (non-MediaCodec) path, so WAV call exports and the host tests
 * do not depend on the phone's codecs: PCM 8/16/24/32-bit, IEEE float 32/64-bit and their
 * WAVE_FORMAT_EXTENSIBLE forms. A streamed file whose data size is 0 or unknown reads to the end.
 */
class WavSource(private val input: InputStream) : AudioSource {
    val sampleRate: Int
    val channels: Int
    private val bits: Int
    private val float: Boolean
    private val blockAlign: Int
    private var remaining: Long
    override val durationMs: Long?

    init {
        try {
            val head = readExactly(12)
            if (String(head, 0, 4, Charsets.US_ASCII) != "RIFF" || String(head, 8, 4, Charsets.US_ASCII) != "WAVE") throw DecodeException("not a WAV file")
            var fmt: ByteArray? = null
            var dataSize = 0L
            while (true) {
                val ch = readExactly(8)
                val id = String(ch, 0, 4, Charsets.US_ASCII)
                val size = le32(ch, 4)
                if (id == "fmt ") {
                    fmt = readExactly(size.toInt())
                    if (size % 2 == 1L) skip(1)
                } else if (id == "data") {
                    dataSize = size
                    break
                } else {
                    skip(size + size % 2)
                }
            }
            val f = fmt ?: throw DecodeException("WAV file has no format chunk")
            if (f.size < 16) throw DecodeException("WAV format chunk too short")
            var tag = le16(f, 0)
            channels = le16(f, 2)
            sampleRate = le32(f, 4).toInt()
            blockAlign = le16(f, 12)
            bits = le16(f, 14)
            if (tag == 0xFFFE && f.size >= 26) tag = le16(f, 24)
            float = when (tag) {
                1 -> false
                3 -> true
                else -> throw DecodeException("WAV encoding $tag is not supported (PCM or float only)")
            }
            if (channels < 1 || sampleRate < 1000 || blockAlign != channels * (bits / 8) ||
                (float && bits != 32 && bits != 64) || (!float && bits !in listOf(8, 16, 24, 32))
            ) throw DecodeException("unsupported WAV format: $channels ch, $sampleRate Hz, $bits bit")
            remaining = if (dataSize == 0L || dataSize == 0xFFFFFFFFL) Long.MAX_VALUE else dataSize
            durationMs = if (remaining == Long.MAX_VALUE) null else remaining / blockAlign * 1000 / sampleRate
        } catch (e: EOFException) {
            throw DecodeException("WAV file is cut short", e)
        }
    }

    private val chunk = ByteArray(blockAlign * 4096)

    override fun next(): AudioBlock? {
        if (remaining <= 0) return null
        val want = minOf(chunk.size.toLong(), remaining).toInt() / blockAlign * blockAlign
        if (want <= 0) { remaining = 0; return null }
        var got = 0
        while (got < want) {
            val n = input.read(chunk, got, want - got)
            if (n < 0) break
            got += n
        }
        got = got / blockAlign * blockAlign
        if (got == 0) { remaining = 0; return null }
        if (remaining != Long.MAX_VALUE) remaining -= got
        val bps = bits / 8
        val out = FloatArray(got / bps)
        for (i in out.indices) out[i] = decode(chunk, i * bps)
        return AudioBlock(out, channels, sampleRate)
    }

    private fun decode(b: ByteArray, o: Int): Float = if (float) {
        if (bits == 32) java.lang.Float.intBitsToFloat(le32(b, o).toInt())
        else java.lang.Double.longBitsToDouble(le32(b, o) or (le32(b, o + 4) shl 32)).toFloat()
    } else when (bits) {
        8 -> ((b[o].toInt() and 0xFF) - 128) / 128f
        16 -> ((b[o].toInt() and 0xFF) or (b[o + 1].toInt() shl 8)).toShort() / 32768f
        24 -> (((b[o].toInt() and 0xFF) or ((b[o + 1].toInt() and 0xFF) shl 8) or (b[o + 2].toInt() shl 16))) / 8388608f
        else -> le32(b, o).toInt() / 2147483648f
    }

    private fun readExactly(n: Int): ByteArray {
        val b = ByteArray(n)
        var got = 0
        while (got < n) {
            val r = input.read(b, got, n - got)
            if (r < 0) throw EOFException()
            got += r
        }
        return b
    }

    private fun skip(n: Long) {
        var left = n
        while (left > 0) {
            val s = input.skip(left)
            if (s <= 0) { if (input.read() < 0) throw EOFException(); left-- } else left -= s
        }
    }

    override fun close() = input.close()

    companion object {
        fun le16(b: ByteArray, o: Int) = (b[o].toInt() and 0xFF) or ((b[o + 1].toInt() and 0xFF) shl 8)
        fun le32(b: ByteArray, o: Int): Long =
            ((b[o].toLong() and 0xFF) or ((b[o + 1].toLong() and 0xFF) shl 8) or ((b[o + 2].toLong() and 0xFF) shl 16) or ((b[o + 3].toLong() and 0xFF) shl 24))

        /** "RIFF....WAVE" at the start of [head]. */
        fun isWav(head: ByteArray): Boolean = head.size >= 12 &&
            String(head, 0, 4, Charsets.US_ASCII) == "RIFF" && String(head, 8, 4, Charsets.US_ASCII) == "WAVE"
    }
}
