package dev.murmur.ui

import android.content.Intent
import android.view.View
import dev.murmur.MurmurApp
import dev.murmur.data.AppVersion
import dev.murmur.data.ShellPages
import dev.murmur.data.ShellPages.Tab
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowSettings
import org.robolectric.shadows.ShadowToast

/** App shell: bottom navigation, the drawer (stats, engines, links, extras, version) and Back. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class ShellUiTest {
    @After fun reset() { ShadowSettings.setCanDrawOverlays(false) }

    private fun launch() = Robolectric.buildActivity(MainActivity::class.java).setup().visible().get().also { idle() }

    private fun openDrawer(a: MainActivity) {
        walk(a.window.decorView).first { it.contentDescription == "Open menu" }.performClick()
        repeat(100) { idle(); if ("–" !in texts(a)) return; Thread.sleep(10) }
    }

    @Test fun bottomNavShowsFourTabsWithHomeSelected() {
        val a = launch()
        assertEquals(Tab.HOME, a.current)
        for (t in Tab.entries) {
            val v = a.frame.navView(t)
            assertEquals(t.label, v.contentDescription)
            assertEquals(t == Tab.HOME, v.isSelected)
            assertEquals(t == Tab.HOME, v.createAccessibilityNodeInfo().isSelected)
        }
        UiTestKit.assertTouchTargets(a, "shell")
    }


    @Test fun drawerShowsStatsEnginesLinksExtrasAndVersion() {
        val a = launch()
        openDrawer(a)
        assertTrue(a.frame.drawerOpen)
        val t = texts(a)
        for (want in listOf(
            "words", "wpm", "day streak", "Engines", "Speech to text", "AI cleanup", "Settings", "Permissions", "Help",
            "Notes", "Quick note", "Insights", "History", "Start / show bubble", "Stop bubble",
        )) assertTrue("$want in $t", want in t)
        assertTrue(ShellPages.versionLine(AppVersion.current.version) in t)
        val app = MurmurApp.of(a)
        assertEquals(app.caps.local, "Speech model" in t)
        assertEquals(app.dev.visible, "Developer settings" in t)
        // Nothing granted under Robolectric: mic, overlay, a11y, battery, notifications.
        assertTrue(t.toString(), "5 to fix" in t)
        assertEquals("Permissions, 5 to fix", rowTitled(a, "Permissions").contentDescription)
        // Main content is hidden from TalkBack while the drawer is open.
        val host = a.frame.host.parent as View
        assertEquals(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS, host.importantForAccessibility)
        val engines = walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Engines. Speech to text: ") == true }
        assertTrue(engines.contentDescription.toString(), "AI cleanup: " in engines.contentDescription)
        UiTestKit.assertTouchTargets(a, "drawer")
    }

    @Test fun drawerLinksGoWhereTheySayAndCloseTheDrawer() {
        val a = launch()
        openDrawer(a)
        rowTitled(a, "Permissions").performClick()
        assertEquals(PermissionsActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
        assertFalse(a.frame.drawerOpen)
        openDrawer(a)
        rowTitled(a, "Help").performClick()
        val help = shadowOf(a).nextStartedActivity
        assertEquals(Intent.ACTION_VIEW, help.action)
        assertEquals(ShellPages.HELP_URL, help.dataString)
        for ((title, cls) in listOf("Settings" to SettingsActivity::class.java, "Notes" to NotesActivity::class.java, "Insights" to InsightsActivity::class.java, "History" to HistoryActivity::class.java)) {
            openDrawer(a)
            rowTitled(a, title).performClick()
            assertEquals(title, cls.name, shadowOf(a).nextStartedActivity.component?.className)
        }
        openDrawer(a)
        rowTitled(a, "Quick note").performClick()
        shadowOf(a).nextStartedActivity.let {
            assertEquals(NoteEditorActivity::class.java.name, it.component?.className)
            assertEquals(null, it.getStringExtra(NoteEditorActivity.EXTRA_DOC))
        }
        openDrawer(a)
        rowTitled(a, "Start / show bubble").performClick()
        assertTrue(ShadowToast.getTextOfLatestToast().startsWith("Allow Display over other apps first"))
    }

    @Test fun backClosesTheDrawerFirst() {
        val a = launch()
        openDrawer(a)
        assertTrue(a.frame.drawerOpen)
        @Suppress("DEPRECATION") a.onBackPressed()
        assertFalse(a.frame.drawerOpen)
        assertFalse(a.isFinishing)
        assertEquals(View.IMPORTANT_FOR_ACCESSIBILITY_AUTO, (a.frame.host.parent as View).importantForAccessibility)
    }

    @Test fun scrimClosesTheDrawerAndReadsAsClose() {
        val a = launch()
        openDrawer(a)
        a.frame.scrim.performClick()
        assertFalse(a.frame.drawerOpen)
        assertEquals("Close menu", a.frame.scrim.contentDescription)
    }
}
