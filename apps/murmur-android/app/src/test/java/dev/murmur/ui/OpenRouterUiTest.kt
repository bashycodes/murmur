package dev.murmur.ui

import android.content.DialogInterface
import android.content.Intent
import android.view.accessibility.AccessibilityEvent
import android.widget.EditText
import dev.murmur.MurmurApp
import dev.murmur.data.CloudCatalog
import dev.murmur.data.OpenRouter
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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

/**
 * D28 OpenRouter page + model picker in the Settings look: cards, both opt-ins off with their
 * own consent dialogs, privacy on, the API key never in the a11y tree, the one-time notice.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class OpenRouterUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)

    @Before fun cloudBuild() = assumeTrue(app.caps.cloud)

    private fun launch() = Robolectric.buildActivity(OpenRouterActivity::class.java, Intent(ctx, OpenRouterActivity::class.java)).setup().visible().get()

    private fun switchOf(a: OpenRouterActivity, title: String) =
        walk(rowTitled(a, title)).filterIsInstance<android.widget.Switch>().single()

    @Test fun cardsAndDefaults() {
        val a = launch()
        val t = texts(a)
        for (want in listOf(
            "API KEY", "PRIVACY", "SPEECH-TO-TEXT", "TEXT CLEANUP", "WHAT CLEANUP SENDS", "USAGE",
            "Stored key", "Show", "Paste", "Clear", "Save key", "Check key",
            "Zero-retention providers only", "Deny data collection", "Cloud speech-to-text", OpenRouterActivity.CLEANUP_SWITCH,
            "Model", "Send text around the cursor", "Send on-screen words", "Send app name", "Monthly budget", "Reset counters",
            "Test speech-to-text", "this month", "speech", "cleanup",
        )) assertTrue(want, want in t)
        assertTrue(switchOf(a, "Zero-retention providers only").isChecked)
        assertTrue(switchOf(a, "Deny data collection").isChecked)
        for (off in listOf("Cloud speech-to-text", OpenRouterActivity.CLEANUP_SWITCH, "Send text around the cursor", "Send on-screen words", "Send app name")) {
            assertFalse(off, switchOf(a, off).isChecked)
        }
        assertTrue(t.any { it.startsWith(OpenRouter.DEFAULT_STT_MODEL) })
        assertTrue(t.any { it.startsWith(OpenRouter.DEFAULT_CLEANUP_MODEL) })
        assertEquals(android.widget.Switch::class.java.name, rowTitled(a, "Cloud speech-to-text").createAccessibilityNodeInfo().className)
        UiTestKit.assertTouchTargets(a, "openrouter")
    }

    @Test fun cleanupHasItsOwnConsentAndCancelLeavesItOff() {
        val a = launch()
        rowTitled(a, OpenRouterActivity.CLEANUP_SWITCH).performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        val sd = org.robolectric.Shadows.shadowOf(d)
        assertEquals("Send dictated text to OpenRouter?", sd.title.toString())
        assertTrue(sd.message.toString().startsWith("After each dictation, the text (and your dictionary words) is sent to OpenRouter (openrouter.ai)"))
        assertEquals("Send text to OpenRouter", d.getButton(DialogInterface.BUTTON_POSITIVE).text.toString())
        d.getButton(DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertFalse(switchOf(a, OpenRouterActivity.CLEANUP_SWITCH).isChecked)
        assertFalse(app.dev.load().allowCloudCleanup); assertFalse(app.dev.load().cloud.cleanup.consentOk)

        rowTitled(a, OpenRouterActivity.CLEANUP_SWITCH).performClick(); idle()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        val s = app.dev.load()
        assertTrue(s.allowCloudCleanup); assertTrue(s.cloud.cleanup.consentOk); assertTrue(s.cloudCleanupRouted)
        assertFalse("cleanup never turns speech-to-text on", s.allowCloud); assertFalse(s.cloud.stt.consentOk)
        assertTrue(switchOf(a, OpenRouterActivity.CLEANUP_SWITCH).isChecked)
    }

    @Test fun speechToTextConsentNamesTheAudio() {
        val a = launch()
        rowTitled(a, "Cloud speech-to-text").performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        val sd = org.robolectric.Shadows.shadowOf(d)
        assertEquals("Send your voice to OpenRouter?", sd.title.toString())
        assertTrue(sd.message.toString().contains("audio of each dictation leaves this phone"))
        d.getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        val s = app.dev.load()
        assertTrue(s.allowCloud); assertTrue(s.cloud.stt.consentOk); assertFalse(s.allowCloudCleanup)
        assertEquals(dev.murmur.data.DevSettings.Route.CLOUD, s.routeOrder.first())
    }

    @Test fun turningZeroRetentionOffAsksFirst() {
        val a = launch()
        rowTitled(a, "Zero-retention providers only").performClick(); idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        d.getButton(DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertTrue(switchOf(a, "Zero-retention providers only").isChecked)
        assertTrue(app.dev.load().cloud.privacy.zdr)
    }

    @Test fun typedKeyNeverReachesTheA11yTreeAndIsStoredSealed() {
        val a = launch()
        val key = walk(a.window.decorView).filterIsInstance<EditText>().first { it.createAccessibilityNodeInfo().isPassword }
        key.setText("sk-or-SUPERSECRET-0123")
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Show" }.performClick()
        for (v in walk(a.window.decorView)) {
            val n = v.createAccessibilityNodeInfo()
            val strings = listOf(n.text, n.contentDescription, n.hintText, n.stateDescription, n.error).map { it?.toString() }
            assertFalse(v.javaClass.simpleName, strings.any { it?.contains("SUPERSECRET") == true })
            val ev = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED)
            v.onInitializeAccessibilityEvent(ev); v.dispatchPopulateAccessibilityEvent(ev)
            assertFalse(ev.text.any { it?.contains("SUPERSECRET") == true })
        }
        assertEquals("set (hidden)", key.createAccessibilityNodeInfo().text.toString())
        // Save: Robolectric has no AndroidKeyStore, so the page reports it (never with the key) instead of crashing.
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Save key" }.performClick(); idle()
        assertFalse(UiTestKit.a11yStrings(a).any { it.contains("SUPERSECRET") })
        assertTrue(texts(a).any { it.startsWith("Could not store the key") || it == "Key saved on this phone." })
    }

    @Test fun noticeShowsOnceUntilDismissed() {
        app.dev.cloudNotice = "Cloud speech-to-text now uses OpenRouter only. Groq was removed and its key deleted."
        val a = launch()
        assertTrue(texts(a).any { it.startsWith("Cloud speech-to-text now uses OpenRouter only") })
        walk(a.window.decorView).first { it is android.widget.Button && it.text == "Dismiss" }.performClick(); idle()
        assertNull(app.dev.cloudNotice)
        assertFalse(texts(launch()).any { it.startsWith("Cloud speech-to-text now uses OpenRouter only") })
    }

    @Test fun developerHubLinksTheOnePageAndShowsTheNotice() {
        app.dev.cloudNotice = "The model openai/gpt-transcribe has no zero-retention provider on OpenRouter."
        val a = Robolectric.buildActivity(DeveloperActivity::class.java, Intent(ctx, DeveloperActivity::class.java)).setup().visible().get()
        val t = texts(a)
        assertTrue(t.toString(), OpenRouterActivity.TITLE in t)
        assertFalse("no provider list any more", "Cloud providers" in t)
        assertTrue(t.any { it.startsWith("The model openai/gpt-transcribe") })
        assertTrue("Speech off · Cleanup off · no key" in t)
    }

    @Test fun aRefreshReadsTheCustomServerAddressOnce() {
        // D52: the cleanup card and the custom-server card share one settings value per refresh.
        val prev = dev.murmur.data.CustomServer.check
        var calls = 0
        dev.murmur.data.CustomServer.check = dev.murmur.data.CustomAddressCheck { u, m -> calls++; dev.murmur.data.FakeCustomAddress.check(u, m) }
        try {
            app.dev.saveCloud(cloud = app.dev.load().cloud.copy(
                cleanupProvider = dev.murmur.data.CleanupProvider.CUSTOM,
                customCleanup = dev.murmur.data.CustomCleanupSettings("http://10.0.0.5:4000", "m"),
            ))
            val c = Robolectric.buildActivity(OpenRouterActivity::class.java, Intent(ctx, OpenRouterActivity::class.java)).setup().visible()
            assertTrue("custom card shown", "10.0.0.5" in texts(c.get()).joinToString())
            assertTrue("switch title does not follow the provider", OpenRouterActivity.CLEANUP_SWITCH in texts(c.get()))
            calls = 0
            c.pause().resume()
            assertEquals(1, calls)
        } finally { dev.murmur.data.CustomServer.check = prev }
    }

    @Test fun settingsHubHasTheUserFacingEntry() {
        val a = Robolectric.buildActivity(SettingsActivity::class.java, Intent(ctx, SettingsActivity::class.java)).setup().visible().get()
        val t = texts(a)
        // Connections → "OpenRouter · No key · $0 this month" (AndroidSettings.dc.html).
        assertTrue(t.toString(), OpenRouter.NAME in t)
        assertTrue(t.toString(), "No key · \$0 this month" in t)
        rowTitled(a, OpenRouter.NAME).performClick()
        assertEquals(OpenRouterActivity::class.java.name, org.robolectric.Shadows.shadowOf(a).nextStartedActivity.component?.className)
    }

    @Test fun pagesHaveTheDesignTopBarWithALabelledBack() {
        val pages = listOf(
            launch() to OpenRouterActivity.TITLE,
            Robolectric.buildActivity(
                ModelPickerActivity::class.java,
                Intent(ctx, ModelPickerActivity::class.java).putExtra(ModelPickerActivity.EXTRA_USE, CloudCatalog.Use.CLEANUP.wire),
            ).setup().visible().get() to "Cleanup model",
        )
        for ((a, title) in pages) {
            val back = walk(a.window.decorView).filterIsInstance<android.widget.ImageButton>().first()
            assertEquals("Back", back.contentDescription)
            assertTrue(title, title in texts(a))
            back.performClick()
            assertTrue(a.isFinishing)
        }
    }

    @Test fun pickerHasFiltersAndTheFreeTextRow() {
        val a = Robolectric.buildActivity(
            ModelPickerActivity::class.java,
            Intent(ctx, ModelPickerActivity::class.java).putExtra(ModelPickerActivity.EXTRA_USE, CloudCatalog.Use.STT.wire),
        ).setup().visible().get()
        val t = texts(a)
        for (want in listOf("Transcription", "Chat audio", "✓ Zero-retention only", "Fast providers", "Free", "Use custom model id…", "Refresh", "In use")) {
            assertTrue("$want in $t", want in t)
        }
        val zdr = walk(a.window.decorView).first { it is android.widget.Button && it.text == "✓ Zero-retention only" }
        assertEquals("selected", zdr.stateDescription)
        zdr.performClick(); idle()
        assertTrue("Zero-retention only" in texts(a))
        UiTestKit.assertTouchTargets(a, "picker")
    }
}
