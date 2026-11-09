package dev.murmur.ui

import android.Manifest
import android.provider.Settings
import android.view.accessibility.AccessibilityNodeInfo
import dev.murmur.MurmurApp
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowSettings

/** Permissions page (AndroidSetup): progress, callout, featured step, one card per step, OEM steps, a11y. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class PermissionsUiTest {
    private val app get() = RuntimeEnvironment.getApplication()
    private fun launch() = Robolectric.buildActivity(PermissionsActivity::class.java).setup().get()

    @After fun reset() { ShadowSettings.setCanDrawOverlays(false) }

    @Test fun everyStepIsThereWithProgressAndCallout() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("Permissions", "0 of 5 done", "Microphone", "Display over other apps", "Accessibility service", "Battery optimisation", "Notifications")) {
            assertTrue("$want in $t", want in t)
        }
        assertTrue(t.toString(), t.any { it.startsWith("Dictation can't start yet. Allow Microphone, Display over other apps and Accessibility service") })
        assertTrue(t.any { it.startsWith("Keep it running: ") })
        // The first missing required step gets the filled button.
        val featured = walk(a.window.decorView).first { it is android.widget.Button && it.contentDescription == "Grant Microphone" }
        featured.performClick()
        assertTrue(Manifest.permission.RECORD_AUDIO in shadowOf(a).lastRequestedPermission.requestedPermissions)
    }

    @Test fun statusCardKeepsTheEngineLinesAndLinksToSettings() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("STATUS", "Native engine", "Speech engine", "Language", "Version")) assertTrue("$want in $t", want in t)
        assertTrue(dev.murmur.data.AppVersion.current.label(MurmurApp.of(a).caps.flavor) in t)
        val lang = rowTitled(a, "Language")
        assertTrue(lang.contentDescription.toString(), lang.contentDescription.endsWith("Opens Language settings"))
        lang.performClick()
        val i = shadowOf(a).nextStartedActivity
        assertEquals(SettingsActivity::class.java.name, i.component?.className)
        assertEquals(dev.murmur.data.SettingsPages.Page.LANGUAGE.name, i.getStringExtra(SettingsActivity.EXTRA_PAGE))
    }

    @Test fun backButtonFinishes() {
        val a = launch()
        walk(a.window.decorView).first { it.contentDescription == "Back" }.performClick()
        assertTrue(a.isFinishing)
    }

    @Test fun grantedRowsShowTheirStateAndTheFeaturedStepMoves() {
        shadowOf(app).grantPermissions(Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS)
        ShadowSettings.setCanDrawOverlays(true)
        val a = launch()
        assertTrue(texts(a).toString(), "3 of 5 done" in texts(a))
        val mic = walk(a.window.decorView).first { it.isFocusable && it.contentDescription?.startsWith("Microphone,") == true }
        assertTrue(mic.contentDescription.toString(), mic.contentDescription.startsWith("Microphone, Granted"))
        assertFalse(mic.isClickable)
        // Accessibility is the one blocking step left: featured card with "Turn on".
        assertNotNull(walk(a.window.decorView).firstOrNull { it is android.widget.Button && it.contentDescription == "Turn on Accessibility service" })
        assertTrue(texts(a).any { it == "Dictation can't start yet. Allow Accessibility service below." })
    }

    @Test fun missingRowsRunTheirAction() {
        shadowOf(app).grantPermissions(Manifest.permission.RECORD_AUDIO)
        val a = launch()
        // Overlay is the featured step now; the others are rows.
        walk(a.window.decorView).first { it is android.widget.Button && it.contentDescription == "Allow Display over other apps" }.performClick()
        assertEquals(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, shadowOf(a).nextStartedActivity.action)
        rowTitled(a, "Battery optimisation").performClick()
        assertEquals(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, shadowOf(a).nextStartedActivity.action)
        rowTitled(a, "Notifications").performClick()
        assertTrue(Manifest.permission.POST_NOTIFICATIONS in shadowOf(a).lastRequestedPermission.requestedPermissions)
    }

    @Test fun oemGuidanceIsCollapsibleAndReadsAsExpandable() {
        val a = launch()
        val header = walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Keep it running: ") == true }
        assertTrue(header.createAccessibilityNodeInfo().actionList.contains(AccessibilityNodeInfo.AccessibilityAction.ACTION_EXPAND))
        assertEquals("Collapsed", header.stateDescription)
        header.performClick()
        assertEquals("Expanded", header.stateDescription)
    }

    @Test fun touchTargetsAreAtLeast48dp() {
        val a = launch()
        walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Keep it running: ") == true }.performClick()
        UiTestKit.assertTouchTargets(a, "permissions")
    }
}
