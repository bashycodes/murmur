package dev.murmur.bubble

import dev.murmur.session.DictationState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ShakeDetectorTest {
    private val g = 9.81f
    private fun still(d: ShakeDetector, t: Long) = d.onSample(0f, 0f, g, t)
    private fun jolt(d: ShakeDetector, t: Long) = d.onSample(0f, 0f, 2.2f * g, t) // |a|/g − 1 = 1.2 > 1.0

    @Test fun twoImpulsesWithin1200msTrigger() {
        val d = ShakeDetector()
        assertFalse(jolt(d, 1000))
        assertFalse(still(d, 1100))
        assertTrue(jolt(d, 1900))
    }

    @Test fun impulsesFurtherApartThan1200msDoNotTrigger() {
        val d = ShakeDetector()
        assertFalse(jolt(d, 1000))
        assertFalse(jolt(d, 2300))
        assertTrue(jolt(d, 3000)) // chained from the 2300 impulse
    }

    @Test fun cooldownSwallowsTheSameJolt() {
        val d = ShakeDetector()
        assertFalse(jolt(d, 1000))
        assertFalse(jolt(d, 1050)) // < 100 ms cooldown: same physical impulse
        assertTrue(jolt(d, 1200))
    }

    @Test fun gentleMotionIsIgnored() {
        val d = ShakeDetector()
        repeat(20) { assertFalse(d.onSample(0f, 0f, 1.9f * g, 1000L + it * 150)) }
    }

    @Test fun resetsAfterTrigger() {
        val d = ShakeDetector()
        jolt(d, 1000); assertTrue(jolt(d, 1300))
        assertFalse(jolt(d, 1500))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class BubblePresenceTest {
    private fun TestScope.presence(minimise: Boolean = true) =
        BubblePresence(backgroundScope, minimiseEnabled = { minimise })

    @Test fun readyBubbleMinimisesAfter5s() = runTest {
        val p = presence()
        p.onState(DictationState.Ready); runCurrent()
        advanceTimeBy(4_999); runCurrent()
        assertFalse(p.presence.value.minimized)
        advanceTimeBy(2); runCurrent()
        assertTrue(p.presence.value.minimized)
    }

    @Test fun minimiseDisabledNeverShrinks() = runTest {
        val p = presence(minimise = false)
        p.onState(DictationState.Ready)
        advanceTimeBy(60_000); runCurrent()
        assertFalse(p.presence.value.minimized)
    }

    @Test fun touchRestoresAndRestartsTimer() = runTest {
        val p = presence()
        p.onState(DictationState.Ready)
        advanceTimeBy(6_000); runCurrent()
        assertTrue(p.touch())
        assertFalse(p.presence.value.minimized)
        assertFalse(p.touch()) // already full size
        advanceTimeBy(5_001); runCurrent()
        assertTrue(p.presence.value.minimized)
    }

    @Test fun recordingNeverMinimised() = runTest {
        val p = presence()
        p.onState(DictationState.Ready)
        advanceTimeBy(6_000); runCurrent()
        p.onState(DictationState.Recording(hold = false, startedAt = 0))
        assertFalse(p.presence.value.minimized)
        advanceTimeBy(60_000); runCurrent()
        assertFalse(p.presence.value.minimized)
    }

    @Test fun snoozeHidesFor10MinutesThenReturns() = runTest {
        val p = presence()
        p.onState(DictationState.Ready)
        assertTrue(p.snooze())
        assertTrue(p.presence.value.snoozed)
        assertFalse(p.visible(DictationState.Ready))
        advanceTimeBy(BubblePresence.SNOOZE_MS - 1); runCurrent()
        assertTrue(p.presence.value.snoozed)
        advanceTimeBy(2); runCurrent()
        assertFalse(p.presence.value.snoozed)
        assertTrue(p.visible(DictationState.Ready))
    }

    @Test fun snoozeOnlyFromReady() = runTest {
        val p = presence()
        p.onState(DictationState.Recording(false, 0))
        assertFalse(p.snooze())
        p.onState(DictationState.Error(DictationState.ErrorKind.FailedToTranscribe))
        assertFalse(p.snooze())
        p.onState(DictationState.Ready)
        p.repair = true
        assertFalse(p.snooze()) // refused in Repair
    }

    @Test fun shakeWhileSnoozedUnsnoozes() = runTest {
        val p = presence()
        p.onState(DictationState.Ready); p.snooze()
        assertEquals(BubblePresence.ShakeOutcome.UNSNOOZED, p.onShake(locked = false))
        assertFalse(p.presence.value.snoozed)
    }

    @Test fun shakeOnLockScreenIgnored() = runTest {
        val p = presence()
        p.onState(DictationState.Ready); p.snooze()
        assertEquals(BubblePresence.ShakeOutcome.IGNORED, p.onShake(locked = true))
        assertTrue(p.presence.value.snoozed)
    }

    @Test fun shakeWhenIdleRevealsFor10s() = runTest {
        val p = presence()
        p.onState(DictationState.Idle)
        assertFalse(p.visible(DictationState.Idle))
        assertEquals(BubblePresence.ShakeOutcome.REVEALED, p.onShake(locked = false))
        assertTrue(p.visible(DictationState.Idle))
        advanceTimeBy(BubblePresence.REVEAL_MS + 1); runCurrent()
        assertFalse(p.visible(DictationState.Idle))
    }

    @Test fun shakeWhenReadyDoesNothing() = runTest {
        val p = presence()
        p.onState(DictationState.Ready)
        assertEquals(BubblePresence.ShakeOutcome.IGNORED, p.onShake(locked = false))
    }

    @Test fun activeDictationIsAlwaysVisibleEvenIfSnoozedEarlier() = runTest {
        val p = presence()
        p.onState(DictationState.Ready); p.snooze()
        assertTrue(p.visible(DictationState.Processing(false, 0)))
    }

    @Test fun repairIsVisibleWithoutField() = runTest {
        val p = presence()
        p.repair = true
        assertTrue(p.visible(DictationState.Idle))
    }
}

class SnoozeTargetTest {
    @Test fun nearWithin100dp() {
        val t = SnoozeTarget(screenW = 1080, screenH = 2400, density = 3f)
        val (cx, cy) = t.center()
        assertEquals(540f, cx, 0.1f)
        assertTrue(t.isNear(cx + 299f, cy))
        assertFalse(t.isNear(cx + 301f, cy))
    }

    @Test fun windowHeightIs73dpPlus7Percent() {
        val t = SnoozeTarget(screenW = 1080, screenH = 2000, density = 2f)
        assertEquals(146 + 140, t.windowHeightPx())
    }
}
