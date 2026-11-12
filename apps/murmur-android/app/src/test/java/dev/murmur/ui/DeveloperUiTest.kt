package dev.murmur.ui

import android.app.AlertDialog
import android.content.Intent
import android.view.KeyEvent
import android.view.inputmethod.EditorInfo
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

    /** v1.2.0 bug: Next from Port landed on the TLS switch row and Enter turned TLS on. */
    @Test fun keyboardNextGoesHostPortTokenThenDoneAndNeverTogglesTls() {
        val a = launch(Page.SERVER)
        val tlsBefore = app.dev.load().tls
        val host = field(a, "Host or IP"); val port = field(a, "Port"); val token = field(a, "Token")
        assertEquals(EditorInfo.IME_ACTION_NEXT, host.imeOptions and EditorInfo.IME_MASK_ACTION)
        assertEquals(EditorInfo.IME_ACTION_NEXT, port.imeOptions and EditorInfo.IME_MASK_ACTION)
        assertEquals(EditorInfo.IME_ACTION_DONE, token.imeOptions and EditorInfo.IME_MASK_ACTION)
        host.requestFocus()
        host.onEditorAction(EditorInfo.IME_ACTION_NEXT)
        assertTrue("host → port", port.hasFocus())
        port.onEditorAction(EditorInfo.IME_ACTION_NEXT)
        assertTrue("port → token, not the TLS switch", token.hasFocus())
        // A hardware / IME Enter on the port field takes the same path.
        port.requestFocus()
        port.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
        port.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER))
        token.onEditorAction(EditorInfo.IME_ACTION_DONE)
        assertFalse("Done leaves the form", token.hasFocus())
        val tlsRow = UiTestKit.rowTitled(a, "Use TLS (wss)").createAccessibilityNodeInfo()
        assertEquals("TLS unchanged", tlsBefore, tlsRow.isChecked)
    }

    /** v1.2.0 bug: the warning banner sat above the fields and grew / shrank while typing, moving them. */
    @Test fun serverWarningsSitBelowTheFields() {
        org.junit.Assume.assumeTrue(app.caps.remote && app.dev.load().allowServer)
        val a = launch(Page.SERVER)
        field(a, "Host or IP").setText("")
        val all = walk(a.window.decorView).toList()
        val warning = all.filterIsInstance<TextView>().first { it.text.toString().contains(dev.murmur.data.DevSettingsForm.NO_HOST) }
        assertTrue(warning.visibility == android.view.View.VISIBLE)
        assertTrue("below the last field", all.indexOf(warning) > all.indexOf(field(a, "Health cache (s)")))
        assertFalse("plain words, no 'client'", dev.murmur.data.DevSettingsForm.NO_HOST.contains("client"))
    }

    @Test fun tokenHelpNamesTheServersKeysAddCommand() {
        val t = texts(launch(Page.SERVER))
        assertTrue(t.toString(), t.any { it.contains("murmur-server keys add <name>") })
        assertFalse(t.any { it.contains("gen-token") })
    }

    /** v1.2.0 bug: the top bar (back, Save) scrolled away with the form. */
    @Test fun topBarStaysOutsideTheScrollingColumn() {
        for (p in listOf(Page.SERVER, Page.HUB)) {
            val a = launch(p)
            val back = buttons(a).first { it.contentDescription == "Back" }
            var v: android.view.ViewParent? = back.parent
            while (v != null) { assertFalse("$p: back button inside the scroll view", v is android.widget.ScrollView); v = v.parent }
        }
    }
}
