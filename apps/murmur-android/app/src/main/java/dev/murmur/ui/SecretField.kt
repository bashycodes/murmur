package dev.murmur.ui

import android.text.InputType
import android.text.method.HideReturnsTransformationMethod
import android.text.method.PasswordTransformationMethod
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.EditText

/**
 * Secret inputs (server token, cloud API keys): masked by default, and the value never reaches
 * the accessibility tree, accessibility events, autofill or content capture, masked or shown.
 *
 * Why: any enabled accessibility service (other dictation apps, screen readers) can read every
 * node's text. The old fields also called `isSingleLine = true` after setting the password
 * input type, which replaces the password transformation with SingleLineTransformationMethod:
 * the token was shown and exposed in plain text even with "Show" not pressed.
 */
object SecretField {
    /** What accessibility services see instead of the value. */
    fun a11yText(value: CharSequence?): String = if (value.isNullOrEmpty()) "empty" else "set (hidden)"

    /** Make [e] a secret field. Call after its text is set (order does not matter for the delegate). */
    fun protect(e: EditText) {
        // inputType alone implies single line; never call setSingleLine afterwards (it drops the mask).
        e.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
        e.transformationMethod = PasswordTransformationMethod.getInstance()
        e.importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        e.importantForContentCapture = View.IMPORTANT_FOR_CONTENT_CAPTURE_NO_EXCLUDE_DESCENDANTS
        e.contentDescription = null
        e.accessibilityDelegate = Delegate
    }

    fun isShown(e: EditText) = e.transformationMethod !is PasswordTransformationMethod

    /** "Show" / "Hide": only the on-screen rendering changes; the input type stays a password one. */
    fun setShown(e: EditText, shown: Boolean) {
        val sel = e.selectionEnd
        e.transformationMethod = if (shown) HideReturnsTransformationMethod.getInstance() else PasswordTransformationMethod.getInstance()
        if (sel >= 0) e.setSelection(sel.coerceAtMost(e.text?.length ?: 0))
    }

    private object Delegate : View.AccessibilityDelegate() {
        override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
            super.onInitializeAccessibilityNodeInfo(host, info)
            info.text = a11yText((host as? EditText)?.text)
            info.contentDescription = null
            info.isPassword = true
        }

        override fun onInitializeAccessibilityEvent(host: View, event: AccessibilityEvent) {
            super.onInitializeAccessibilityEvent(host, event)
            scrub(host, event)
        }

        override fun onPopulateAccessibilityEvent(host: View, event: AccessibilityEvent) {
            super.onPopulateAccessibilityEvent(host, event)
            scrub(host, event)
        }

        /** Last stop for every event the field sends (text-changed events carry before/after text). */
        override fun sendAccessibilityEventUnchecked(host: View, event: AccessibilityEvent) {
            scrub(host, event)
            super.sendAccessibilityEventUnchecked(host, event)
        }

        private fun scrub(host: View, event: AccessibilityEvent) {
            event.text.clear()
            event.text.add(a11yText((host as? EditText)?.text))
            event.beforeText = null
            event.contentDescription = null
            event.isPassword = true
        }
    }
}
