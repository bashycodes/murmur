package dev.murmur.ui

import android.app.AlertDialog
import android.content.Intent
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.TextView
import dev.murmur.MurmurApp
import dev.murmur.data.DevPages.Page
import dev.murmur.data.SettingsPages
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
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
import org.robolectric.shadows.ShadowAlertDialog

/** Developer settings after routing moved to Settings → Speech to text: top bar, hub links, Save on Server only. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class DeveloperUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()

    private fun launch(p: Page) = Robolectric.buildActivity(
        DeveloperActivity::class.java, Intent(ctx, DeveloperActivity::class.java).putExtra(DeveloperActivity.EXTRA_PAGE, p.name),
    ).setup().visible().get()

    private fun controller(p: Page) = Robolectric.buildActivity(
        DeveloperActivity::class.java, Intent(ctx, DeveloperActivity::class.java).putExtra(DeveloperActivity.EXTRA_PAGE, p.name),
    ).setup().visible()

    /** The input a field label ([TextView.getLabelFor]) points at. */
    private fun field(a: android.app.Activity, label: String): EditText {
        val l = walk(a.window.decorView).filterIsInstance<TextView>().first { it.text.toString() == label && it.labelFor != 0 }
        return a.findViewById(l.labelFor)
    }

    private val app get() = MurmurApp.of(ctx)

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

    @Test fun serverPageHasAVisibleSaveButtonThatSaves() {
        val a = launch(Page.SERVER)
        val save = walk(a.window.decorView).filterIsInstance<Button>().first { it.text.toString() == "Save" }
        assertFalse("nothing to save yet", save.isEnabled)
        field(a, "Port").setText("7401")
        assertTrue(save.isEnabled)
        save.performClick()
        assertEquals(7401, app.dev.load().port)
    }

    @Test fun backgroundingKeepsValidEdits() {
        val c = controller(Page.SERVER)
        field(c.get(), "Port").setText("7402")
        c.pause()
        assertEquals("typed but not confirmed with Save, still kept", 7402, app.dev.load().port)
    }

    @Test fun backWithValidEditsSavesAndCloses() {
        val a = launch(Page.SERVER)
        field(a, "Port").setText("7403")
        buttons(a).first { it.contentDescription == "Back" }.performClick()
        assertEquals(7403, app.dev.load().port)
        assertTrue(a.isFinishing)
    }

    @Test fun backWithInvalidEditsAsksInsteadOfDropping() {
        val before = app.dev.load().port
        val c = controller(Page.SERVER)
        val a = c.get()
        field(a, "Port").setText("0")
        buttons(a).first { it.contentDescription == "Back" }.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog() as AlertDialog
        assertTrue(d.isShowing)
        assertFalse("not closed behind the user's back", a.isFinishing)
        c.pause()
        assertEquals("an invalid value is never written", before, app.dev.load().port)
    }
}
