package dev.flowclone.ui

import android.app.Activity
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevSummary
import dev.flowclone.data.BuildCaps
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Security, new Developer settings layout: the Server page's token row (DevUi.secretRow, the
 * same builder the cloud page and server page use) never exposes the secret in any node of the
 * accessibility tree or in any event, masked or after Show / Paste; the hub subtitle never
 * contains it either.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = android.app.Application::class)
class DevUiSecretTest {
    private val secret = "tok_SUPERSECRET_0123456789abcdef"

    private fun buildServerLike(): Pair<View, DevUi.FieldRow> {
        val a = Robolectric.buildActivity(Activity::class.java).setup().get()
        val ui = DevUi(a)
        lateinit var token: DevUi.FieldRow
        with(ui) {
            screen {
                sectionLabel("ADDRESS")
                card {
                    fieldRow("Host or IP", "100.64.0.10", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI) {}
                    toggleRow("Use TLS (wss)", "Encrypt the connection", false)
                }
                sectionLabel("AUTHENTICATION")
                card { token = secretRow("Token", secret, "From `flow-server gen-token`", "token", onEdit = {}) }
                action("Test connection") {}
            }
        }
        return a.window.decorView to token
    }

    private fun walk(v: View, out: MutableList<View> = ArrayList()): List<View> {
        out += v
        if (v is ViewGroup) for (i in 0 until v.childCount) walk(v.getChildAt(i), out)
        return out
    }

    private fun AccessibilityNodeInfo.leaks() = listOf(text, contentDescription, hintText, stateDescription, tooltipText, paneTitle, error)
        .any { it?.toString()?.contains("SUPERSECRET") == true }

    private fun assertTreeClean(root: View, state: String) {
        for (v in walk(root)) {
            assertFalse("$state: node ${v.javaClass.simpleName}", v.createAccessibilityNodeInfo().leaks())
            for (type in listOf(AccessibilityEvent.TYPE_VIEW_FOCUSED, AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED, AccessibilityEvent.TYPE_VIEW_CLICKED)) {
                val ev = AccessibilityEvent.obtain(type)
                v.onInitializeAccessibilityEvent(ev)
                v.dispatchPopulateAccessibilityEvent(ev)
                val texts = ev.text.map { it?.toString() } + listOf(ev.beforeText?.toString(), ev.contentDescription?.toString())
                assertFalse("$state: event $type from ${v.javaClass.simpleName}", texts.any { it?.contains("SUPERSECRET") == true })
            }
        }
    }

    @Test fun tokenNeverInTheA11yTree_maskedShownAndPasted() {
        val (root, token) = buildServerLike()
        assertEquals(secret, token.text)
        assertTrue(token.edit.createAccessibilityNodeInfo().isPassword)
        assertEquals("set (hidden)", token.edit.createAccessibilityNodeInfo().text.toString())
        assertTreeClean(root, "masked")

        val show = walk(root).filterIsInstance<Button>().first { it.text == "Show" }
        show.performClick()
        assertTrue(SecretField.isShown(token.edit))
        assertEquals("Hide", show.text.toString())
        assertTreeClean(root, "shown")

        // Paste puts the clipboard text in the field: still masked for accessibility.
        val cm = root.context.getSystemService(android.content.ClipboardManager::class.java)
        cm.setPrimaryClip(android.content.ClipData.newPlainText("t", "pasted_SUPERSECRET_9876543210zyxw"))
        walk(root).filterIsInstance<Button>().first { it.text == "Paste" }.performClick()
        assertTrue(token.text.contains("SUPERSECRET"))
        assertTreeClean(root, "pasted")

        walk(root).filterIsInstance<Button>().first { it.text == "Clear" }.performClick()
        assertEquals("", token.text)
        assertEquals("empty", token.edit.createAccessibilityNodeInfo().text.toString())
    }

    @Test fun secretFieldStaysAPasswordFieldAndNoPlainTextViewHoldsIt() {
        val (root, token) = buildServerLike()
        assertTrue(token.edit.inputType and InputType.TYPE_TEXT_VARIATION_PASSWORD == InputType.TYPE_TEXT_VARIATION_PASSWORD)
        assertEquals(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS, token.edit.importantForAutofill)
        // Only the protected EditText holds the value; no label / help / result TextView does.
        for (v in walk(root).filterIsInstance<TextView>()) if (v !== token.edit) assertFalse(v.text.toString().contains("SUPERSECRET"))
        assertTrue(walk(root).filterIsInstance<EditText>().count { it.text.toString() == secret } == 1)
    }

    @Test fun hubServerSubtitleNeverHoldsTheToken() {
        val s = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = secret)
        assertFalse(DevSummary.server(s, BuildCaps.ALL).contains("SUPERSECRET"))
    }
}
