package dev.murmur.ui

import android.content.DialogInterface
import android.content.Intent
import android.view.View
import android.widget.EditText
import dev.murmur.MurmurApp
import dev.murmur.core.DictEntry
import dev.murmur.data.DictionaryPages.Kind
import dev.murmur.data.ShellPages.Tab
import dev.murmur.learn.Suggestion
import dev.murmur.learn.Suggestions
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import dev.murmur.ui.tabs.EntryTab
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/**
 * Dictionary & Snippets tabs (search, rows, FAB, delete, suggestions, import / export) and the
 * add / edit page (top-bar Save / Delete, inline errors, Discard prompt).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class DictionaryUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)
    private val store get() = app.dictionary

    @After fun clear() {
        store.replaceAll(emptyList())
        app.suggestions.update { Suggestions() }
    }

    private fun launch(i: Intent) = Robolectric.buildActivity(DictionaryActivity::class.java, i).setup().visible().get()

    private fun tab(t: Tab): Pair<MainActivity, EntryTab> {
        val a = Robolectric.buildActivity(MainActivity::class.java).setup().visible().get()
        idle()
        a.frame.navView(t).performClick(); idle()
        assertEquals(t, a.current)
        return a to a.tab(t) as EntryTab
    }

    private fun byDescription(a: android.app.Activity, d: String): View = walk(a.window.decorView).first { it.contentDescription?.toString() == d && UiTestKit.visible(it) }

    // ---- tabs ----

    @Test fun dictionaryTabIsEmbeddedWithSearchRowsAndFab() {
        store.replaceAll(listOf(DictEntry("jason", "JSON", false), DictEntry("Wolt", null, false), DictEntry("sig", "Best", true)))
        val (a, t) = tab(Tab.DICTIONARY)
        assertTrue(a.frame.navView(Tab.DICTIONARY).isSelected)
        val tx = texts(a)
        assertTrue(tx.toString(), "jason →" in tx && "JSON" in tx && "Wolt" in tx)
        assertFalse("snippets stay on their tab", "sig" in tx)
        assertFalse("no learned marker", tx.any { "✨" in it })
        assertEquals("Search dictionary", t.search.contentDescription)
        assertEquals("Search", t.search.hint)
        assertEquals("Add a word", t.fab.contentDescription)
        // The FAB floats at the bottom right of the tab host, i.e. above the bottom nav.
        val lp = t.fab.layoutParams as android.widget.FrameLayout.LayoutParams
        assertEquals(android.view.Gravity.END or android.view.Gravity.BOTTOM, lp.gravity)
        assertTrue(generateSequence(t.fab.parent as View?) { it.parent as? View }.any { it === a.frame.host })
        // One TalkBack stop per row: "heard, replaced with word".
        byDescription(a, "jason, replaced with JSON")
        UiTestKit.assertTouchTargets(a, "dictionary tab")
        t.fab.performClick()
        val i = shadowOf(a).nextStartedActivity
        assertEquals(DictionaryActivity::class.java.name, i.component?.className)
        assertEquals(Kind.WORDS.name, i.getStringExtra(DictionaryActivity.EXTRA_KIND))
        assertFalse(i.hasExtra(DictionaryActivity.EXTRA_WORD))
    }

    @Test fun searchFiltersByWordOrReplacementAndShowsNoMatch() {
        store.replaceAll(listOf(DictEntry("jason", "JSON", false), DictEntry("Wolt", null, false)))
        val (a, t) = tab(Tab.DICTIONARY)
        t.search.setText("json")
        assertTrue("JSON" in texts(a)); assertFalse("Wolt" in texts(a))
        t.search.setText("zzz")
        assertTrue("Nothing matches “zzz”" in texts(a))
        t.search.setText("")
        assertTrue("Wolt" in texts(a))
    }

    @Test fun emptyTabsShowTheirEmptyState() {
        val (a, _) = tab(Tab.DICTIONARY)
        assertTrue("No words yet" in texts(a))
        a.frame.navView(Tab.SNIPPETS).performClick(); idle()
        assertTrue("No snippets yet" in texts(a))
    }

    @Test fun tapEditsAndDeleteIsATalkBackActionThatAsksFirst() {
        store.replaceAll(listOf(DictEntry("kubectl", null, false)))
        val (a, _) = tab(Tab.DICTIONARY)
        val row = byDescription(a, "kubectl")
        row.performClick()
        val i = shadowOf(a).nextStartedActivity
        assertEquals(DictionaryActivity.PAGE_EDIT, i.getStringExtra(DictionaryActivity.EXTRA_PAGE))
        assertEquals("kubectl", i.getStringExtra(DictionaryActivity.EXTRA_WORD))
        val del = row.createAccessibilityNodeInfo().actionList.first { it.label == "Delete" }
        row.performAccessibilityAction(del.id, null)
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d)
        d.getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(store.all().isEmpty())
        assertTrue("No words yet" in texts(a))
    }

    @Test fun snippetsTabShowsCardsSearchAndFab() {
        store.replaceAll(listOf(DictEntry("my sign-off", "Thanks,\nAlex", true), DictEntry("on my way", "On my way, be there in 15 minutes.", true), DictEntry("Wolt", null, false)))
        val (a, t) = tab(Tab.SNIPPETS)
        val tx = texts(a)
        assertTrue(tx.toString(), "my sign-off" in tx && "Thanks, Alex" in tx && "on my way" in tx)
        assertFalse("Wolt" in tx)
        assertEquals("Search snippets", t.search.contentDescription)
        assertEquals("Add a snippet", t.fab.contentDescription)
        byDescription(a, "my sign-off, inserts Thanks, Alex")
        UiTestKit.assertTouchTargets(a, "snippets tab")
        t.search.setText("15 min")
        assertTrue("on my way" in texts(a)); assertFalse("my sign-off" in texts(a))
        t.fab.performClick()
        assertEquals(Kind.SNIPPETS.name, shadowOf(a).nextStartedActivity.getStringExtra(DictionaryActivity.EXTRA_KIND))
    }

    @Test fun tabReloadsWhenTheShellResumes() {
        val ctl = Robolectric.buildActivity(MainActivity::class.java).setup().visible()
        val a = ctl.get(); idle()
        a.frame.navView(Tab.DICTIONARY).performClick(); idle()
        ctl.pause()
        store.replaceAll(listOf(DictEntry("Tailscale", null, false)))
        ctl.resume(); idle()
        assertTrue("Tailscale" in texts(a))
    }

    @Test fun suggestionsShowAboveTheWordsAndAddAddsTheWord() {
        app.suggestions.update { Suggestions(listOf(Suggestion("Hetzner", "hetsner", "pkg", 1L))) }
        val (a, _) = tab(Tab.DICTIONARY)
        assertTrue(texts(a).toString(), "Suggestions" in texts(a) && "Hetzner" in texts(a))
        walk(a.window.decorView).first { it is android.widget.Button && it.text.toString() == "Add" }.performClick(); idle()
        assertEquals(listOf(DictEntry("Hetzner")), store.all())
        assertFalse("Suggestions" in texts(a))
        UiTestKit.assertTouchTargets(a, "suggestions")
    }

    @Test fun importExportActionOpensTheFilePicker() {
        val (a, _) = tab(Tab.DICTIONARY)
        byDescription(a, "Import or export").performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        shadowOf(d).clickOnItem(1)
        val i = shadowOf(a).nextStartedActivity
        assertEquals(DictionaryActivity.PAGE_EXPORT, i.getStringExtra(DictionaryActivity.EXTRA_PAGE))
        val t = launch(i)
        val req = shadowOf(t).nextStartedActivityForResult
        assertEquals(Intent.ACTION_CREATE_DOCUMENT, req.intent.action)
        shadowOf(t).receiveResult(req.intent, android.app.Activity.RESULT_CANCELED, null)
        assertTrue(t.isFinishing)
    }

    // ---- add / edit page ----

    @Test fun editPageHasBackTitleAndSaveInTheTopBar() {
        store.replaceAll(listOf(DictEntry("tailscail", "Tailscale", false)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.WORDS, null))
        assertEquals("Add word", a.title.toString())
        assertTrue("Add word" in texts(a))
        byDescription(a, "Back")
        val save = a.saveButton!!
        assertEquals("Save word", save.contentDescription)
        assertFalse("pristine", save.isEnabled)
        assertTrue("no Delete on a new entry", walk(a.window.decorView).none { it.contentDescription == "Delete word" })
        assertFalse("no error on a pristine form", texts(a).contains("Enter a word or trigger phrase."))
        val (word, repl) = walk(a.window.decorView).filterIsInstance<EditText>().let { it[0] to it[1] }
        word.setText("Tailscail")
        assertFalse(save.isEnabled)
        assertTrue(texts(a).toString(), "\"Tailscail\" is already in the dictionary." in texts(a))
        word.setText("Kubernetes")
        assertTrue(save.isEnabled)
        repl.setText("")
        save.performClick()
        assertEquals(DictEntry("Kubernetes", null, false), store.all().last())
        assertTrue(a.isFinishing)
    }

    @Test fun snippetSwitchNeedsAnExpansionAndBackOnADirtyFormAsks() {
        store.replaceAll(listOf(DictEntry("kubectl", null, false)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.WORDS, "kubectl"))
        assertEquals("Edit word", a.title.toString())
        val toggle = rowTitled(a, "Snippet")
        assertEquals(android.widget.Switch::class.java.name, toggle.createAccessibilityNodeInfo().className)
        toggle.performClick()
        assertTrue("Expands to" in texts(a))
        assertTrue("Edit snippet" in texts(a))
        assertTrue("A snippet needs the text it expands to." in texts(a))
        assertFalse(a.saveButton!!.isEnabled)
        byDescription(a, "Back").performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        assertFalse(a.isFinishing)
        d.getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(a.isFinishing)
        assertEquals(listOf(DictEntry("kubectl", null, false)), store.all())
    }

    @Test fun backOnACleanFormLeaves() {
        store.replaceAll(listOf(DictEntry("kubectl", null, false)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.WORDS, "kubectl"))
        byDescription(a, "Back").performClick()
        assertTrue(a.isFinishing)
    }

    @Test fun topBarDeleteAsksAndTouchTargets() {
        store.replaceAll(listOf(DictEntry("sig", "Best,\nAlex", true)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.SNIPPETS, "sig"))
        UiTestKit.assertTouchTargets(a, "edit")
        byDescription(a, "Delete snippet").performClick()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(store.all().isEmpty())
        assertTrue(a.isFinishing)
    }

    @Test fun editingAMissingEntryCloses() {
        val ctl = Robolectric.buildActivity(DictionaryActivity::class.java, DictionaryActivity.editIntent(ctx, Kind.WORDS, "gone")).setup()
        assertTrue(ctl.get().isFinishing)
    }
}
