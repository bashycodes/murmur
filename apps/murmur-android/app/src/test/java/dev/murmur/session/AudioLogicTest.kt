package dev.murmur.session

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

/** D21 `endpoint_wait_ms`: trailing silence is measured against the adaptive noise floor (as core vad.rs). */
class SilenceTrackerTest {
    private fun SilenceTracker.feed(ms: Int, sample: (Int) -> Int) =
        repeat(ms / 100) { update(ShortArray(1600) { i -> sample(i).toShort().toInt().toShort() }, 1600) }
    private fun noise(amp: Int, seed: Long = 3): (Int) -> Int { val r = java.util.Random(seed); return { (r.nextGaussian() * amp).toInt() } }

    @Test fun roomNoiseAfterSpeechIsTrailingSilence() {
        val t = SilenceTracker()
        t.feed(300, noise(300)) // phone room noise ≈ −41 dBFS, above a fixed −45 dBFS bar
        t.feed(1000) { 3000 }
        t.feed(500, noise(300))
        assertEquals(500L, t.trailingSilenceMs)
    }

    @Test fun digitalSilenceAfterSpeechIsTrailingSilence() {
        val t = SilenceTracker()
        t.feed(800) { 3000 }
        t.feed(400) { 0 }
        assertEquals(400L, t.trailingSilenceMs)
    }

    @Test fun speechWithoutLeadingSilenceIsNotTrailingSilence() {
        // Floor estimate = speech itself; the −36 dBFS cap keeps clear speech as speech (vad.rs ABS_CAP).
        val t = SilenceTracker()
        t.feed(2000) { 3000 }
        assertEquals(0L, t.trailingSilenceMs)
    }

    @Test fun speechResetsTrailingSilence() {
        val t = SilenceTracker()
        t.feed(500) { 3000 }
        t.feed(600, noise(300))
        t.feed(200) { 3000 }
        t.feed(300, noise(300, seed = 9))
        assertEquals(300L, t.trailingSilenceMs)
    }

    @Test fun noSpeechYetHintKeepsTheFixedBar() {
        val t = SilenceTracker()
        t.feed(3000) { 100 } // RMS 100 < 184: "can't hear you"
        assertEquals(false, t.heardSpeech)
        assertEquals(3000L, t.leadingSilenceMs)
        t.feed(100) { 300 }  // RMS 300 > 184: signal for the hint, whatever the floor says
        assertEquals(true, t.heardSpeech)
    }
}

class StartGateTest {
    private fun frame(i: Int) = ShortArray(1600) { i.toShort() }

    @Test fun heldFramesReplayInOrderThenPassThrough() {
        val g = StartGate()
        repeat(3) { g.push(frame(it), 1600) }
        val got = mutableListOf<Int>()
        g.open { pcm, _ -> got += pcm[0].toInt() }
        g.push(frame(3), 1600)
        assertEquals(listOf(0, 1, 2, 3), got)
    }

    /**
     * D55: minutes of held audio replay outside the gate's lock, so the microphone thread
     * keeps pushing during the replay (it never blocks on it), and nothing is reordered or lost.
     */
    @Test fun theMicrophoneNeverWaitsForALongReplay() {
        val held = 1500 // 150 s of 100 ms frames
        val g = StartGate(held * 1600 + 16_000)
        repeat(held) { g.push(frame(it % 30_000), 1600) }
        val got = java.util.Collections.synchronizedList(mutableListOf<Int>())
        val micPushed = java.util.concurrent.CountDownLatch(1)
        val mic = Thread {
            // Runs while the replay below is stuck on its first frame: never waits for it.
            g.push(frame(held), 1600)
            micPushed.countDown()
        }
        var first = true
        g.open { pcm, _ ->
            if (first) {
                first = false
                mic.start()
                assertTrue("push returned during the replay", micPushed.await(5, java.util.concurrent.TimeUnit.SECONDS))
            }
            got += pcm[0].toInt()
        }
        mic.join()
        assertEquals((0..held).map { it % 30_000 }, got.toList())
        assertEquals(0, g.droppedSamples)
    }
}
