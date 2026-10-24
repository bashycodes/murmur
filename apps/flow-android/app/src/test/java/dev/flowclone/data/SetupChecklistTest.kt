package dev.flowclone.data

import dev.flowclone.data.SetupChecklist.Item
import dev.flowclone.data.SetupChecklist.Need
import dev.flowclone.health.AccessibilityHealth.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Main screen SETUP card: one row per permission in Wispr's order, the problem banner on top. */
class SetupChecklistTest {
    private val allOk = SetupChecklist.State(mic = true, overlay = true, a11y = Status.OK, batteryExempt = true, notifications = true)

    @Test fun rowsComeInOnboardingOrder() {
        assertEquals(listOf(Item.MIC, Item.OVERLAY, Item.ACCESSIBILITY, Item.BATTERY, Item.NOTIFICATIONS), SetupChecklist.rows(allOk).map { it.item })
        assertEquals(
            listOf("Microphone", "Display over other apps", "Accessibility service", "Battery optimisation", "Notifications"),
            SetupChecklist.rows(allOk).map { it.title },
        )
    }

    @Test fun grantedRowsShowTheirStateAndNoAction() {
        val r = SetupChecklist.rows(allOk)
        assertEquals(listOf("Granted", "Allowed", "On", "Unrestricted", "Allowed"), r.map { it.state })
        assertTrue(r.all { it.ok && it.action == null })
        assertNull(SetupChecklist.banner(allOk))
        assertEquals("All 5 done", SetupChecklist.progress(allOk))
        assertFalse(SetupChecklist.setupFirst(allOk))
    }

    @Test fun missingRowsOfferTheirAction() {
        val s = SetupChecklist.State(mic = false, overlay = false, a11y = Status.DISABLED, batteryExempt = false, notifications = false)
        val r = SetupChecklist.rows(s).associateBy { it.item }
        assertEquals("Grant", r.getValue(Item.MIC).action)
        assertEquals("Allow", r.getValue(Item.OVERLAY).action)
        assertEquals("Turn on", r.getValue(Item.ACCESSIBILITY).action)
        assertEquals("Allow", r.getValue(Item.BATTERY).action)
        assertEquals("Allow", r.getValue(Item.NOTIFICATIONS).action)
        assertEquals(Need.REQUIRED, r.getValue(Item.MIC).need)
        assertEquals(Need.RECOMMENDED, r.getValue(Item.BATTERY).need)
        assertEquals(Need.OPTIONAL, r.getValue(Item.NOTIFICATIONS).need)
        assertEquals("0 of 5 done", SetupChecklist.progress(s))
    }

    /** The prominent disclosure (Play policy wording) must stay on the main screen while a11y is off. */
    @Test fun accessibilityOffKeepsTheFullDisclosure() {
        val r = SetupChecklist.rows(allOk.copy(a11y = Status.DISABLED)).first { it.item == Item.ACCESSIBILITY }
        assertTrue(r.subtitle, "read up to 500 characters around the cursor" in r.subtitle)
        assertTrue("never reads or types into password" in r.subtitle)
        assertTrue("Allow restricted settings" in r.subtitle)
        assertEquals("Off", r.state)
    }

    @Test fun ghostAndStartingAccessibility() {
        val ghost = SetupChecklist.rows(allOk.copy(a11y = Status.GHOST)).first { it.item == Item.ACCESSIBILITY }
        assertEquals("Not responding", ghost.state); assertEquals("Fix", ghost.action)
        assertTrue("OFF and then ON again" in ghost.subtitle)
        val starting = SetupChecklist.rows(allOk.copy(a11y = Status.STARTING)).first { it.item == Item.ACCESSIBILITY }
        assertEquals("Connecting…", starting.state); assertNull(starting.action)
        // Connecting is transient: no banner, no reordering.
        assertNull(SetupChecklist.banner(allOk.copy(a11y = Status.STARTING)))
    }

    @Test fun micSubtitleMentionsTheDebugAudioException() {
        val on = SetupChecklist.rows(allOk.copy(mic = false, keepDebugAudio = true)).first()
        assertTrue(on.subtitle, "Keep last dictation audio" in on.subtitle)
        val off = SetupChecklist.rows(allOk.copy(mic = false)).first()
        assertFalse("Keep last dictation audio" in off.subtitle)
    }

    @Test fun bannerNamesRequiredStepsFirstThenRecommended() {
        val s = allOk.copy(mic = false, a11y = Status.DISABLED, batteryExempt = false, notifications = false)
        val b = SetupChecklist.banner(s)!!
        assertTrue(b.blocking)
        assertEquals("Dictation can't start yet: allow Microphone and Accessibility service below.", b.text)
        assertTrue(SetupChecklist.setupFirst(s))
        val onlyBattery = SetupChecklist.banner(allOk.copy(batteryExempt = false))!!
        assertFalse(onlyBattery.blocking)
        assertTrue(onlyBattery.text, onlyBattery.text.startsWith("Recommended: Battery optimisation"))
        assertFalse(SetupChecklist.setupFirst(allOk.copy(batteryExempt = false)))
        // Notifications are optional: no banner for them alone.
        assertNull(SetupChecklist.banner(allOk.copy(notifications = false)))
    }

    @Test fun ghostBannerSaysItStopped() {
        val b = SetupChecklist.banner(allOk.copy(a11y = Status.GHOST))!!
        assertTrue(b.blocking)
        assertTrue(b.text, "stopped responding" in b.text)
    }

    @Test fun announcementsOnlyForNewlyGrantedItems() {
        val before = allOk.copy(mic = false, overlay = false)
        assertEquals(listOf("Microphone granted"), SetupChecklist.newlyDone(before, before.copy(mic = true)))
        assertEquals(emptyList<String>(), SetupChecklist.newlyDone(null, allOk))
        assertEquals(emptyList<String>(), SetupChecklist.newlyDone(allOk, before))
    }
}
