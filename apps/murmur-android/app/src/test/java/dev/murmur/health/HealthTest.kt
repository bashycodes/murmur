package dev.murmur.health

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class A11ySettingTest {
    private val me = "dev.murmur/dev.murmur.service.MurmurAccessibilityService"

    @Test fun parsesFullAndShortComponentNames() {
        assertTrue(A11ySetting.isEnabled("com.other/.Svc:$me", "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
        assertTrue(A11ySetting.isEnabled("dev.murmur/.service.MurmurAccessibilityService", "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
        assertTrue(A11ySetting.isEnabled(me.uppercase(), "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
    }

    @Test fun absentOrEmpty() {
        assertFalse(A11ySetting.isEnabled(null, "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
        assertFalse(A11ySetting.isEnabled("", "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
        assertFalse(A11ySetting.isEnabled("dev.murmur.evil/.service.MurmurAccessibilityService", "dev.murmur", "dev.murmur.service.MurmurAccessibilityService"))
    }
}

class AccessibilityHealthTest {
    private fun h(enabled: Boolean, connected: Boolean, connectedAt: Long? = 0, beat: Long? = null, now: Long = 100_000, start: Long = 0, screenOn: Boolean = true) =
        AccessibilityHealth.evaluate(
            AccessibilityHealth.Input(
                enabledInSettings = enabled, connected = connected, processStartAt = start,
                connectedAt = connectedAt, lastHeartbeatAt = beat, screenOn = screenOn, now = now,
            ),
        )

    @Test fun disabled() = assertEquals(AccessibilityHealth.Status.DISABLED, h(enabled = false, connected = false))

    @Test fun healthyWithFreshHeartbeat() =
        assertEquals(AccessibilityHealth.Status.OK, h(enabled = true, connected = true, beat = 90_000))

    @Test fun enabledButNeverConnectedAfterGraceIsGhost() {
        assertEquals(AccessibilityHealth.Status.STARTING, h(true, false, connectedAt = null, now = 5_000))
        assertEquals(AccessibilityHealth.Status.GHOST, h(true, false, connectedAt = null, now = 20_000))
    }

    @Test fun connectedButStaleHeartbeatWhileScreenOnIsGhost() {
        assertEquals(AccessibilityHealth.Status.GHOST, h(true, true, connectedAt = 0, beat = 10_000, now = 100_000))
        assertEquals(AccessibilityHealth.Status.OK, h(true, true, connectedAt = 0, beat = 10_000, now = 100_000, screenOn = false))
    }

    @Test fun freshlyConnectedHasGraceBeforeFirstBeat() {
        assertEquals(AccessibilityHealth.Status.OK, h(true, true, connectedAt = 90_000, beat = null, now = 100_000))
        assertEquals(AccessibilityHealth.Status.GHOST, h(true, true, connectedAt = 0, beat = null, now = 100_000))
    }

    @Test fun connectedButSettingOffIsDisabled() =
        assertEquals(AccessibilityHealth.Status.DISABLED, h(enabled = false, connected = true, beat = 99_000))

    private fun hp(beat: Long?, probe: (() -> Boolean)?, now: Long = 100_000): AccessibilityHealth.Status =
        AccessibilityHealth.evaluate(
            AccessibilityHealth.Input(
                enabledInSettings = true, connected = true, processStartAt = 0, connectedAt = 0,
                lastHeartbeatAt = beat, screenOn = true, now = now, liveProbe = probe,
            ),
        )

    /**
     * Device bug (S25+): the heartbeat is stamped by a 15 s uptime timer, so after the CPU
     * sleeps the first check sees a > 60 s old beat although the service is bound and idle.
     * A live probe that answers means OK.
     */
    @Test fun staleHeartbeatButLiveProbeAnswersIsOk() =
        assertEquals(AccessibilityHealth.Status.OK, hp(beat = 10_000, probe = { true }))

    @Test fun staleHeartbeatAndFailedProbeIsGhost() =
        assertEquals(AccessibilityHealth.Status.GHOST, hp(beat = 10_000, probe = { false }))

    @Test fun probeThatThrowsCountsAsFailed() =
        assertEquals(AccessibilityHealth.Status.GHOST, hp(beat = 10_000, probe = { error("DeadObjectException") }))

    @Test fun freshHeartbeatDoesNotProbe() {
        var probed = false
        assertEquals(AccessibilityHealth.Status.OK, hp(beat = 95_000, probe = { probed = true; false }))
        assertFalse(probed)
    }
}

class AccessibilityHealthMonitorTest {
    private val G = AccessibilityHealth.Status.GHOST
    private val OK = AccessibilityHealth.Status.OK

    @Test fun singleGhostReadingIsNotReported() {
        val m = AccessibilityHealth.Monitor()
        assertEquals(OK, m.update(OK, 0))
        assertEquals(OK, m.update(G, 15_000))
        assertEquals(OK, m.update(OK, 15_500)) // the S25+ flap: GHOST then OK 0.5 s later never surfaces
    }

    @Test fun ghostNeedsTwoConsecutiveReadingsSpanningConfirmWindow() {
        val m = AccessibilityHealth.Monitor()
        assertEquals(OK, m.update(OK, 0))
        assertEquals(OK, m.update(G, 15_000))
        assertEquals(OK, m.update(G, 17_000)) // 2 readings, but only 2 s apart (MainActivity's 2 s refresh)
        assertEquals(G, m.update(G, 15_000 + AccessibilityHealth.GHOST_CONFIRM_MS))
        assertEquals(G, m.update(G, 40_000))
        assertEquals(OK, m.update(OK, 41_000)) // recovery is immediate
    }

    @Test fun okResetsTheGhostStreak() {
        val m = AccessibilityHealth.Monitor()
        m.update(G, 0)
        m.update(OK, 5_000)
        assertEquals(OK, m.update(G, 30_000))
    }

    @Test fun nonGhostStatusesPassThroughImmediately() {
        val m = AccessibilityHealth.Monitor()
        assertEquals(AccessibilityHealth.Status.DISABLED, m.update(AccessibilityHealth.Status.DISABLED, 0))
        assertEquals(AccessibilityHealth.Status.STARTING, m.update(AccessibilityHealth.Status.STARTING, 1))
    }

    @Test fun unconfirmedGhostAtStartupReadsAsStarting() {
        val m = AccessibilityHealth.Monitor()
        assertEquals(AccessibilityHealth.Status.STARTING, m.update(G, 20_000))
        assertEquals(G, m.update(G, 20_000 + AccessibilityHealth.GHOST_CONFIRM_MS))
    }
}

class HealthChangeTest {
    @Test fun onlyAChangeRepostsTheNotification() {
        val c = AccessibilityHealth.ChangeGate()
        assertFalse(c.changed(AccessibilityHealth.Status.OK)) // initial OK == default
        assertTrue(c.changed(AccessibilityHealth.Status.GHOST))
        assertFalse(c.changed(AccessibilityHealth.Status.GHOST))
        assertTrue(c.changed(AccessibilityHealth.Status.OK))
        assertFalse(c.changed(AccessibilityHealth.Status.OK))
    }
}

class ServiceWatchdogTest {
    @Test fun restartsWhenWantedAndDownAtMostEvery5s() {
        val w = ServiceWatchdog()
        assertTrue(w.shouldRestart(wanted = true, running = false, now = 10_000))
        assertFalse(w.shouldRestart(true, false, 12_000))
        assertTrue(w.shouldRestart(true, false, 15_001))
    }

    @Test fun noRestartWhenRunningOrUnwanted() {
        val w = ServiceWatchdog()
        assertFalse(w.shouldRestart(wanted = true, running = true, now = 10_000))
        assertFalse(w.shouldRestart(wanted = false, running = false, now = 10_000))
    }
}

class OemGuidanceTest {
    @Test fun xiaomiHasAutostartScreen() {
        val g = OemGuidance.forManufacturer("Xiaomi")
        assertTrue(g.steps.contains("Autostart"))
        assertTrue(g.intents.any { it.packageName == "com.miui.securitycenter" })
    }

    @Test fun redmiAndPocoMapToXiaomi() {
        assertEquals(OemGuidance.forManufacturer("Xiaomi").title, OemGuidance.forManufacturer("Redmi").title)
        assertEquals(OemGuidance.forManufacturer("Xiaomi").title, OemGuidance.forManufacturer("POCO").title)
    }

    @Test fun samsungAndHuaweiAndOppo() {
        assertTrue(OemGuidance.forManufacturer("samsung").steps.contains("Never sleeping apps"))
        assertTrue(OemGuidance.forManufacturer("HUAWEI").steps.contains("App launch"))
        assertTrue(OemGuidance.forManufacturer("OPPO").steps.contains("Auto launch") || OemGuidance.forManufacturer("OPPO").steps.contains("Auto-launch"))
    }

    @Test fun pixelNeedsNothingSpecial() {
        val g = OemGuidance.forManufacturer("Google")
        assertTrue(g.intents.isEmpty())
        assertNull(g.extraWarning)
    }
}
