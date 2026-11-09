package dev.flowclone.ui

import android.content.Intent
import android.widget.ImageButton
import dev.flowclone.FlowApp
import dev.flowclone.data.DevPages.Page
import dev.flowclone.data.SettingsPages
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Developer settings after routing moved to Settings → Speech to text: top bar, hub links, Save on Server only. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class DeveloperUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()

    private fun launch(p: Page) = Robolectric.buildActivity(
        DeveloperActivity::class.java, Intent(ctx, DeveloperActivity::class.java).putExtra(DeveloperActivity.EXTRA_PAGE, p.name),
    ).setup().visible().get()

    private fun buttons(a: android.app.Activity) = walk(a.window.decorView).filterIsInstance<ImageButton>()

    @Test fun hubLinksSpeechToTextInSettingsAndEngineOptions() {
        val a = launch(Page.HUB)
        val t = texts(a)
        for (want in listOf("Speech to text", "Engine options")) assertTrue(want, want in t)
        assertFalse("routing lives in Settings now", "Route order" in t)
        assertEquals("Back", buttons(a).first().contentDescription)
        rowTitled(a, "Speech to text").performClick()
        val next = shadowOf(a).nextStartedActivity
        assertEquals(SettingsActivity::class.java.name, next.component?.className)
        assertEquals(SettingsPages.Page.SPEECH.name, next.getStringExtra(SettingsActivity.EXTRA_PAGE))
        UiTestKit.assertTouchTargets(a, "dev hub")
    }

    @Test fun onlyTheServerPageHasSave() {
        assertTrue(buttons(launch(Page.SERVER)).any { it.contentDescription.toString().startsWith("Save ") })
        for (p in listOf(Page.HUB, Page.ENGINES, Page.DEBUG, Page.STATUS)) {
            val a = launch(p)
            assertFalse("$p", buttons(a).any { it.contentDescription.toString().startsWith("Save ") })
            UiTestKit.assertTouchTargets(a, "dev $p")
        }
    }
}
