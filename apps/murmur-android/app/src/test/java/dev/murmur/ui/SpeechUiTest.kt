package dev.murmur.ui

import android.app.Activity
import android.widget.ImageButton
import android.widget.Switch
import dev.murmur.MurmurApp
import dev.murmur.data.DevPages
import dev.murmur.data.DevSettings
import dev.murmur.data.DevSettings.Route
import dev.murmur.data.SettingsPages.Page
import dev.murmur.data.SettingsStore
import dev.murmur.data.SettingsSummary
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
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
@Config(sdk = [34], application = MurmurApp::class)
class SpeechUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)

    private fun launch(): Activity =
        Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(ctx, Page.SPEECH)).setup().visible().get()

    @After fun reset() {
        app.dev.save(DevSettings.defaults(app.caps))
        app.dev.saveCloud(allowCloud = false, cloud = app.dev.load().cloud)
        app.settings.save(app.settings.load().copy(decodeWhileSpeaking = dev.murmur.data.Settings().decodeWhileSpeaking))
    }

    /** The row body (rank, name, state line): opens the engine's page. */
    private fun engineRow(a: Activity, r: Route) = rowTitled(a, SettingsSummary.engineName(r))

    /** The switch target at the end of the engine's row: the only part that turns it on or off. */
    private fun engineSwitch(a: Activity, r: Route): android.view.View =
        (engineRow(a, r).parent as android.view.ViewGroup).let { it.getChildAt(it.childCount - 1) }

    @Test fun enginesAreRankedSwitchRowsInTheStoredOrder() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("Speech to text", "Engines", "Transcribe while I speak")) assertTrue(want, want in t)
        assertFalse("omitted: Android has no such switch", "Use dictionary words" in t)
        val list = DevPages.engines(app.dev.load(), app.caps)
        assertTrue(list.isNotEmpty())
        list.forEachIndexed { i, r ->
            val row = engineRow(a, r).createAccessibilityNodeInfo()
            assertEquals(android.widget.Button::class.java.name, row.className)
            assertTrue(row.text.toString(), row.text.startsWith("${i + 1}. ${SettingsSummary.engineName(r)}. "))
            val info = engineSwitch(a, r).createAccessibilityNodeInfo()
            assertEquals(Switch::class.java.name, info.className)
            assertTrue(info.isCheckable)
            assertEquals(DevPages.engineOn(app.dev.load(), r), info.isChecked)
            assertTrue(info.text.toString(), info.text.startsWith("Use "))
        }
        assertEquals(app.caps.local, "On this phone" in t)
        UiTestKit.assertTouchTargets(a, "speech")
    }

    @Test fun engineSwitchAppliesAtOnce() {
        val a = launch()
        val r = if (app.caps.remote) Route.SERVER else Route.CLIENT
        val was = DevPages.engineOn(app.dev.load(), r)
        engineSwitch(a, r).performClick(); idle()
        assertEquals(!was, DevPages.engineOn(app.dev.load(), r))
        assertEquals(!was, engineSwitch(a, r).createAccessibilityNodeInfo().isChecked)
    }

    /** v1.2.0 bug: tapping "Your server" toggled it, and nothing on this page led to its address and key. */
    @Test fun tappingTheServerRowOpensTheServerPageWithoutToggling() {
        assumeTrue(app.caps.remote)
        val a = launch()
        val was = DevPages.engineOn(app.dev.load(), Route.SERVER)
        engineRow(a, Route.SERVER).performClick(); idle()
        assertEquals("the row body does not toggle", was, DevPages.engineOn(app.dev.load(), Route.SERVER))
        val next = org.robolectric.Shadows.shadowOf(a).nextStartedActivity
        assertEquals(DeveloperActivity::class.java.name, next.component?.className)
        assertEquals(DevPages.Page.SERVER.name, next.getStringExtra(DeveloperActivity.EXTRA_PAGE))
    }

    @Test fun tappingTheOpenRouterRowOpensItsPage() {
        assumeTrue(app.caps.cloud)
        val a = launch()
        engineRow(a, Route.CLOUD).performClick(); idle()
        assertEquals(OpenRouterActivity::class.java.name, org.robolectric.Shadows.shadowOf(a).nextStartedActivity.component?.className)
        assertFalse(app.dev.load().cloudRouted)
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
        engineSwitch(a, Route.CLOUD).performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertFalse(app.dev.load().cloudRouted)
        assertFalse(engineSwitch(a, Route.CLOUD).createAccessibilityNodeInfo().isChecked)
    }

    @Test fun transcribeWhileISpeakAppliesAtOnce() {
        val a = launch()
        val was = app.settings.load().decodeWhileSpeaking
        rowTitled(a, "Transcribe while I speak").performClick()
        assertEquals(!was, app.settings.load().decodeWhileSpeaking)
    }

    @Test fun keepBothLanguagesLoadedIsOffByDefaultAndPersists() {
        assumeTrue(app.caps.local)
        app.settings.save(app.settings.load().copy(keepBothLanguages = false))
        val a = launch()
        val row = rowTitled(a, "Keep both languages loaded")
        assertFalse(row.createAccessibilityNodeInfo().isChecked)
        // No Hebrew model in a fresh install: the summary says the switch does nothing.
        assertTrue(texts(a).any { it.startsWith("No effect") })
        row.performClick()
        assertTrue("persisted", SettingsStore(a).load().keepBothLanguages)
        row.performClick()
        assertFalse(SettingsStore(a).load().keepBothLanguages)
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
