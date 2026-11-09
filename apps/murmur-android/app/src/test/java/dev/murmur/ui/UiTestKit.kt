package dev.flowclone.ui

import android.app.Activity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.EditText
import android.widget.TextView
import org.junit.Assert.assertTrue

/** View-tree helpers shared by the Robolectric screen tests (MainUiTest, DictionaryUiTest, HistoryUiTest, ModelImportUiTest). */
internal object UiTestKit {
    fun walk(v: View, out: MutableList<View> = ArrayList()): List<View> {
        out += v
        if (v is ViewGroup) for (i in 0 until v.childCount) walk(v.getChildAt(i), out)
        return out
    }

    fun visible(v: View): Boolean {
        var c: View? = v
        while (c != null) { if (c.visibility != View.VISIBLE) return false; c = c.parent as? View }
        return true
    }

    /** Texts of every visible TextView (buttons included). */
    fun texts(a: Activity) = walk(a.window.decorView).filter { it is TextView && visible(it) }.map { (it as TextView).text.toString() }

    fun rowTitled(a: Activity, title: String): View =
        walk(a.window.decorView).first { v -> v.isClickable && v is ViewGroup && visible(v) && walk(v).any { it is TextView && it.text.toString() == title } }

    fun rowsTitled(a: Activity, title: String): List<View> =
        walk(a.window.decorView).filter { v -> v.isClickable && v is ViewGroup && visible(v) && walk(v).any { it is TextView && it.text.toString() == title } }

    private fun dp(a: Activity, v: Int) = (v * a.resources.displayMetrics.density).toInt()

    /** 48 dp minimum for every visible clickable view (selectable text is not a button). */
    fun assertTouchTargets(a: Activity, what: String) {
        for (v in walk(a.window.decorView).filter { it.isClickable && visible(it) && !(it is TextView && it.isTextSelectable && it !is EditText) }) {
            val h = maxOf(v.minimumHeight, v.height)
            assertTrue("$what ${v.javaClass.simpleName} ${(v as? TextView)?.text ?: v.contentDescription} h=$h", h >= dp(a, 48))
        }
    }

    /** Every string TalkBack could read from the tree: texts, descriptions, hints, node texts. */
    fun a11yStrings(a: Activity): List<String> = walk(a.window.decorView).flatMap { v ->
        val info: AccessibilityNodeInfo = v.createAccessibilityNodeInfo()
        listOfNotNull(info.text?.toString(), info.contentDescription?.toString(), (v as? TextView)?.hint?.toString())
    }

    fun idle() = org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
}
