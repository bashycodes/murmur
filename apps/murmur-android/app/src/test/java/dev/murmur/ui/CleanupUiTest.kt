package dev.flowclone.ui

import android.app.Activity
import android.widget.ImageButton
import android.widget.Switch
import dev.flowclone.FlowApp
import dev.flowclone.data.CleanupEngines
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevSettings.CleanupRoute
import dev.flowclone.data.SettingsPages.Page
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/** D36 Settings → AI cleanup: the cleanup engines as ranked switch rows (same widget as Speech to text). */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class CleanupUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    private fun launch(): Activity =
        Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(ctx, Page.CLEANUP)).setup().visible().get()

    @Before fun onDeviceBuildsOnly() = assumeTrue(app.caps.local)

    @After fun reset() {
        app.dev.save(DevSettings.defaults(app.caps))
        app.settings.save(app.settings.load().copy(llmEnabled = dev.flowclone.data.Settings().llmEnabled))
    }

    private fun row(a: Activity, r: CleanupRoute) = rowTitled(a, CleanupEngines.name(r))

    @Test fun enginesAreRankedSwitchRowsInTheStoredOrder() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("Engines", "AI cleanup model")) assertTrue(want, want in t)
        assertFalse("replaced by the phone's row", "Clean up with local AI" in t)
        val list = CleanupEngines.engines(app.dev.load(), app.caps)
        assertEquals(DevSettings.DEFAULT_CLEANUP_ORDER.filter { DevSettings.cleanupAvailable(it, app.caps) }, list)
        list.forEachIndexed { i, r ->
            val info = row(a, r).createAccessibilityNodeInfo()
            assertEquals(Switch::class.java.name, info.className)
            assertTrue(info.isCheckable)
            assertEquals(CleanupEngines.on(app.dev.load(), app.settings.load(), r), info.isChecked)
            assertTrue(info.text.toString(), info.text.startsWith("${i + 1}. Clean up "))
        }
        UiTestKit.assertTouchTargets(a, "cleanup")
    }

    @Test fun serverSwitchAppliesAtOnce() {
        assumeTrue(app.caps.remote)
        val a = launch()
        assertTrue(app.dev.load().cleanupOn(CleanupRoute.SERVER))
        row(a, CleanupRoute.SERVER).performClick(); idle()
        assertFalse(app.dev.load().cleanupOn(CleanupRoute.SERVER))
        assertFalse(row(a, CleanupRoute.SERVER).createAccessibilityNodeInfo().isChecked)
        assertTrue(org.json.JSONObject(app.dev.load().routerConfigFor(app.caps)).getJSONArray("cleanup_off").toString().contains("\"server\""))
    }

    @Test fun phoneSwitchIsTheOnDeviceAi() {
        app.settings.save(app.settings.load().copy(llmEnabled = false))
        val a = launch()
        assertFalse(row(a, CleanupRoute.LOCAL).createAccessibilityNodeInfo().isChecked)
        row(a, CleanupRoute.LOCAL).performClick(); idle()
        assertTrue(app.settings.load().llmEnabled); assertTrue(app.dev.load().cleanupOn(CleanupRoute.LOCAL))
        row(a, CleanupRoute.LOCAL).performClick(); idle()
        assertFalse(app.settings.load().llmEnabled); assertFalse(app.dev.load().cleanupOn(CleanupRoute.LOCAL))
    }

    @Test fun changeOrderSwapsNeighboursAndSaves() {
        assumeTrue(CleanupEngines.engines(app.dev.load(), app.caps).size > 1)
        val a = launch()
        val before = CleanupEngines.engines(app.dev.load(), app.caps)
        rowTitled(a, "Change order").performClick(); idle()
        assertTrue("Done" in texts(a))
        val first = CleanupEngines.name(before[0])
        assertFalse("the first engine cannot move up",
            walk(a.window.decorView).filterIsInstance<ImageButton>().first { it.contentDescription == "Move $first up" }.isEnabled)
        walk(a.window.decorView).filterIsInstance<ImageButton>().first { it.contentDescription == "Move $first down" }.performClick(); idle()
        val after = CleanupEngines.engines(app.dev.load(), app.caps)
        assertEquals(before[0], after[1]); assertEquals(before[1], after[0])
        rowTitled(a, "Done").performClick(); idle()
        assertTrue("Change order" in texts(a))
    }

    @Test fun openRouterAsksForConsentAndCancelLeavesItOff() {
        assumeTrue(app.caps.cloud)
        val a = launch()
        row(a, CleanupRoute.CLOUD).performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertFalse(app.dev.load().allowCloudCleanup)
        assertFalse(row(a, CleanupRoute.CLOUD).createAccessibilityNodeInfo().isChecked)
    }
}
