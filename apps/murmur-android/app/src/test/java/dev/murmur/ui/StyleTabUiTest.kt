package dev.flowclone.ui

import android.content.Intent
import android.content.pm.ActivityInfo
import android.content.pm.ApplicationInfo
import android.content.pm.PackageInfo
import android.content.pm.ResolveInfo
import android.view.View
import android.widget.TextView
import dev.flowclone.FlowApp
import dev.flowclone.core.Style
import dev.flowclone.core.Styles
import dev.flowclone.data.ShellPages.Tab
import dev.flowclone.data.SettingsStore
import dev.flowclone.policy.AppCategorizer.Category
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import dev.flowclone.ui.tabs.StyleTab
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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

/** Style tab (AndroidStyle.dc.html): group tabs, the group's apps, style cards that save on tap. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class StyleTabUiTest {
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
        install("com.whatsapp", "WhatsApp")
        install("com.Slack", "Slack")
    }

    @After fun reset() { app.settings.save(app.settings.load().copy(appBuckets = emptyMap(), styles = Styles())) }

    private fun settle(ready: () -> Boolean) {
        repeat(500) { idle(); if (ready()) return; Thread.sleep(10) }
        idle()
    }

    private fun appRow(a: MainActivity): View? = walk(a.window.decorView).firstOrNull { it.contentDescription?.startsWith("Apps in ") == true }

    /** The shell with the Style tab selected, its apps loaded. */
    private fun launch(): Pair<MainActivity, StyleTab> {
        val a = Robolectric.buildActivity(MainActivity::class.java).setup().visible().get().also { idle() }
        a.frame.navView(Tab.STYLE).performClick()
        idle()
        settle { appRow(a) != null }
        return a to a.tab(Tab.STYLE) as StyleTab
    }

    private fun tab(a: MainActivity, label: String): TextView =
        walk(a.window.decorView).filterIsInstance<TextView>().single { it.text.toString() == label && it.isClickable }

    private fun card(a: MainActivity, name: String): View =
        walk(a.window.decorView).single { it.isClickable && it.contentDescription?.startsWith("$name. ") == true }

    @Test fun styleIsAnEmbeddedTabShowingTheGroupAppsAndStyles() {
        val (a, _) = launch()
        assertEquals(Tab.STYLE, a.current)
        assertTrue(a.frame.navView(Tab.STYLE).isSelected)
        assertEquals(null, shadowOf(a).nextStartedActivity)
        val t = texts(a)
        for (want in listOf("Personal", "Work", "Email", "Other", "Used when you dictate into chat apps for friends and family", "Formal", "Casual", "Very casual", "Capitals, lighter punctuation"))
            assertTrue("$want in $t", want in t)
        assertTrue("English only" !in t.joinToString())
        assertEquals("Apps in Personal: WhatsApp", appRow(a)!!.contentDescription)
        UiTestKit.assertTouchTargets(a, "style tab")
    }

    @Test fun groupTabsReadAsCheckableAndSwitchAppsAndStyle() {
        val (a, st) = launch()
        val personal = tab(a, "Personal").createAccessibilityNodeInfo()
        assertTrue(personal.isCheckable); assertTrue(personal.isChecked)
        assertFalse(tab(a, "Email").createAccessibilityNodeInfo().isChecked)
        assertTrue(card(a, "Casual").createAccessibilityNodeInfo().isChecked)

        tab(a, "Email").performClick()
        idle()
        assertEquals(Category.EMAIL, st.category)
        assertTrue(tab(a, "Email").createAccessibilityNodeInfo().isChecked)
        assertTrue("Used when you dictate into email" in texts(a))
        assertEquals("Apps in Email: Gmail", appRow(a)!!.contentDescription)
        // Email defaults to Formal: that card is the checked one, read with its rule and example.
        val formal = card(a, "Formal")
        assertTrue(formal.createAccessibilityNodeInfo().isChecked)
        assertEquals("Formal. Capitals, full punctuation. Example: Sounds good. See you at 7:30.", formal.contentDescription)
        assertFalse(card(a, "Casual").createAccessibilityNodeInfo().isChecked)
    }

    @Test fun tappingAStyleCardSavesItForThatGroupOnly() {
        val (a, _) = launch()
        tab(a, "Work").performClick()
        idle()
        card(a, "Very casual").performClick()
        idle()
        val s = SettingsStore(a).load().styles
        assertEquals(Style.VERY_CASUAL, s.work)
        assertEquals(Style.CASUAL, s.personal)
        assertTrue(card(a, "Very casual").createAccessibilityNodeInfo().isChecked)
        assertFalse(card(a, "Casual").createAccessibilityNodeInfo().isChecked)
    }

    @Test fun plusOpensTheAddAppPickerAndPutsTheAppInTheShownGroup() {
        val (a, _) = launch()
        walk(a.window.decorView).single { it.contentDescription == "Add an app to Personal" }.performClick()
        idle()
        val picker = ShadowAlertDialog.getLatestAlertDialog()
        settle { walk(picker.window!!.decorView).any { it is TextView && it.text.toString() == "Slack" } }
        walk(picker.window!!.decorView).first { v -> v.isClickable && v is android.view.ViewGroup && walk(v).any { it is TextView && it.text.toString() == "Slack" } }.performClick()
        idle()
        val groups = ShadowAlertDialog.getLatestAlertDialog()
        assertEquals("the shown group preselected", 0, groups.listView.checkedItemPosition)
        shadowOf(groups).clickOnItem(0)
        idle()
        assertEquals(mapOf("com.Slack" to Category.PERSONAL), SettingsStore(a).load().appBuckets)
        assertEquals("Apps in Personal: Slack, WhatsApp", appRow(a)!!.contentDescription)
    }
}
