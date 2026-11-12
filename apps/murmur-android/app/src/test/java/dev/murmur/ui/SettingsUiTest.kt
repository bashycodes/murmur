package dev.murmur.ui

import android.app.Activity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.Switch
import android.widget.TextView
import dev.murmur.BuildConfig
import dev.murmur.MurmurApp
import dev.murmur.data.AppVersion
import dev.murmur.data.DevUnlock
import dev.murmur.data.SettingsPages.Page
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
@Config(sdk = [34], application = MurmurApp::class)
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
        val caps = MurmurApp.of(a).caps
        // AndroidSettings.dc.html: General / Models / Connections / Data & privacy.
        for (want in listOf("General", "Models", "Data & privacy", "Advanced", "About", "Language", "Writing style", "Chats casual · Email formal",
            "Bubble size", "Excluded apps", "Timing and limits", "Speech to text", "Keep history", "Debug log", "Learn from my edits", "Mock engine")) {
            assertTrue(want, want in t)
        }
        // Omitted on purpose: no opacity setting, no screen reading.
        for (gone in listOf("Bubble opacity", "Use what's on screen")) assertFalse(gone, gone in t)
        // docs/hybrid-architecture.md §1.4: `remote` has no on-device engine; its cleanup is OpenRouter's.
        assertEquals(BuildConfig.CAP_LOCAL || caps.cloud, "AI cleanup" in t)
        assertEquals(caps.remote || caps.cloud, "Connections" in t)
        assertEquals(caps.remote, "Your server" in t)
        assertEquals(caps.cloud, dev.murmur.data.OpenRouter.NAME in t)
        assertTrue(t.any { it.startsWith("Version ") && "(${caps.flavor})" in it })
        // remote always shows Developer settings; local / hybrid only once unlocked.
        assertEquals(MurmurApp.of(a).dev.visible, "Developer settings" in t)
    }

    @Test fun hubHasATopBarWithBackAndNoOptionsMenu() {
        val a = launch()
        val back = walk(a.window.decorView).filterIsInstance<android.widget.ImageButton>().first()
        assertEquals("Back", back.contentDescription)
        assertTrue("Settings" in texts(a))
        back.performClick()
        assertTrue(a.isFinishing)
    }

    @Test fun aboutRowShowsTheAppVersionLabelAndSevenTapsStillUnlock() {
        val a = launch()
        val app = MurmurApp.of(a)
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
        assertTrue(t.toString(), "100% · shrinks to a dot · shake to restore" in t)
        assertTrue("Banking apps" in t)
        assertTrue("Until you delete it. Stored only on this phone." in t)
        assertTrue(t.any { it.startsWith("Stops after a 1.5 s pause") })
        val row = rowTitled(a, "Bubble size")
        assertTrue(row.contentDescription.toString(), row.contentDescription.startsWith("Bubble size, 100%"))
    }

    @Test fun debugLogRowAppliesAtOnce() {
        val a = launch()
        val app = MurmurApp.of(a)
        val was = app.dev.debugLog
        val row = rowTitled(a, "Debug log")
        assertEquals(Switch::class.java.name, row.createAccessibilityNodeInfo().className)
        row.performClick()
        assertEquals(!was, app.dev.debugLog)
        row.performClick()
        assertEquals(was, app.dev.debugLog)
    }

    @Test fun everyTouchTargetIsAtLeast48dp() {
        for (p in listOf(Page.HUB, Page.SPEECH, Page.LANGUAGE, Page.STYLE, Page.APP_STYLES, Page.TIMING, Page.BUBBLE, Page.APPS, Page.HISTORY) + if (BuildConfig.CAP_LOCAL) listOf(Page.ENGINE, Page.CLEANUP) else emptyList()) {
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
        val app = MurmurApp.of(a)
        row.performClick()
        assertTrue("applies without Save", app.settings.load().mockEngine)
        assertTrue(row.createAccessibilityNodeInfo().isChecked)
        row.performClick()
        assertFalse(app.settings.load().mockEngine)
    }

    @Test fun bubbleSizeSliderShowsThePercentAndAppliesAtOnce() {
        val a = launch(Page.BUBBLE)
        val app = MurmurApp.of(a)
        val bar = walk(a.window.decorView).filterIsInstance<android.widget.SeekBar>().single()
        assertEquals("Bubble size", bar.contentDescription)
        assertTrue(texts(a).toString(), "100%" in texts(a))
        assertEquals("100%", bar.stateDescription)
        // 75..150 in 5 % steps: 16 positions.
        assertEquals(15, bar.max - bar.min)
        shadowOf(bar).onSeekBarChangeListener.onProgressChanged(bar, bar.max, true)
        assertEquals(1.5f, app.settings.load().bubbleScale)
        assertEquals("the running bubble sees it", 1.5f, app.settings.bubbleScale.value)
        assertTrue("150%" in texts(a))
        shadowOf(bar).onSeekBarChangeListener.onProgressChanged(bar, bar.min, true)
        assertEquals(0.75f, app.settings.load().bubbleScale)
        // Persisted: a fresh store over the same prefs reads it back.
        assertEquals(0.75f, dev.murmur.data.SettingsStore(a).load().bubbleScale)
        app.settings.save(app.settings.load().copy(bubbleScale = 1f))
    }

    @Test fun shrinkToADotSitsUnderShrinkBubbleAndNeedsIt() {
        val a = launch(Page.BUBBLE)
        val app = MurmurApp.of(a)
        val t = texts(a)
        assertTrue(t.toString(), t.indexOf("Shrink bubble") in 0 until t.indexOf("Shrink to a dot"))
        val dot = rowTitled(a, "Shrink to a dot")
        val shrink = rowTitled(a, "Shrink bubble")
        assertTrue("default on", dot.createAccessibilityNodeInfo().isChecked)
        assertTrue(dot.isEnabled)
        dot.performClick()
        assertFalse("applies without Save", app.settings.load().shrinkToDot)
        assertFalse("the running bubble sees it", app.settings.shrinkToDot.value)
        assertFalse("persisted", dev.murmur.data.SettingsStore(a).load().shrinkToDot)
        dot.performClick()
        assertTrue(app.settings.load().shrinkToDot)
        // Only active while "Shrink bubble" is on.
        shrink.performClick()
        assertFalse(app.settings.load().shrinkBubble)
        assertFalse(dot.isEnabled)
        dot.performClick()
        assertTrue("a disabled row ignores taps", app.settings.load().shrinkToDot)
        shrink.performClick()
        assertTrue(dot.isEnabled)
        app.settings.save(app.settings.load().copy(shrinkBubble = false))
        assertFalse("a page opened with Shrink bubble off starts it disabled", rowTitled(launch(Page.BUBBLE), "Shrink to a dot").isEnabled)
        app.settings.save(app.settings.load().copy(shrinkBubble = true, shrinkToDot = true))
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
        assertEquals(dev.murmur.data.Retention.KEEP, MurmurApp.of(a).settings.load().retention)
        assertTrue(rowTitled(a, "Keep").createAccessibilityNodeInfo().isChecked)
    }

    @Test fun formPageSavesOnlyWhenChangedAndValidAndAsksBeforeDiscarding() {
        val ctl = Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(androidx_ctx(), Page.TIMING)).setup().visible()
        val a = ctl.get()
        // Top bar: back first, Save as the trailing action (no options menu).
        val buttons = walk(a.window.decorView).filterIsInstance<android.widget.ImageButton>()
        val back = buttons.first { it.contentDescription == "Back" }
        val save = buttons.first { it.contentDescription.toString().startsWith("Save") }
        assertEquals("Save Timing & limits", save.contentDescription)
        assertFalse("nothing changed yet", save.isEnabled)
        val fields = walk(a.window.decorView).filterIsInstance<EditText>()
        val cap = fields[1]
        cap.setText("30")
        assertFalse("invalid", save.isEnabled)
        assertTrue(texts(a).contains("Enter 1–20 minutes"))
        cap.setText("12")
        assertTrue(save.isEnabled)
        // Back with unsaved edits asks first.
        back.performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        assertFalse(a.isFinishing)
        d.dismiss()
        save.performClick()
        assertEquals(12, MurmurApp.of(a).settings.load().capMinutes)
        assertFalse("clean after Save", save.isEnabled)
        back.performClick()
        assertTrue("clean page leaves without asking", a.isFinishing)
        MurmurApp.of(a).settings.save(MurmurApp.of(a).settings.load().copy(capMinutes = dev.murmur.data.Settings().capMinutes))
    }

    /** D27: one choice group per bucket, apply at once, with an example line per choice. */
    @Test fun writingStylePageAppliesAtOnce() {
        val a = launch(Page.STYLE)
        val t = texts(a)
        for (want in listOf("PERSONAL MESSAGES", "WORK MESSAGES", "EMAIL", "OTHER", "Style per app", "Built-in list only")) assertTrue(want, want in t)
        assertTrue(t.toString(), t.any { it.startsWith("“Sounds good. See you at 7:30”") })
        val app = MurmurApp.of(a)
        // Second "Very casual" row = Work messages (bucket order: personal, work, email, other).
        val rows = walk(a.window.decorView).filter { v -> v.isClickable && v is ViewGroup && walk(v).any { it is TextView && it.text.toString() == "Very casual" } }
        assertEquals(4, rows.size)
        rows[1].performClick()
        assertEquals(dev.murmur.core.Style.VERY_CASUAL, app.settings.load().styles.work)
        assertEquals(dev.murmur.core.Style.CASUAL, app.settings.load().styles.personal)
        assertTrue(rows[1].createAccessibilityNodeInfo().isChecked)
        app.settings.save(app.settings.load().copy(styles = dev.murmur.core.Styles()))
    }

    @Test fun pageMissingFromThisBuildOpensTheHub() {
        val a = launch(Page.CLEANUP)
        assertEquals(if (BuildConfig.CAP_LOCAL) "AI cleanup" else "Settings", a.title.toString())
    }

    @Test fun hubOffersBackupAndRestoreThroughTheSystemFilePicker() {
        val a = launch()
        val t = texts(a)
        for (want in listOf("Back up & restore", "Export all data", "Restore from a backup", dev.murmur.data.Backup.SECRETS_NOTE)) assertTrue(want, want in t)
        rowTitled(a, "Export all data").performClick()
        val export = shadowOf(a).nextStartedActivityForResult.intent
        assertEquals(android.content.Intent.ACTION_CREATE_DOCUMENT, export.action)
        assertTrue(export.getStringExtra(android.content.Intent.EXTRA_TITLE)!!.matches(Regex("""murmur-backup-\d{4}-\d{2}-\d{2}\.json""")))
        rowTitled(a, "Restore from a backup").performClick()
        assertEquals(android.content.Intent.ACTION_OPEN_DOCUMENT, shadowOf(a).nextStartedActivityForResult.intent.action)
    }
}
