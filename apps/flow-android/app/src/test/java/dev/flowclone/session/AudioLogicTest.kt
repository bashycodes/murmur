package dev.flowclone.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LevelMeterTest {
    @Test fun silenceIsZero() = assertEquals(0f, LevelMeter().update(ShortArray(1600), 1600), 0f)

    @Test fun fullScaleAfterQuietFloorIsOne() {
        val m = LevelMeter()
        m.update(ShortArray(1600) { 100 }, 1600) // ≈ -50 dBFS → floor clamps at -50
        val loud = m.update(ShortArray(1600) { if (it % 2 == 0) 20000 else -20000 }, 1600)
        assertEquals(1f, loud, 0.001f)
    }

    @Test fun floorStartsAtZeroDbSoFirstChunkReadsZero() {
        // bg starts at 0 dB and only follows the signal down: first chunk sets bg = db → level 0.
        assertEquals(0f, LevelMeter().update(ShortArray(1600) { 3000 }, 1600), 0.001f)
    }

    @Test fun midLevelIsProportionalTo30dbRange() {
        val m = LevelMeter()
        m.update(ShortArray(1600) { 104 }, 1600) // ≈ -50 dB floor
        val l = m.update(ShortArray(1600) { 1036 }, 1600) // ≈ -30 dB → (20/30)
        assertEquals(0.667f, l, 0.02f)
    }
}

class AudioBatcherTest {
    @Test fun emitsExact100msFrames() {
        val frames = mutableListOf<Int>()
        val b = AudioBatcher(1600) { pcm, n -> frames += n; assertEquals(1600, pcm.size) }
        b.feed(ShortArray(1000), 1000)
        assertTrue(frames.isEmpty())
        b.feed(ShortArray(2500), 2500)
        assertEquals(listOf(1600, 1600), frames)
        b.flush()
        assertEquals(listOf(1600, 1600, 300), frames)
    }

    @Test fun preservesSampleOrder() {
        val out = mutableListOf<Short>()
        val b = AudioBatcher(4) { pcm, n -> for (i in 0 until n) out += pcm[i] }
        b.feed(shortArrayOf(1, 2, 3), 3)
        b.feed(shortArrayOf(4, 5, 6, 7, 8, 9), 5)
        b.flush()
        assertEquals((1..8).map { it.toShort() }, out)
    }

    @Test fun flushOfEmptyBufferEmitsNothing() {
        var calls = 0
        AudioBatcher(1600) { _, _ -> calls++ }.flush()
        assertEquals(0, calls)
    }
}

class SessionCapTest {
    @Test fun defaultFiveMinuteCapWarnsAt239sStopsAt299s() {
        val c = SessionCap()
        assertEquals(239_000L, c.warnAtMs)
        assertEquals(299_000L, c.stopAtMs)
        assertEquals(300L * 16_000, c.maxSamples)
    }

    @Test fun configurableCap() {
        val c = SessionCap(capMs = 120_000)
        assertEquals(59_000L, c.warnAtMs)
        assertEquals(119_000L, c.stopAtMs)
    }
}
