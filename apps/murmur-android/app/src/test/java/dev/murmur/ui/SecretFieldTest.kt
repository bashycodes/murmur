package dev.murmur.ui

import android.text.InputType
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.EditText
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Security: the server token / cloud API key must never be readable by accessibility services. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SecretFieldTest {
    private val secret = "tok_SUPERSECRET_0123456789abcdef"

    /** Built like DeveloperActivity.field(): input type, text, then setSingleLine (the old bug). */
    private fun field(): EditText = EditText(RuntimeEnvironment.getApplication()).apply {
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        setText(secret)
        isSingleLine = true
    }

    private fun AccessibilityNodeInfo.leaks() = listOf(text, contentDescription, hintText, stateDescription, tooltipText, paneTitle, error)
        .any { it?.toString()?.contains("SUPERSECRET") == true }

    private fun AccessibilityEvent.leaks() = (text.map { it?.toString() } + listOf(beforeText?.toString(), contentDescription?.toString()))
        .any { it?.contains("SUPERSECRET") == true }

    @Test fun unprotectedFieldWithSingleLineLeaks_documentsTheBug() {
        // setSingleLine after the password input type drops the mask: the old field exposed the token.
        val e = field()
        assertTrue("regression guard for the root cause", e.createAccessibilityNodeInfo().leaks())
    }

    @Test fun nodeNeverHoldsTheSecret_maskedOrShown() {
        val e = field()
        SecretField.protect(e)
        val masked = e.createAccessibilityNodeInfo()
        assertFalse(masked.leaks())
        assertEquals("set (hidden)", masked.text.toString())
        assertTrue(masked.isPassword)
        assertFalse(SecretField.isShown(e))

        SecretField.setShown(e, true)
        assertTrue(SecretField.isShown(e))
        assertFalse("shown on screen, still hidden from a11y", e.createAccessibilityNodeInfo().leaks())
        assertEquals(secret, e.text.toString())
        SecretField.setShown(e, false)
        assertFalse(e.createAccessibilityNodeInfo().leaks())
    }

    @Test fun emptyFieldSaysEmpty() {
        val e = field()
        SecretField.protect(e)
        e.setText("")
        assertEquals("empty", e.createAccessibilityNodeInfo().text.toString())
    }

    @Test fun eventsNeverHoldTheSecret() {
        val e = field()
        SecretField.protect(e)
        SecretField.setShown(e, true)
        for (type in listOf(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED, AccessibilityEvent.TYPE_VIEW_FOCUSED, AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED)) {
            val ev = AccessibilityEvent.obtain(type)
            e.onInitializeAccessibilityEvent(ev)
            e.dispatchPopulateAccessibilityEvent(ev)
            ev.beforeText = secret
            e.accessibilityDelegate.sendAccessibilityEventUnchecked(e, ev)
            assertFalse("event $type", ev.leaks())
        }
    }

    @Test fun excludedFromAutofillAndContentCapture() {
        val e = field()
        SecretField.protect(e)
        assertEquals(android.view.View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS, e.importantForAutofill)
        assertTrue(e.inputType and InputType.TYPE_TEXT_VARIATION_PASSWORD == InputType.TYPE_TEXT_VARIATION_PASSWORD)
    }
}
