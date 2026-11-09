package dev.murmur.calls

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.sin
import kotlin.math.sqrt

/** D46 decoding: WAV reader, downmix, 16 kHz resampling, with synthetic tones only. */
class AudioTest {
    private fun tone(hz: Double, rate: Int, seconds: Double, amp: Double = 0.5) =
        FloatArray((rate * seconds).toInt()) { i -> (amp * sin(2 * PI * hz * i / rate)).toFloat() }

    /** Amplitude of [hz] in [x] (16 kHz), by correlation. */
    private fun magnitude(x: ShortArray, hz: Double, rate: Int = 16_000): Double {
        var re = 0.0; var im = 0.0
        for (i in x.indices) { val a = 2 * PI * hz * i / rate; re += x[i] * kotlin.math.cos(a); im += x[i] * sin(a) }
        return 2 * sqrt(re * re + im * im) / x.size / 32768.0
    }

    private fun convert(blocks: List<AudioBlock>): ShortArray {
        val c = To16kMono()
        val out = ArrayList<Short>()
        for (b in blocks) out += c.feed(b).toList()
        out += c.flush().toList()
        return out.toShortArray()
    }

    @Test fun resamples44k1And48kToSixteenKeepingLengthAndPitch() {
        for (rate in listOf(44_100, 48_000, 8_000, 22_050)) {
            val x = tone(440.0, rate, 2.0)
            // Fed in odd-sized pieces, as a decoder hands them out.
            val blocks = x.toList().chunked(1234).map { AudioBlock(it.toFloatArray(), 1, rate) }
            val y = convert(blocks)
            assertEquals("length at $rate", 32_000, y.size)
            val mid = y.copyOfRange(4000, 28_000)
            assertEquals("440 Hz kept at $rate", 0.5, magnitude(mid, 440.0), 0.02)
            assertTrue("no 1 kHz at $rate", magnitude(mid, 1000.0) < 0.01)
        }
    }

    @Test fun removesWhatIsAboveTheNewNyquist() {
        // 12 kHz at 48 kHz would alias to 4 kHz at 16 kHz.
        val y = convert(listOf(AudioBlock(tone(12_000.0, 48_000, 1.0), 1, 48_000)))
        assertTrue(magnitude(y.copyOfRange(2000, 14_000), 4000.0) < 0.01)
    }

    @Test fun sixteenKPassesThroughUnchanged() {
        val x = tone(300.0, 16_000, 0.5)
        val y = convert(listOf(AudioBlock(x, 1, 16_000)))
        assertEquals(x.size, y.size)
        for (i in x.indices) assertEquals(To16kMono.toPcm16(floatArrayOf(x[i]))[0], y[i])
    }

    @Test fun stereoIsAveragedToMono() {
        val l = tone(440.0, 16_000, 0.5)
        val inter = FloatArray(l.size * 2) { i -> if (i % 2 == 0) l[i / 2] else -l[i / 2] * 0f + 0.25f }
        val m = To16kMono.downmix(AudioBlock(inter, 2, 16_000))
        assertEquals(l.size, m.size)
        for (i in m.indices) assertEquals((l[i] + 0.25f) / 2, m[i], 1e-6f)
    }

    @Test fun clipsInsteadOfWrapping() {
        val p = To16kMono.toPcm16(floatArrayOf(1.5f, -1.5f, 0f))
        assertEquals(32767, p[0].toInt()); assertEquals(-32768, p[1].toInt()); assertEquals(0, p[2].toInt())
    }

    // ---- WAV ----

    private fun wav(rate: Int, channels: Int, bits: Int, frames: Int, float: Boolean = false, extensible: Boolean = false, streamed: Boolean = false, sample: (Int, Int) -> Double): ByteArray {
        val bps = bits / 8
        val data = ByteBuffer.allocate(frames * channels * bps).order(ByteOrder.LITTLE_ENDIAN)
        for (f in 0 until frames) for (c in 0 until channels) {
            val v = sample(f, c)
            when {
                float && bits == 32 -> data.putFloat(v.toFloat())
                float -> data.putDouble(v)
                bits == 8 -> data.put(((v * 127).toInt() + 128).toByte())
                bits == 16 -> data.putShort((v * 32767).toInt().toShort())
                bits == 24 -> { val i = (v * 8_388_607).toInt(); data.put(i.toByte()); data.put((i shr 8).toByte()); data.put((i shr 16).toByte()) }
                else -> data.putInt((v * 2_147_483_647.0).toInt())
            }
        }
        val fmtLen = if (extensible) 40 else 16
        val fmt = ByteBuffer.allocate(fmtLen).order(ByteOrder.LITTLE_ENDIAN).apply {
            putShort(if (extensible) 0xFFFE.toShort() else if (float) 3 else 1)
            putShort(channels.toShort()); putInt(rate); putInt(rate * channels * bps)
            putShort((channels * bps).toShort()); putShort(bits.toShort())
            if (extensible) { putShort(22); putShort(bits.toShort()); putInt(0); putShort(if (float) 3 else 1); put(ByteArray(14)) }
        }.array()
        val out = ByteArrayOutputStream()
        fun le32(v: Int) = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(v).array()
        out.write("RIFF".toByteArray()); out.write(le32(0)); out.write("WAVE".toByteArray())
        // An unknown chunk first (odd size, padded), as some recorders write.
        out.write("LIST".toByteArray()); out.write(le32(3)); out.write(byteArrayOf(1, 2, 3, 0))
        out.write("fmt ".toByteArray()); out.write(le32(fmtLen)); out.write(fmt)
        out.write("data".toByteArray()); out.write(le32(if (streamed) 0 else data.capacity())); out.write(data.array())
        return out.toByteArray()
    }

    private fun readAll(s: WavSource): FloatArray {
        val out = ArrayList<Float>()
        while (true) { val b = s.next() ?: break; out += b.samples.toList() }
        return out.toFloatArray()
    }

    @Test fun readsEveryPcmAndFloatWidth() {
        for ((bits, float) in listOf(8 to false, 16 to false, 24 to false, 32 to false, 32 to true, 64 to true)) {
            val bytes = wav(8000, 2, bits, 10_000, float) { f, c -> if (c == 0) 0.5 * sin(f / 10.0) else -0.25 }
            val s = WavSource(ByteArrayInputStream(bytes))
            assertEquals(8000, s.sampleRate); assertEquals(2, s.channels)
            assertEquals(1250L, s.durationMs)
            val x = readAll(s)
            assertEquals(20_000, x.size)
            val tol = if (bits == 8) 0.02f else 0.001f
            assertEquals("$bits/$float", (0.5 * sin(123 / 10.0)).toFloat(), x[246], tol)
            assertEquals("$bits/$float", -0.25f, x[247], tol)
        }
    }

    @Test fun readsExtensibleAndStreamedFiles() {
        val ext = WavSource(ByteArrayInputStream(wav(48_000, 1, 16, 4800, extensible = true) { _, _ -> 0.5 }))
        assertEquals(100L, ext.durationMs)
        assertEquals(4800, readAll(ext).size)
        val streamed = WavSource(ByteArrayInputStream(wav(16_000, 1, 16, 1600, streamed = true) { _, _ -> 0.1 }))
        assertNull(streamed.durationMs)
        assertEquals(1600, readAll(streamed).size)
    }

    @Test fun refusesWhatItCannotRead() {
        fun bad(b: ByteArray) = try { WavSource(ByteArrayInputStream(b)); fail("accepted") } catch (_: DecodeException) {}
        bad("not a wav at all".toByteArray())
        bad(wav(16_000, 1, 16, 100) { _, _ -> 0.0 }.copyOf(30)) // cut short
        val adpcm = wav(16_000, 1, 16, 100) { _, _ -> 0.0 }.also { it[12 + 12 + 8] = 2 } // format tag 2
        bad(adpcm)
        assertTrue(WavSource.isWav(wav(16_000, 1, 16, 1) { _, _ -> 0.0 }))
        assertFalse(WavSource.isWav("ftypM4A ....".toByteArray()))
    }

    @Test fun wavToSixteenKMonoEndToEnd() {
        val bytes = wav(44_100, 2, 16, 44_100) { f, _ -> 0.4 * sin(2 * PI * 440 * f / 44_100) }
        val s = WavSource(ByteArrayInputStream(bytes))
        val c = To16kMono()
        val out = ArrayList<Short>()
        while (true) { val b = s.next() ?: break; out += c.feed(b).toList() }
        out += c.flush().toList()
        assertEquals(16_000, out.size)
        assertEquals(0.4, magnitude(out.toShortArray().copyOfRange(2000, 14_000), 440.0), 0.02)
        assertTrue(abs(out.max().toInt()) < 32767)
    }
}
