package dev.flowclone.ui

import android.Manifest
import android.content.Intent
import android.provider.Settings
import android.view.accessibility.AccessibilityNodeInfo
import dev.flowclone.FlowApp
import dev.flowclone.data.AppVersion
import dev.flowclone.data.SettingsPages
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
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

/** Main screen in the Settings look: banner + SETUP first while a permission is missing, state rows, links, a11y. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class MainUiTest {
    private val app get() = RuntimeEnvironment.getApplication()
    private fun launch() = Robolectric.buildActivity(MainActivity::class.java).setup()

    @After fun reset() { ShadowSettings.setCanDrawOverlays(false) }

    @Test fun everySectionAndRowIsThere() {
        val a = launch().get()
        val t = texts(a)
        for (want in listOf(
            "STATUS", "SETUP", "RUN", "MORE", "Native engine", "Speech engine", "Language", "Version",
            "Microphone", "Display over other apps", "Accessibility service", "Battery optimisation", "Notifications",
            "Start / show bubble", "Stop bubble", "Settings", "Scratchpad", "Insights", "Dictionary & snippets", "History",
        )) assertTrue(want, want in t)
        assertTrue(t.toString(), t.any { it.startsWith("Keep it running: ") })
        assertTrue(AppVersion.current.label(FlowApp.of(a).caps.flavor) in t)
        // `remote` has no on-device model import.
        assertEquals(FlowApp.of(a).caps.local, "Speech model" in t)
        assertEquals(FlowApp.of(a).dev.visible, "Developer settings" in t)
    }

    @Test fun missingPermissionsShowABannerAndPutSetupFirst() {
        val a = launch().get()
        val t = texts(a)
        assertTrue(t.toString(), t.any { it.startsWith("Dictation can't start yet: allow Microphone, Display over other apps and Accessibility service") })
        assertTrue("SETUP above STATUS while blocked", t.indexOf("SETUP") < t.indexOf("STATUS"))
        assertTrue("0 of 5 done" in t)
    }

    @Test fun grantedRowsShowTheirStateAndTheScreenReorders() {
        shadowOf(app).grantPermissions(Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS)
        ShadowSettings.setCanDrawOverlays(true)
        val ctl = launch()
        val a = ctl.get()
        val mic = UiTestKit.walk(a.window.decorView).first { it.isFocusable && it.contentDescription?.startsWith("Microphone,") == true }
        assertTrue(mic.contentDescription.toString(), mic.contentDescription.startsWith("Microphone, Granted"))
        // A granted row is not a button any more; a missing one is.
        assertFalse(mic.isClickable)
        val a11y = rowTitled(a, "Accessibility service")
        assertTrue(a11y.isClickable)
        assertTrue(a11y.contentDescription.toString(), "double-tap to Turn on" in a11y.contentDescription)
        // Accessibility is still off, so the banner names only it.
        assertTrue(texts(a).any { it == "Dictation can't start yet: allow Accessibility service below." })
    }

    @Test fun missingRowsRunTheirAction() {
        val a = launch().get()
        rowTitled(a, "Microphone").performClick()
        val req = shadowOf(a).lastRequestedPermission
        assertNotNull(req)
        assertTrue(Manifest.permission.RECORD_AUDIO in req.requestedPermissions)
        rowTitled(a, "Display over other apps").performClick()
        assertEquals(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, shadowOf(a).nextStartedActivity.action)
        rowTitled(a, "Accessibility service").performClick()
        assertEquals(Settings.ACTION_ACCESSIBILITY_SETTINGS, shadowOf(a).nextStartedActivity.action)
    }

    @Test fun statusLinksOpenTheirSettingsPage() {
        val a = launch().get()
        val lang = rowTitled(a, "Language")
        assertTrue(lang.contentDescription.toString(), lang.contentDescription.endsWith("Opens Language settings"))
        lang.performClick()
        val i: Intent = shadowOf(a).nextStartedActivity
        assertEquals(SettingsActivity::class.java.name, i.component?.className)
        assertEquals(SettingsPages.Page.LANGUAGE.name, i.getStringExtra(SettingsActivity.EXTRA_PAGE))
        rowTitled(a, "History").performClick()
        assertEquals(HistoryActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
        rowTitled(a, "Dictionary & snippets").performClick()
        assertEquals(DictionaryActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
        rowTitled(a, "Scratchpad").performClick()
        assertEquals(NoteEditorActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
        rowTitled(a, "Insights").performClick()
        assertEquals(InsightsActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
    }

    @Test fun oemGuidanceIsCollapsibleAndReadsAsExpandable() {
        val a = launch().get()
        val header = UiTestKit.walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Keep it running: ") == true }
        val info = header.createAccessibilityNodeInfo()
        assertTrue(info.actionList.contains(AccessibilityNodeInfo.AccessibilityAction.ACTION_EXPAND))
        assertEquals("Collapsed", header.stateDescription)
        header.performClick()
        assertEquals("Expanded", header.stateDescription)
        assertTrue(header.createAccessibilityNodeInfo().actionList.contains(AccessibilityNodeInfo.AccessibilityAction.ACTION_COLLAPSE))
    }

    @Test fun touchTargetsAreAtLeast48dp() {
        val a = launch().get()
        UiTestKit.walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Keep it running: ") == true }.performClick()
        UiTestKit.assertTouchTargets(a, "main")
    }

    @Test fun startBubbleNeedsTheOverlayFirst() {
        val a = launch().get()
        rowTitled(a, "Start / show bubble").performClick()
        assertTrue(org.robolectric.shadows.ShadowToast.getTextOfLatestToast().startsWith("Allow Display over other apps first"))
    }
}
