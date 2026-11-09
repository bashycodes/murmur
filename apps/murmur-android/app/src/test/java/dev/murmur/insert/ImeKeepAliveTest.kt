package dev.murmur.insert

import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ImeKeepAliveTest {
    private class Deps : ImeKeepAlive.Deps {
        val field = FakeField()
        var skip = false
        var epoch = 0L
        var nudgeOk = true
        var nudges = 0
        var probes = mutableListOf<Long>()
        var probeCostMs = 0L
        var clockSkew = 0L
        lateinit var clock: () -> Long
        override fun shouldSkip() = skip
        override fun connection(): InputConnectionPort = object : InputConnectionPort by field {
            override fun getSurroundingText(before: Int, after: Int): SurroundingText? {
                probes += clock(); clockSkew += probeCostMs
                return field.getSurroundingText(before, after)
            }
        }
        override fun inputSessionEpoch() = epoch
        override fun onStaleTick(): Boolean { nudges++; return nudgeOk }
    }

    private fun TestScope.keepAlive(d: Deps): ImeKeepAlive {
        d.clock = { currentTime }
        return ImeKeepAlive(d) { currentTime + d.clockSkew }
    }

    @Test fun firstTickAfter1sThenEvery10s() = runTest {
        val d = Deps(); val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(21_001)
        assertEquals(listOf(1_000L, 11_000L, 21_000L), d.probes)
        k.disarm()
    }

    @Test fun nullReadNudgesOncePerEpochAndRetriesAfter1s() = runTest {
        val d = Deps(); d.field.surroundingNull = true
        val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(2_001)
        assertEquals(listOf(1_000L, 2_000L), d.probes)
        assertEquals("one nudge per epoch", 1, d.nudges)
        advanceTimeBy(10_000)
        assertEquals(1, d.nudges)
        d.epoch++ // onStartInput after the nudge
        advanceTimeBy(10_000)
        assertTrue(k.recovered)
        k.disarm()
    }

    @Test fun skippedTicksDoNotProbe() = runTest {
        val d = Deps(); d.skip = true
        val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(30_000)
        assertTrue(d.probes.isEmpty())
        assertEquals(3, k.skipped)
        k.disarm()
    }

    @Test fun slowProbesBackOffUpTo40s() = runTest {
        val d = Deps(); d.probeCostMs = 600
        val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(200_000)
        val gaps = d.probes.zipWithNext { a, b -> b - a }
        assertEquals(listOf(20_000L, 40_000L, 40_000L, 40_000L), gaps.take(4))
        k.disarm()
    }

    @Test fun disarmStopsTicks() = runTest {
        val d = Deps(); val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(1_001)
        k.disarm()
        advanceTimeBy(60_000)
        assertEquals(1, d.probes.size)
    }

    @Test fun armIsIdempotent() = runTest {
        val d = Deps(); val k = keepAlive(d)
        k.arm(backgroundScope); k.arm(backgroundScope)
        advanceTimeBy(1_001)
        assertEquals(1, d.probes.size)
        k.disarm()
    }

    @Test fun ttlDisarmsAfter15Minutes() = runTest {
        val d = Deps(); val k = keepAlive(d)
        k.arm(backgroundScope)
        advanceTimeBy(1_000_000)
        runCurrent()
        assertTrue(d.probes.last() <= 900_000 + 10_000)
        assertTrue(!k.isArmed)
    }
}
