package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.ResolveInfo
import android.widget.ImageButton
import android.widget.Spinner
import dev.flowclone.FlowApp
import dev.flowclone.data.SettingsPages.Page
import dev.flowclone.data.SettingsStore
import dev.flowclone.policy.AppCategorizer.Category
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/** Settings → Writing style → Style per app: override rows, the "Add app" picker, and what is stored. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class AppStylesUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    private fun install(pkg: String, label: String) {
        val ai = ApplicationInfo().apply { packageName = pkg; nonLocalizedLabel = label }
        val pm = shadowOf(ctx.packageManager)
        pm.installPackage(PackageInfo().apply { packageName = pkg; applicationInfo = ai })
        val ri = ResolveInfo().apply {
            activityInfo = ActivityInfo().apply { packageName = pkg; name = "$pkg.Main"; applicationInfo = ai; nonLocalizedLabel = label }
            nonLocalizedLabel = label
        }
        pm.addResolveInfoForIntent(Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), ri)
    }

    @Before fun apps() {
        install("com.google.android.gm", "Gmail")
        install("com.Slack", "Slack")
    }

    @After fun reset() { app.settings.save(app.settings.load().copy(appBuckets = emptyMap())) }

    private fun launch(): Activity =
        Robolectric.buildActivity(SettingsActivity::class.java, SettingsActivity.intent(ctx, Page.APP_STYLES)).setup().visible().get().also { idle() }

    /** The page loads the launchable apps on a thread and posts back (an open picker refills): wait for [ready]. */
    private fun settle(ready: () -> Boolean) {
        repeat(500) { idle(); if (ready()) return; Thread.sleep(10) }
        idle()
    }

    private fun dialogTexts(d: android.app.Dialog) = walk(d.window!!.decorView).filterIsInstance<android.widget.TextView>().map { it.text.toString() }

    private fun openPicker(a: Activity): android.app.AlertDialog {
        walk(a.window.decorView).filterIsInstance<android.widget.Button>().single { it.text.toString() == "Add app" }.performClick()
        idle()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        settle { "Gmail" in dialogTexts(d) }
        return d
    }

    @Test fun typedOverridesShowAsRowsAndUninstalledOnesCanBeRemoved() {
        // As the old text page stored them: one `package = bucket` line each.
        ctx.getSharedPreferences("flow_settings", Context.MODE_PRIVATE).edit()
            .putString("app_buckets", "com.gone.app = work\ncom.google.android.gm = personal").commit()
        val a = launch()
        val t = texts(a)
        assertTrue(t.toString(), "Gmail" in t)
        assertTrue(t.toString(), "com.gone.app" in t)
        assertTrue(t.toString(), "Not installed · Built-in: Other" in t)

        val spinners = walk(a.window.decorView).filterIsInstance<Spinner>()
        assertEquals(listOf("Group for com.gone.app", "Group for Gmail"), spinners.map { it.contentDescription.toString() })
        assertEquals(listOf("Work", "Personal"), spinners.map { it.selectedItem.toString() })

        walk(a.window.decorView).filterIsInstance<ImageButton>().single { it.contentDescription == "Remove com.gone.app" }.performClick()
        idle()
        assertEquals(mapOf("com.google.android.gm" to Category.PERSONAL), app.settings.load().appBuckets)
        assertTrue("com.gone.app" !in texts(a))

        // Dropdown: Gmail → Email, applied at once and persisted in the same format.
        val gmail = walk(a.window.decorView).filterIsInstance<Spinner>().single()
        gmail.onItemSelectedListener!!.onItemSelected(gmail, null, 2, 2)
        assertEquals(mapOf("com.google.android.gm" to Category.EMAIL), SettingsStore(a).load().appBuckets)
        assertEquals(
            "com.google.android.gm = email",
            ctx.getSharedPreferences("flow_settings", Context.MODE_PRIVATE).getString("app_buckets", null),
        )
    }

    @Test fun addAppPicksFromTheInstalledListWithoutTypingAPackage() {
        val a = launch()
        assertTrue("No apps moved" in texts(a))
        val picker = openPicker(a)
        val rows = walk(picker.window!!.decorView)
        val gmailRow = rows.first { v -> v.isClickable && v is android.view.ViewGroup && walk(v).any { it is android.widget.TextView && it.text.toString() == "Gmail" } }
        assertTrue(walk(gmailRow).any { it is android.widget.TextView && it.text.toString() == "Built-in: Email" })
        gmailRow.performClick()
        idle()
        val groups = ShadowAlertDialog.getLatestAlertDialog()
        assertEquals("Email preselected (built-in group)", 2, groups.listView.checkedItemPosition)
        shadowOf(groups).clickOnItem(1) // Work
        idle()
        assertEquals(mapOf("com.google.android.gm" to Category.WORK), SettingsStore(a).load().appBuckets)
        assertTrue("Gmail" in texts(a))
    }

    @Test fun pickerPutsRecentlyDictatedAppsFirstAndSearches() {
        app.history.add(
            dev.flowclone.data.HistoryRow(0, System.currentTimeMillis(), "com.Slack", "RAW_TRANSCRIPT", "hi", "hi", true, 1_000),
            dev.flowclone.data.Retention.KEEP,
        )
        try {
            val a = launch()
            val d = openPicker(a)
            val labels = dialogTexts(d)
                .filter { it in setOf("RECENTLY DICTATED INTO", "ALL APPS", "Slack", "Gmail") }
            assertEquals(listOf("RECENTLY DICTATED INTO", "Slack", "ALL APPS", "Gmail"), labels)
            walk(d.window!!.decorView).filterIsInstance<android.widget.EditText>().single().setText("gma")
            idle()
            val shown = dialogTexts(d)
            assertTrue("Gmail" in shown)
            assertTrue("Slack" !in shown)
        } finally {
            app.history.clear()
        }
    }
}
