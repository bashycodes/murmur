package dev.flowclone.ui

import android.content.Intent
import android.widget.EditText
import dev.flowclone.FlowApp
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.rowTitled
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
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
import org.robolectric.fakes.RoboMenu
import org.robolectric.fakes.RoboMenuItem
import org.robolectric.shadows.ShadowAlertDialog

/** Dictionary & snippets: hub counts, list pages (empty state, search, delete), edit form Save / Discard. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class DictionaryUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val store get() = FlowApp.of(ctx).dictionary

    @After fun clear() { store.replaceAll(emptyList()) }

    private fun launch(i: Intent) = Robolectric.buildActivity(DictionaryActivity::class.java, i).setup().visible().get()

    @Test fun hubShowsBothListsWithCountsAndImportExport() {
        store.replaceAll(listOf(DictEntry("wisper", "Wispr", false), DictEntry("sig", "Best", true)))
        val a = launch(DictionaryActivity.intent(ctx))
        val t = texts(a)
        for (want in listOf("LISTS", "Dictionary", "Snippets", "IMPORT & EXPORT", "Import JSON", "Export JSON", "1 word · 1 replacement", "1 snippet")) assertTrue(want, want in t)
        rowTitled(a, "Snippets").performClick()
        val i = shadowOf(a).nextStartedActivity
        assertEquals(DictionaryActivity.PAGE_LIST, i.getStringExtra(DictionaryActivity.EXTRA_PAGE))
        assertEquals(Kind.SNIPPETS.name, i.getStringExtra(DictionaryActivity.EXTRA_KIND))
        rowTitled(a, "Import JSON").performClick()
        assertEquals(Intent.ACTION_OPEN_DOCUMENT, shadowOf(a).nextStartedActivityForResult.intent.action)
        UiTestKit.assertTouchTargets(a, "hub")
    }

    @Test fun emptyListShowsItsEmptyStateAndNoSearch() {
        val a = launch(DictionaryActivity.intent(ctx, DictionaryActivity.PAGE_LIST, Kind.WORDS))
        assertTrue("No words yet" in texts(a))
        assertTrue(walk(a.window.decorView).none { it is EditText })
        val menu = RoboMenu(a); a.onCreateOptionsMenu(menu)
        assertEquals("Add", menu.findItem(1).title.toString())
        a.onOptionsItemSelected(menu.findItem(1))
        val i = shadowOf(a).nextStartedActivity
        assertEquals(DictionaryActivity.PAGE_EDIT, i.getStringExtra(DictionaryActivity.EXTRA_PAGE))
        assertFalse(i.hasExtra(DictionaryActivity.EXTRA_WORD))
    }

    @Test fun longListGetsSearchAndOnlyItsKind() {
        store.replaceAll(List(DictionaryPages.SEARCH_FROM) { DictEntry("word$it", null, false) } + DictEntry("sig", "Best", true))
        val a = launch(DictionaryActivity.intent(ctx, DictionaryActivity.PAGE_LIST, Kind.WORDS))
        assertTrue("word3" in texts(a))
        assertFalse("sig" in texts(a))
        val search = walk(a.window.decorView).filterIsInstance<EditText>().single()
        assertEquals("Search words", search.contentDescription)
        search.setText("word7")
        assertTrue("word7" in texts(a)); assertFalse("word3" in texts(a))
        search.setText("zzz")
        assertTrue("Nothing matches “zzz”" in texts(a))
    }

    @Test fun rowsOfferDeleteAsATalkBackActionThatAsksFirst() {
        store.replaceAll(listOf(DictEntry("kubectl", null, false)))
        val a = launch(DictionaryActivity.intent(ctx, DictionaryActivity.PAGE_LIST, Kind.WORDS))
        val row = rowTitled(a, "kubectl")
        assertTrue(row.contentDescription.toString(), row.contentDescription.startsWith("kubectl, Spelling hint"))
        val del = row.createAccessibilityNodeInfo().actionList.first { it.label == "Delete" }
        row.performAccessibilityAction(del.id, null)
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d)
        d.getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(store.all().isEmpty())
        assertTrue("No words yet" in texts(a))
    }

    @Test fun editFormSavesOnlyWhenChangedAndValid() {
        store.replaceAll(listOf(DictEntry("wisper", "Wispr", false)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.WORDS, null))
        assertEquals("Add word", a.title.toString())
        val menu = RoboMenu(a); a.onCreateOptionsMenu(menu)
        val save = menu.findItem(2)
        assertFalse("pristine", save.isEnabled)
        assertFalse("no error on a pristine form", texts(a).contains("Enter a word or trigger phrase."))
        val (word, repl) = walk(a.window.decorView).filterIsInstance<EditText>().let { it[0] to it[1] }
        word.setText("Wisper")
        assertFalse(save.isEnabled)
        assertTrue(texts(a).toString(), "\"Wisper\" is already in the dictionary." in texts(a))
        word.setText("Tailscale")
        assertTrue(save.isEnabled)
        repl.setText("")
        a.onOptionsItemSelected(save)
        assertEquals(DictEntry("Tailscale", null, false), store.all().last())
        assertTrue(a.isFinishing)
    }

    @Test fun snippetSwitchNeedsAnExpansionAndLeavingDirtyAsks() {
        store.replaceAll(listOf(DictEntry("kubectl", null, false)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.WORDS, "kubectl"))
        assertEquals("Edit word", a.title.toString())
        val menu = RoboMenu(a); a.onCreateOptionsMenu(menu)
        val save = menu.findItem(2)
        val toggle = rowTitled(a, "Snippet")
        assertEquals(android.widget.Switch::class.java.name, toggle.createAccessibilityNodeInfo().className)
        toggle.performClick()
        assertTrue("Expands to" in texts(a))
        assertTrue("A snippet needs the text it expands to." in texts(a))
        assertFalse(save.isEnabled)
        a.onOptionsItemSelected(RoboMenuItem(android.R.id.home))
        val d = ShadowAlertDialog.getLatestAlertDialog()
        assertNotNull(d); assertTrue(d.isShowing)
        assertFalse(a.isFinishing)
        d.getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(a.isFinishing)
        assertEquals(listOf(DictEntry("kubectl", null, false)), store.all())
    }

    @Test fun editPageDeleteAndTouchTargets() {
        store.replaceAll(listOf(DictEntry("sig", "Best,\nAlex", true)))
        val a = launch(DictionaryActivity.editIntent(ctx, Kind.SNIPPETS, "sig"))
        UiTestKit.assertTouchTargets(a, "edit")
        walk(a.window.decorView).first { it is android.widget.Button && (it as android.widget.Button).text == "Delete snippet" }.performClick()
        ShadowAlertDialog.getLatestAlertDialog().getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick(); idle()
        assertTrue(store.all().isEmpty())
        assertTrue(a.isFinishing)
    }

    @Test fun editingAMissingEntryCloses() {
        val ctl = Robolectric.buildActivity(DictionaryActivity::class.java, DictionaryActivity.editIntent(ctx, Kind.WORDS, "gone")).setup()
        assertTrue(ctl.get().isFinishing)
    }

}
