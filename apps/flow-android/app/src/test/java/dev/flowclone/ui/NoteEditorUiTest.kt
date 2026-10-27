package dev.flowclone.ui

import android.content.ClipboardManager
import android.content.Intent
import android.widget.EditText
import android.widget.TextView
import dev.flowclone.FlowApp
import dev.flowclone.notes.DocumentStore
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
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

/** Scratchpad: restore on open, save on pause, Copy / Share / Clear, word count, dictation surface. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class NoteEditorUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)
    private val file get() = File(ctx.filesDir, "notes/scratchpad.txt")

    @After fun cleanup() { File(ctx.filesDir, "notes").deleteRecursively(); app.dictationSurface = null }

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

    @Test fun isTheDictationSurfaceOnlyWhileInFront() {
        val c = launch()
        assertSame(c.get(), app.dictationSurface)
        c.pause()
        assertNull(app.dictationSurface)
        c.resume()
        assertSame(c.get(), app.dictationSurface)
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

    @Test fun touchTargetsAreAtLeast48dp() {
        val a = launch().visible().get()
        idle()
        UiTestKit.assertTouchTargets(a, "scratchpad")
    }
}
