package dev.flowclone.audio

import dev.flowclone.debug.Wav
import dev.flowclone.session.AudioBatcher
import dev.flowclone.session.AudioSource
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Debug "WAV as microphone": the WAV's PCM reaches the controller exactly as mic frames would. */
class WavAudioSourceTest {
    private val pcm = ShortArray(16_000 * 2 + 700) { ((it * 37) % 20_000 - 10_000).toShort() } // 2.044 s

    /** What MicRecorder hands the controller for the same samples: AudioBatcher's 100 ms frames + the flushed tail. */
    private fun micFrames(samples: ShortArray): List<ShortArray> {
        val out = mutableListOf<ShortArray>()
        val b = AudioBatcher { f, n -> out += f.copyOf(n) }
        var off = 0
        while (off < samples.size) { // AudioRecord read size (getMinBufferSize / 2 = 640 on many phones)
            val n = minOf(640, samples.size - off)
            b.feed(samples.copyOfRange(off, off + n), n); off += n
        }
        b.flush()
        return out
    }

    private fun run(src: AudioSource, ended: CountDownLatch): List<ShortArray> {
        val frames = java.util.Collections.synchronizedList(mutableListOf<ShortArray>())
        assertTrue(src.start({ f, n -> frames += f.copyOf(n) }, { fail("no error expected") }))
        assertTrue("WAV end not signalled", ended.await(5, TimeUnit.SECONDS))
        src.stop()
        return frames.toList()
    }

    @Test fun decodeReadsWhatEncodeWrote() {
        assertArrayEquals(pcm, Wav.decode(Wav.encode(pcm)))
    }

    @Test fun decodeSkipsExtraChunksAndAnOddPad() {
        val fmt = Wav.encode(shortArrayOf()).copyOfRange(12, 36) // the "fmt " chunk
        val data = Wav.encode(shortArrayOf(1, 2, 3)).copyOfRange(36, 50)
        val body = ByteArrayOutputStream().apply {
            write("WAVE".toByteArray())
            write("LIST".toByteArray()); write(le(3)); write(byteArrayOf(9, 9, 9, 0)) // odd size + pad byte
            write(fmt)
            write(data)
        }.toByteArray()
        val wav = "RIFF".toByteArray() + le(body.size) + body
        assertArrayEquals(shortArrayOf(1, 2, 3), Wav.decode(wav))
    }

    @Test fun decodeRefusesAnythingButSixteenKilohertzMonoPcm16() {
        fun patched(at: Int, v: Int, short: Boolean) = Wav.encode(pcm).also {
            val b = ByteBuffer.wrap(it).order(ByteOrder.LITTLE_ENDIAN)
            if (short) b.putShort(at, v.toShort()) else b.putInt(at, v)
        }
        val bad = mapOf(
            "stereo" to patched(22, 2, true),
            "44.1 kHz" to patched(24, 44_100, false),
            "8-bit" to patched(34, 8, true),
            "float" to patched(20, 3, true),
            "not RIFF" to "garbage that is not a wav file at all, honestly".toByteArray(),
            "truncated" to Wav.encode(pcm).copyOf(30),
        )
        for ((name, bytes) in bad) {
            try { Wav.decode(bytes); fail("$name accepted") } catch (e: IllegalArgumentException) {
                assertTrue("$name: message ${e.message}", !e.message.isNullOrBlank())
            }
        }
    }

    @Test fun feedsTheSameFramesTheMicPathWouldThenSignalsTheEnd() {
        val ended = CountDownLatch(1)
        val frames = run(WavAudioSource(pcm, realTime = false, onEnd = { ended.countDown() }), ended)
        val want = micFrames(pcm)
        assertEquals(want.size, frames.size)
        for (i in want.indices) assertArrayEquals("frame $i", want[i], frames[i])
        assertEquals(21, frames.size)
        assertEquals(700, frames.last().size)
    }

    @Test fun realTimePacesOneFrameEvery100msOfAudio() {
        var now = 1_000L
        val sleeps = mutableListOf<Long>()
        val delivered = mutableListOf<Long>()
        val ended = CountDownLatch(1)
        val src = WavAudioSource(
            pcm, realTime = true, onEnd = { ended.countDown() },
            clockMs = { now }, sleepMs = { sleeps += it; now += it },
        )
        src.start({ _, _ -> delivered += now }, {})
        assertTrue(ended.await(5, TimeUnit.SECONDS))
        src.stop()
        // A frame is delivered once its audio "has been spoken": 100, 200, … ms after the start; the tail at 2.044 s.
        assertEquals((1..20).map { 1_000L + it * 100 } + (1_000L + 2_044), delivered)
        assertEquals(2_044L, sleeps.sum())
    }

    @Test fun stopMidFileEndsTheFramesWithoutSignallingTheEnd() {
        val first = CountDownLatch(1)
        val ended = java.util.concurrent.atomic.AtomicBoolean(false)
        val src = WavAudioSource(pcm, realTime = true, onEnd = { ended.set(true) })
        val count = java.util.concurrent.atomic.AtomicInteger()
        src.start({ _, _ -> count.incrementAndGet(); first.countDown() }, {})
        assertTrue(first.await(2, TimeUnit.SECONDS))
        src.stop()
        val after = count.get()
        Thread.sleep(250)
        assertEquals("no frame after stop", after, count.get())
        assertTrue(after < 21)
        assertFalse(ended.get())
    }

    @Test fun sourceIsLabelledWav() = assertEquals("wav", WavAudioSource(pcm).source)

    private class Fake(override val source: String) : AudioSource {
        var starts = 0; var stops = 0
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { starts++; return true }
        override fun stop() { stops++ }
    }

    @Test fun switchUsesAnArmedSourceForOneStartOnly() {
        val mic = Fake("mic"); val wav = Fake("wav")
        val sw = AudioSwitch(mic)
        assertEquals("mic", sw.source)
        sw.arm(wav)
        assertTrue(sw.start({ _, _ -> }, {}))
        assertEquals("wav", sw.source)
        assertFalse("used, nothing left armed", sw.disarm())
        sw.stop()
        assertEquals(1, wav.stops)
        assertTrue(sw.start({ _, _ -> }, {}))
        assertEquals("next dictation is the mic again", "mic", sw.source)
        assertEquals(1, mic.starts); assertEquals(1, wav.starts)
    }

    @Test fun disarmDropsAnUnusedSourceSoTheNextMicDictationIsNotHijacked() {
        val mic = Fake("mic"); val wav = Fake("wav")
        val sw = AudioSwitch(mic)
        sw.arm(wav)
        assertTrue(sw.disarm())
        sw.start({ _, _ -> }, {})
        assertEquals(1, mic.starts); assertEquals(0, wav.starts)
    }

    private fun le(v: Int) = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(v).array()
}
