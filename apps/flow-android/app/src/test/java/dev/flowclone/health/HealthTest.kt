package dev.flowclone.health

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class A11ySettingTest {
    private val me = "dev.flowclone/dev.flowclone.service.FlowAccessibilityService"

    @Test fun parsesFullAndShortComponentNames() {
        assertTrue(A11ySetting.isEnabled("com.other/.Svc:$me", "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
        assertTrue(A11ySetting.isEnabled("dev.flowclone/.service.FlowAccessibilityService", "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
        assertTrue(A11ySetting.isEnabled(me.uppercase(), "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
    }

    @Test fun absentOrEmpty() {
        assertFalse(A11ySetting.isEnabled(null, "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
        assertFalse(A11ySetting.isEnabled("", "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
        assertFalse(A11ySetting.isEnabled("dev.flowclone.evil/.service.FlowAccessibilityService", "dev.flowclone", "dev.flowclone.service.FlowAccessibilityService"))
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
