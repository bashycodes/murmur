package dev.flowclone.ui

import android.app.Activity
import android.widget.ImageButton
import android.widget.Switch
import dev.flowclone.FlowApp
import dev.flowclone.data.DevPages
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevSettings.Route
import dev.flowclone.data.SettingsPages.Page
import dev.flowclone.data.SettingsSummary
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
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/**
 * Settings → Speech to text (AndroidSpeech.dc.html), per flavour: ranked engine rows that read as
 * one switch each, apply-at-once switches, Change order, OpenRouter's consent, the model card and
 * Transcribe while I speak.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class SpeechUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    private fun launch(): Activity =
        Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(ctx, Page.SPEECH)).setup().visible().get()

    @After fun reset() {
        app.dev.save(DevSettings.defaults(app.caps))
        app.dev.saveCloud(allowCloud = false, cloud = app.dev.load().cloud)
        app.settings.save(app.settings.load().copy(decodeWhileSpeaking = dev.flowclone.data.Settings().decodeWhileSpeaking))
    }

    private fun engineRow(a: Activity, r: Route) = rowTitled(a, SettingsSummary.engineName(r))

    @Test fun enginesAreRankedSwitchRowsInTheStoredOrder() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("Speech to text", "Engines", "Transcribe while I speak")) assertTrue(want, want in t)
        assertFalse("omitted: Android has no such switch", "Use dictionary words" in t)
        val list = DevPages.engines(app.dev.load(), app.caps)
        assertTrue(list.isNotEmpty())
        list.forEachIndexed { i, r ->
            val info = engineRow(a, r).createAccessibilityNodeInfo()
            assertEquals(Switch::class.java.name, info.className)
            assertTrue(info.isCheckable)
            assertEquals(DevPages.engineOn(app.dev.load(), r), info.isChecked)
            assertTrue(info.text.toString(), info.text.startsWith("${i + 1}. Use "))
        }
        assertEquals(app.caps.local, "On this phone" in t)
        UiTestKit.assertTouchTargets(a, "speech")
    }

    @Test fun engineSwitchAppliesAtOnce() {
        val a = launch()
        val r = if (app.caps.remote) Route.SERVER else Route.CLIENT
        val was = DevPages.engineOn(app.dev.load(), r)
        engineRow(a, r).performClick(); idle()
        assertEquals(!was, DevPages.engineOn(app.dev.load(), r))
        assertEquals(!was, engineRow(a, r).createAccessibilityNodeInfo().isChecked)
    }

    @Test fun changeOrderSwapsNeighboursAndSaves() {
        assumeTrue(DevPages.engines(app.dev.load(), app.caps).size > 1)
        val a = launch()
        val before = DevPages.engines(app.dev.load(), app.caps)
        rowTitled(a, "Change order").performClick(); idle()
        assertTrue("Done" in texts(a))
        val first = SettingsSummary.engineName(before[0])
        val up = walk(a.window.decorView).filterIsInstance<ImageButton>().first { it.contentDescription == "Move $first up" }
        assertFalse("the first engine cannot move up", up.isEnabled)
        walk(a.window.decorView).filterIsInstance<ImageButton>().first { it.contentDescription == "Move $first down" }.performClick(); idle()
        val after = DevPages.engines(app.dev.load(), app.caps)
        assertEquals(before[0], after[1]); assertEquals(before[1], after[0])
        UiTestKit.assertTouchTargets(a, "speech reorder")
        rowTitled(a, "Done").performClick(); idle()
        assertTrue("Change order" in texts(a))
    }

    @Test fun openRouterAsksForConsentAndCancelLeavesItOff() {
        assumeTrue(app.caps.cloud)
        val a = launch()
        engineRow(a, Route.CLOUD).performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertFalse(app.dev.load().cloudRouted)
        assertFalse(engineRow(a, Route.CLOUD).createAccessibilityNodeInfo().isChecked)
    }

    @Test fun transcribeWhileISpeakAppliesAtOnce() {
        val a = launch()
        val was = app.settings.load().decodeWhileSpeaking
        rowTitled(a, "Transcribe while I speak").performClick()
        assertEquals(!was, app.settings.load().decodeWhileSpeaking)
    }

    @Test fun modelCardOpensTheEnginePageAndImport() {
        assumeTrue(app.caps.local)
        app.settings.modelPath = null; app.settings.parakeetPath = null
        val a = launch()
        assertTrue("No model imported" in texts(a))
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Import a model" }.performClick()
        assertEquals(ModelImportActivity::class.java.name, org.robolectric.Shadows.shadowOf(a).nextStartedActivity.component?.className)
    }
}
