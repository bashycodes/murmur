package dev.murmur.ui

import android.content.ClipboardManager
import dev.murmur.MurmurApp
import dev.murmur.data.HistoryRow
import dev.murmur.data.Retention
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
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
import org.robolectric.shadows.ShadowPopupMenu
import org.robolectric.shadows.ShadowSettings

/** Home tab (AndroidHome): day-grouped dictation cards with route tag, Copy and ⋮ (details, delete), empty state, notices. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class MainUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)

    @After fun reset() { app.history.clear(); ShadowSettings.setCanDrawOverlays(false) }

    private fun add(text: String, route: String? = "remote", at: Long = System.currentTimeMillis()) =
        app.history.add(HistoryRow(0, at, "com.example.chat", "FORMATTED", text.lowercase(), text, true, 2_000, route = route), Retention.KEEP)

    private fun launch(): MainActivity {
        val a = Robolectric.buildActivity(MainActivity::class.java).setup().visible().get()
        val home = a.tab(dev.murmur.data.ShellPages.Tab.HOME) as HomeTab
        repeat(200) { idle(); if (home.loaded) return a; Thread.sleep(10) }
        idle(); return a
    }

    @Test fun topBarTitleAndEmptyState() {
        val a = launch()
        val t = texts(a)
        assertTrue(t.toString(), "Murmur" in t)
        assertTrue("No dictations yet" in t)
        for (want in listOf("Home", "Dictionary", "Style", "Snippets")) assertTrue(want, want in t)
        // The old main screen's sections are gone (Permissions has its own page now).
        assertFalse("SETUP" in t)
        assertFalse("Microphone" in t)
    }

    @Test fun cardsAreGroupedByDayWithRouteTagAndTalkBackText() {
        add("Running ten minutes late.", route = "remote")
        add("Send me the address again?", route = "local")
        add("Groceries on the way home?", route = null, at = System.currentTimeMillis() - 86_400_000L)
        val a = launch()
        val t = texts(a)
        assertTrue(t.toString(), "Today" in t)
        assertTrue("Yesterday" in t)
        assertTrue("Running ten minutes late." in t)
        assertTrue("Your server" in t)
        assertTrue("This phone" in t)
        assertTrue("Today above its cards", t.indexOf("Today") < t.indexOf("Running ten minutes late."))
        assertTrue("newest first", t.indexOf("Send me the address again?") < t.indexOf("Running ten minutes late."))
        val card = walk(a.window.decorView).first { it.isFocusable && it.contentDescription?.startsWith("Running ten minutes late.") == true }
        assertTrue(card.contentDescription.toString(), card.contentDescription.endsWith(", Your server"))
        UiTestKit.assertTouchTargets(a, "home")
    }

    @Test fun copyPutsTheTextOnTheClipboard() {
        add("Copy me please.")
        val a = launch()
        walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("Copy dictation from ") == true }.performClick()
        val clip = ctx.getSystemService(ClipboardManager::class.java).primaryClip
        assertEquals("Copy me please.", clip?.getItemAt(0)?.text?.toString())
    }

    @Test fun moreMenuOpensDetailsAndDeletes() {
        add("Delete me.")
        val a = launch()
        val id = app.history.recent().single().id
        val more = walk(a.window.decorView).first { it.isClickable && it.contentDescription?.startsWith("More actions for dictation from ") == true }
        more.performClick()
        val menu = ShadowPopupMenu.getLatestPopupMenu()
        val items = (0 until menu.menu.size()).map { menu.menu.getItem(it).title.toString() }
        assertEquals(listOf("Open details", "Delete"), items)
        assertFalse("no Retry: audio is never stored", items.any { it.startsWith("Retry") })
        shadowOf(menu).onMenuItemClickListener.onMenuItemClick(menu.menu.getItem(0))
        val i = shadowOf(a).nextStartedActivity
        assertEquals(HistoryActivity::class.java.name, i.component?.className)
        assertEquals(id, i.getLongExtra(HistoryActivity.EXTRA_ID, -1))

        more.performClick()
        val m2 = ShadowPopupMenu.getLatestPopupMenu()
        shadowOf(m2).onMenuItemClickListener.onMenuItemClick(m2.menu.getItem(1))
        ShadowAlertDialog.getLatestAlertDialog().getButton(android.app.AlertDialog.BUTTON_POSITIVE).performClick()
        repeat(200) { idle(); if ("No dictations yet" in texts(a)) return@repeat; Thread.sleep(10) }
        assertTrue(app.history.recent().isEmpty())
        assertTrue(texts(a).toString(), "No dictations yet" in texts(a))
    }

    @Test fun missingSetupShowsANoticeThatOpensPermissions() {
        val a = launch()
        val notice = walk(a.window.decorView).first { it.isClickable && (it as? android.widget.TextView)?.text?.startsWith("Dictation can't start yet.") == true }
        assertTrue(notice.toString(), (notice as android.widget.TextView).text.toString().endsWith("in Permissions."))
        notice.performClick()
        assertEquals(PermissionsActivity::class.java.name, shadowOf(a).nextStartedActivity.component?.className)
    }
}
