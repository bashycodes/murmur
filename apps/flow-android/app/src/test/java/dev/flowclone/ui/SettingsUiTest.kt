package dev.flowclone.ui

import android.app.Activity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.Switch
import android.widget.TextView
import dev.flowclone.BuildConfig
import dev.flowclone.FlowApp
import dev.flowclone.data.AppVersion
import dev.flowclone.data.DevUnlock
import dev.flowclone.data.SettingsPages.Page
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/**
 * Main Settings layout, per flavour (this runs once per flavour's unit tests): the hub's rows,
 * the `remote` gates, 48 dp targets, switch rows that read as one switch, apply-at-once
 * switches, and the Save / Discard behaviour of a form page.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class SettingsUiTest {
    private fun launch(p: Page = Page.HUB): Activity =
        Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(androidx_ctx(), p)).setup().get()

    private fun androidx_ctx() = org.robolectric.RuntimeEnvironment.getApplication()

    private fun walk(v: View, out: MutableList<View> = ArrayList()): List<View> {
        out += v
        if (v is ViewGroup) for (i in 0 until v.childCount) walk(v.getChildAt(i), out)
        return out
    }

    private fun texts(a: Activity) = walk(a.window.decorView).filterIsInstance<TextView>().map { it.text.toString() }

    private fun dp(a: Activity, v: Int) = (v * a.resources.displayMetrics.density).toInt()

    private fun rowTitled(a: Activity, title: String): View =
        walk(a.window.decorView).first { v -> v.isClickable && v is ViewGroup && walk(v).any { it is TextView && it.text.toString() == title } }

    @Test fun hubShowsEverySectionAndTheFlavourGates() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("STATUS", "DICTATION", "BUBBLE & APPS", "PRIVACY", "ADVANCED", "ABOUT", "Language", "Timing & limits", "Bubble", "Excluded apps", "History", "Mock engine")) {
            assertTrue(want, want in t)
        }
        // docs/hybrid-architecture.md §1.4: `remote` has no on-device engine, model import or LLM.
        assertEquals(BuildConfig.CAP_LOCAL, "Speech engine" in t)
        assertEquals(BuildConfig.CAP_LOCAL, "AI cleanup" in t)
        assertTrue(t.any { it.startsWith("Version ") && "(${FlowApp.of(a).caps.flavor})" in it })
        // remote always shows Developer settings; local / hybrid only once unlocked.
        assertEquals(FlowApp.of(a).dev.visible, "Developer settings" in t)
    }

    @Test fun aboutRowShowsTheAppVersionLabelAndSevenTapsStillUnlock() {
        val a = launch()
        val app = FlowApp.of(a)
        val label = AppVersion.current.label(app.caps.flavor)
        // D26: the About row names the commit, like the main screen.
        assertTrue(label, label in texts(a))
        assertTrue(label, label.endsWith(" · ${AppVersion.current.build}"))
        if (app.caps.remote) return // remote always shows Developer settings; nothing to unlock
        app.dev.devUnlocked = false
        val row = rowTitled(a, label)
        repeat(DevUnlock.TAPS - 1) { row.performClick() }
        assertFalse(app.dev.devUnlocked)
        row.performClick()
        assertTrue(app.dev.devUnlocked)
        app.dev.devUnlocked = false
    }

    @Test fun hubRowsShowTheirCurrentValue() {
        val a = launch()
        val t = texts(a)
        assertTrue(t.toString(), "Live preview · shrinks to a dot · shake to restore" in t)
        assertTrue("None (banking apps are always excluded)" in t)
        assertTrue("Keep everything" in t)
        assertTrue(t.any { it.startsWith("Stops after 1.5 s of silence") })
        val row = rowTitled(a, "Bubble")
        assertTrue(row.contentDescription.toString(), row.contentDescription.startsWith("Bubble, Live preview"))
    }

    @Test fun everyTouchTargetIsAtLeast48dp() {
        for (p in listOf(Page.HUB, Page.LANGUAGE, Page.TIMING, Page.BUBBLE, Page.APPS, Page.HISTORY) + if (BuildConfig.CAP_LOCAL) listOf(Page.ENGINE, Page.CLEANUP) else emptyList()) {
            val a = launch(p)
            for (v in walk(a.window.decorView).filter { it.isClickable && it.visibility == View.VISIBLE && !(it is TextView && it.isTextSelectable && it !is EditText) }) {
                val h = maxOf(v.minimumHeight, v.height)
                assertTrue("$p ${v.javaClass.simpleName} ${(v as? TextView)?.text} h=$h", h >= dp(a, 48))
            }
        }
    }

    @Test fun switchRowsReadAsOneSwitchAndApplyAtOnce() {
        val a = launch()
        val row = rowTitled(a, "Mock engine")
        val info = row.createAccessibilityNodeInfo()
        assertEquals(Switch::class.java.name, info.className)
        assertTrue(info.isCheckable); assertFalse(info.isChecked)
        assertTrue(info.text.toString().startsWith("Mock engine. "))
        // The inner Switch is not a separate TalkBack stop.
        assertTrue(walk(row).filterIsInstance<Switch>().all { it.importantForAccessibility == View.IMPORTANT_FOR_ACCESSIBILITY_NO && !it.isFocusable })
        val app = FlowApp.of(a)
        row.performClick()
        assertTrue("applies without Save", app.settings.load().mockEngine)
        assertTrue(row.createAccessibilityNodeInfo().isChecked)
        row.performClick()
        assertFalse(app.settings.load().mockEngine)
    }

    @Test fun choiceRowsReadAsRadioButtons() {
        val a = launch(Page.HISTORY)
        val keep = rowTitled(a, "Keep").createAccessibilityNodeInfo()
        assertEquals(android.widget.RadioButton::class.java.name, keep.className)
        assertTrue(keep.isChecked)
        assertFalse(rowTitled(a, "24 hours").createAccessibilityNodeInfo().isChecked)
    }

    @Test fun stricterRetentionAsksBeforeDeletingAndCancelKeepsIt() {
        val a = launch(Page.HISTORY)
        rowTitled(a, "Never save").performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick()
        shadowOf(android.os.Looper.getMainLooper()).idle()
        assertEquals(dev.flowclone.data.Retention.KEEP, FlowApp.of(a).settings.load().retention)
        assertTrue(rowTitled(a, "Keep").createAccessibilityNodeInfo().isChecked)
    }

    @Test fun formPageSavesOnlyWhenChangedAndValidAndAsksBeforeDiscarding() {
        val ctl = Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(androidx_ctx(), Page.TIMING)).setup().visible()
        val a = ctl.get()
        // The test theme has no action bar: drive the activity's own menu callbacks.
        val menu = org.robolectric.fakes.RoboMenu(a)
        a.onCreateOptionsMenu(menu)
        val home = org.robolectric.fakes.RoboMenuItem(android.R.id.home)
        val save = menu.findItem(1)
        assertFalse("nothing changed yet", save.isEnabled)
        val fields = walk(a.window.decorView).filterIsInstance<EditText>()
        val cap = fields[1]
        cap.setText("30")
        assertFalse("invalid", save.isEnabled)
        assertTrue(texts(a).contains("Enter 1–20 minutes"))
        cap.setText("12")
        assertTrue(save.isEnabled)
        // Back with unsaved edits asks first.
        a.onOptionsItemSelected(home)
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        assertFalse(a.isFinishing)
        d.dismiss()
        a.onOptionsItemSelected(save)
        assertEquals(12, FlowApp.of(a).settings.load().capMinutes)
        assertFalse("clean after Save", save.isEnabled)
        a.onOptionsItemSelected(home)
        assertTrue("clean page leaves without asking", a.isFinishing)
    }

    @Test fun pageMissingFromThisBuildOpensTheHub() {
        val a = launch(Page.CLEANUP)
        assertEquals(if (BuildConfig.CAP_LOCAL) "AI cleanup" else "Settings", a.title.toString())
    }
}
