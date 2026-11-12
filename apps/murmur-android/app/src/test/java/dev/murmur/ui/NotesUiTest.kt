package dev.murmur.ui

import android.content.DialogInterface
import android.content.Intent
import android.widget.EditText
import dev.murmur.MurmurApp
import dev.murmur.notes.DocumentStore
import dev.murmur.notes.NotesView
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.rowTitled
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
import org.robolectric.android.controller.ActivityController
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.io.File

/** Notes list: titles, Quick note pinned, search, New, Pin, Share, Delete, no bubble here. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class NotesUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)
    private val dir get() = File(ctx.filesDir, "notes")

    @After fun cleanup() { dir.deleteRecursively() }

    /** Store calls run on the documents thread: wait until the list shows [want]. */
    private fun ActivityController<NotesActivity>.await(want: String): NotesActivity {
        repeat(300) { idle(); if (want in texts(get())) return get(); Thread.sleep(10) }
        assertTrue("$want not in ${texts(get())}", want in texts(get()))
        return get()
    }

    private fun awaitGone(a: NotesActivity, title: String) {
        repeat(300) { idle(); if (title !in texts(a)) return; Thread.sleep(10) }
        assertFalse(title in texts(a))
    }

    private fun launch() = Robolectric.buildActivity(NotesActivity::class.java).setup()

    private fun seed() {
        app.documents.save("a1", "Plumber Tuesday\ncall before 9")
        app.documents.save("b2", "Groceries\nmilk, eggs")
    }

    @Test fun listsNotesByFirstLineWithTheOldScratchpadPinnedAsQuickNote() {
        // The Scratchpad from before Notes: only its text file, no index.
        dir.mkdirs(); File(dir, "scratchpad.txt").writeText("kept from before")
        seed()
        val a = launch().await("Plumber Tuesday")
        val t = texts(a)
        assertTrue(t.toString(), NotesView.QUICK_NOTE in t && "Groceries" in t && NotesActivity.PINNED in t)
        assertTrue(t.indexOf(NotesView.QUICK_NOTE) < t.indexOf("Plumber Tuesday"))
        assertEquals("kept from before", app.documents.load(DocumentStore.SCRATCHPAD))
        // Not a dictation screen: the bubble stays off our own list screen.
        assertTrue(app.ownScreens.front.value.hidesBubble)
        assertFalse(app.ownScreens.front.value.dictationScreen)
    }

    @Test fun tappingANoteOpensItInTheEditor() {
        seed()
        val a = launch().await("Groceries")
        rowTitled(a, "Groceries").performClick()
        val i: Intent = shadowOf(a).nextStartedActivity
        assertEquals(NoteEditorActivity::class.java.name, i.component?.className)
        assertEquals("b2", i.getStringExtra(NoteEditorActivity.EXTRA_DOC))
    }

    @Test fun newNoteOpensAFreshEditor() {
        val a = launch().await("No notes yet")
        // The FAB: a 56 dp ink circle labelled for TalkBack.
        val fab = walk(a.window.decorView).first { it.contentDescription == NotesActivity.NEW_NOTE }
        assertTrue(fab is android.widget.ImageButton && fab.minimumHeight >= (56 * a.resources.displayMetrics.density).toInt())
        fab.performClick()
        val i: Intent = shadowOf(a).nextStartedActivity
        assertEquals(NoteEditorActivity::class.java.name, i.component?.className)
        assertEquals(NoteEditorActivity.ACTION_NEW_NOTE, i.action)
    }

    @Test fun searchFiltersByContent() {
        seed()
        val a = launch().await("Groceries")
        walk(a.window.decorView).filterIsInstance<EditText>().single().setText("EGGS")
        idle()
        assertTrue("Groceries" in texts(a))
        assertFalse("Plumber Tuesday" in texts(a))
        walk(a.window.decorView).filterIsInstance<EditText>().single().setText("nothing like this")
        idle()
        assertTrue("No matching notes" in texts(a))
    }

    private fun optionsFor(a: NotesActivity, title: String) {
        walk(a.window.decorView).first { it.contentDescription?.toString() == "Options for $title" }.performClick()
        idle()
    }

    @Test fun pinMovesANoteToTheTop() {
        app.documents.save("a1", "Older note")
        Thread.sleep(5) // distinct updated times
        app.documents.save("b2", "Newer note")
        val c = launch()
        val a = c.await("Older note")
        assertTrue(texts(a).indexOf("Newer note") < texts(a).indexOf("Older note"))
        optionsFor(a, "Older note")
        shadowOf(ShadowAlertDialog.getLatestAlertDialog()).clickOnItem(0)
        c.await(NotesActivity.PINNED)
        assertTrue(app.documents.list().first { it.id == "a1" }.pinned)
        assertTrue(texts(a).indexOf("Older note") < texts(a).indexOf("Newer note"))
    }

    @Test fun shareSendsTheNoteText() {
        seed()
        val a = launch().await("Groceries")
        optionsFor(a, "Groceries")
        shadowOf(ShadowAlertDialog.getLatestAlertDialog()).clickOnItem(1)
        val chooser: Intent = shadowOf(a).nextStartedActivity
        @Suppress("DEPRECATION") val send = chooser.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)!!
        assertEquals("Groceries\nmilk, eggs", send.getStringExtra(Intent.EXTRA_TEXT))
    }

    @Test fun deleteAsksThenRemovesTheFile() {
        seed()
        val a = launch().await("Groceries")
        optionsFor(a, "Groceries")
        shadowOf(ShadowAlertDialog.getLatestAlertDialog()).clickOnItem(2)
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_NEGATIVE).performClick()
        idle()
        assertTrue(File(dir, "b2.txt").isFile)
        optionsFor(a, "Groceries")
        shadowOf(ShadowAlertDialog.getLatestAlertDialog()).clickOnItem(2)
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick()
        awaitGone(a, "Groceries")
        assertFalse(File(dir, "b2.txt").exists())
        assertTrue("Plumber Tuesday" in texts(a))
    }

    @Test fun cardReadsAsOneStopWithItsActions() {
        seed()
        val a = launch().await("Groceries")
        val row = rowTitled(a, "Groceries")
        assertTrue(row.contentDescription.toString(), row.contentDescription.startsWith("Groceries, milk, eggs, "))
        val labels = row.createAccessibilityNodeInfo().actionList.mapNotNull { it.label?.toString() }
        assertTrue(labels.toString(), labels.containsAll(listOf("Options", "Pin", "Share", "Delete")))
        assertTrue(NotesActivity.TITLE in texts(a))
        walk(a.window.decorView).first { it.contentDescription == "Back" }.performClick()
        assertTrue(a.isFinishing)
    }

    @Test fun touchTargetsAreAtLeast48dp() {
        seed()
        val c = Robolectric.buildActivity(NotesActivity::class.java).setup().visible()
        c.await("Groceries")
        idle()
        UiTestKit.assertTouchTargets(c.get(), "notes")
    }
}
