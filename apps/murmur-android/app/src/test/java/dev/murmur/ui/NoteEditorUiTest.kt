package dev.murmur.ui

import android.content.ClipboardManager
import android.content.Intent
import android.widget.EditText
import android.widget.TextView
import dev.murmur.MurmurApp
import dev.murmur.notes.DocumentStore
import dev.murmur.ui.UiTestKit.idle
import dev.murmur.ui.UiTestKit.texts
import dev.murmur.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.io.File

/** Note editor (Quick note + notes): restore on open, save on pause, Copy / Share / Clear, word count, dictation screen, note ids. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = MurmurApp::class)
class NoteEditorUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = MurmurApp.of(ctx)
    private val file get() = File(ctx.filesDir, "notes/scratchpad.txt")

    @After fun cleanup() { File(ctx.filesDir, "notes").deleteRecursively() }

    private fun launch() = Robolectric.buildActivity(NoteEditorActivity::class.java).setup()

    private fun editor(a: NoteEditorActivity) = walk(a.window.decorView).filterIsInstance<EditText>().single()

    private fun button(a: NoteEditorActivity, text: String) =
        walk(a.window.decorView).first { it is TextView && it.text.toString() == text }

    /** Saves run on an IO thread: wait for the file to say [want]. */
    private fun awaitSaved(want: String) {
        repeat(300) { idle(); if (app.documents.load(DocumentStore.SCRATCHPAD) == want) return; Thread.sleep(10) }
        assertEquals(want, app.documents.load(DocumentStore.SCRATCHPAD))
    }

    @Test fun textTypedIsSavedOnPauseAndRestoredOnNextOpen() {
        val c = launch()
        editor(c.get()).setText("Call the plumber tomorrow")
        c.pause().stop().destroy()
        awaitSaved("Call the plumber tomorrow")
        // Process killed and reopened: a fresh activity reads it back from files/notes/.
        val a = launch().get()
        assertEquals("Call the plumber tomorrow", editor(a).text.toString())
        assertTrue(texts(a).toString(), "4 words" in texts(a))
    }

    @Test fun wordCountFollowsTheText() {
        val a = launch().get()
        assertTrue("0 words" in texts(a))
        editor(a).setText("one two three")
        assertTrue(texts(a).toString(), "3 words" in texts(a))
    }

    @Test fun isTheDictationScreenOnlyWhileInFront() {
        val c = launch()
        val front = app.ownScreens.front
        assertTrue(front.value.dictationScreen)
        assertFalse(front.value.hidesBubble) // the bubble stays over the note editor
        c.pause().stop()
        assertFalse(front.value.dictationScreen)
        c.restart().resume()
        assertTrue(front.value.dictationScreen)
    }

    @Test fun copyPutsTheWholeTextOnTheClipboard() {
        val a = launch().get()
        editor(a).setText("line one\nline two")
        button(a, "Copy all").performClick()
        idle()
        val cm = ctx.getSystemService(ClipboardManager::class.java)
        assertEquals("line one\nline two", cm.primaryClip?.getItemAt(0)?.text?.toString())
    }

    @Test fun shareSendsThePlainText() {
        val a = launch().get()
        editor(a).setText("share me")
        button(a, "Share").performClick()
        val chooser: Intent = shadowOf(a).nextStartedActivity
        assertEquals(Intent.ACTION_CHOOSER, chooser.action)
        @Suppress("DEPRECATION") val send = chooser.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)!!
        assertEquals(Intent.ACTION_SEND, send.action)
        assertEquals("text/plain", send.type)
        assertEquals("share me", send.getStringExtra(Intent.EXTRA_TEXT))
    }

    @Test fun clearAsksFirstThenEmptiesAndSaves() {
        val a = launch().get()
        editor(a).setText("keep unless confirmed")
        awaitSaved("keep unless confirmed") // the debounced autosave, without a pause
        button(a, "Clear").performClick()
        val d = ShadowAlertDialog.getLatestAlertDialog()
        d.getButton(android.content.DialogInterface.BUTTON_NEGATIVE).performClick()
        idle()
        assertEquals("keep unless confirmed", editor(a).text.toString())
        button(a, "Clear").performClick()
        ShadowAlertDialog.getLatestAlertDialog().getButton(android.content.DialogInterface.BUTTON_POSITIVE).performClick()
        idle()
        assertEquals("", editor(a).text.toString())
        awaitSaved("")
        assertTrue(file.isFile)
    }

    private fun awaitNote(id: String, want: String?) {
        repeat(300) { idle(); if (app.documents.load(id) == want) return; Thread.sleep(10) }
        assertEquals(want, app.documents.load(id))
    }

    @Test fun aNoteIdOpensAndSavesThatNoteNotTheScratchpad() {
        app.documents.save("n-shop", "milk")
        val c = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.note(ctx, "n-shop")).setup()
        assertEquals("milk", editor(c.get()).text.toString())
        assertEquals(NoteEditorActivity.NOTE_TITLE, c.get().title.toString())
        assertTrue(app.ownScreens.front.value.dictationScreen) // the bubble works in every note editor
        editor(c.get()).setText("milk and eggs")
        c.pause().stop().destroy()
        awaitNote("n-shop", "milk and eggs")
        assertNull(app.documents.load(DocumentStore.SCRATCHPAD))
    }

    @Test fun unsafeNoteIdFallsBackToTheScratchpad() {
        val a = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.note(ctx, "../../x")).setup().get()
        assertEquals(DocumentStore.SCRATCHPAD, a.noteId)
        assertEquals(NoteEditorActivity.TITLE, a.title.toString())
    }

    @Test fun newNoteGetsAFreshIdAndIsOnlyWrittenOnceItHasText() {
        val c = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.newNote(ctx)).setup()
        val id = c.get().noteId
        assertTrue(id, DocumentStore.isValidId(id) && id != DocumentStore.SCRATCHPAD)
        assertTrue(editor(c.get()).hasFocus())
        c.pause()
        idle()
        assertTrue("untouched new note leaves nothing", app.documents.list().isEmpty())
        c.resume()
        editor(c.get()).setText("dictated idea")
        c.pause().stop().destroy()
        awaitNote(id, "dictated idea")
        assertEquals(listOf(id), app.documents.list().map { it.id })
    }

    @Test fun newNoteKeepsItsIdAcrossRecreation() {
        val c = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.newNote(ctx)).setup()
        val id = c.get().noteId
        editor(c.get()).setText("half a thought")
        val state = android.os.Bundle()
        c.pause().saveInstanceState(state).stop().destroy()
        awaitNote(id, "half a thought")
        val again = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.newNote(ctx)).setup(state).get()
        assertEquals(id, again.noteId)
        assertEquals("half a thought", editor(again).text.toString())
    }

    @Test fun newNoteIntentWhileInFrontSavesAndSwitches() {
        val c = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.note(ctx, "first")).setup()
        editor(c.get()).setText("first text")
        c.newIntent(NoteEditorActivity.newNote(ctx))
        awaitNote("first", "first text")
        val second = c.get().noteId
        assertTrue(second != "first")
        assertEquals("", editor(c.get()).text.toString())
        editor(c.get()).setText("second text")
        c.pause()
        awaitNote(second, "second text")
        assertEquals("first text", app.documents.load("first"))
    }

    @Test fun topBarNamesTheNoteAndBackFinishes() {
        val c = Robolectric.buildActivity(NoteEditorActivity::class.java, NoteEditorActivity.note(ctx, "first")).setup()
        assertTrue(texts(c.get()).toString(), NoteEditorActivity.NOTE_TITLE in texts(c.get()))
        // Switching to the Quick note in place renames the bar too.
        c.newIntent(NoteEditorActivity.scratchpad(ctx))
        assertTrue(texts(c.get()).toString(), NoteEditorActivity.TITLE in texts(c.get()))
        walk(c.get().window.decorView).first { it.contentDescription == "Back" }.performClick()
        assertTrue(c.get().isFinishing)
    }

    @Test fun touchTargetsAreAtLeast48dp() {
        val a = launch().visible().get()
        idle()
        UiTestKit.assertTouchTargets(a, "scratchpad")
    }
}
