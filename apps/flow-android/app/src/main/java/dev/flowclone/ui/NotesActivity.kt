package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.os.Bundle
import android.text.format.DateFormat
import android.text.format.DateUtils
import android.view.MenuItem
import android.widget.LinearLayout
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.notes.DocumentStore
import dev.flowclone.notes.Note
import dev.flowclone.notes.NotesView
import java.util.Date

/**
 * Notes (main screen → *Notes*): every saved note, pinned first then most recently changed, with
 * its first line as the title and when it was last changed. *New note*, search by content, and
 * per note (the ⋮ button, long-press, or TalkBack actions) Pin / Unpin, Share and Delete (asks
 * first). Tapping a note opens it in [NoteEditorActivity]. The Quick note (the Scratchpad from
 * before Notes) is one of them, pinned by default. Reloaded on every resume, so an edit shows
 * when you come back. All store calls run on [FlowApp.documentsIo], in order with the editor's saves.
 *
 * Not a dictation surface: like every other screen of ours the bubble stays hidden here (the
 * search box included); it appears in the editor.
 */
class NotesActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi
    private var notes: List<Note> = emptyList()
    private var query = ""
    private var listBox: LinearLayout? = null
    private var built = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        actionBar?.setDisplayHomeAsUpEnabled(true)
        query = savedInstanceState?.getString(STATE_QUERY).orEmpty()
    }

    override fun onSaveInstanceState(out: Bundle) {
        super.onSaveInstanceState(out)
        out.putString(STATE_QUERY, query)
    }

    override fun onResume() {
        super.onResume()
        load()
    }

    private fun io(work: () -> Unit) { app.documentsIo.execute { runCatching(work) } }

    private fun load() = io {
        val n = app.documents.list()
        runOnUiThread {
            if (isFinishing || isDestroyed) return@runOnUiThread
            notes = n
            if (!built) render() else fill()
        }
    }

    private fun render(): Unit = with(ui) {
        built = true
        rebuild {
            action("New note", DevUi.Style.PRIMARY) { startActivity(NoteEditorActivity.newNote(this@NotesActivity)) }
            searchField("Search notes", query) { q -> query = q; fill() }
            listBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
        }
        fill()
    }

    private fun fill(): Unit = with(ui) {
        val box = listBox ?: return@with
        box.removeAllViews()
        val shown = NotesView.filter(notes, query)
        if (shown.isEmpty()) {
            box.card {
                if (notes.isEmpty()) emptyState("No notes yet", "Tap New note, then the bubble, and talk.")
                else emptyState("No matching notes", "Nothing contains \"${query.trim()}\".")
            }
            return@with
        }
        val (pinned, rest) = shown.partition { it.pinned }
        for ((label, group) in listOf("PINNED" to pinned, "NOTES" to rest)) {
            if (group.isEmpty()) continue
            box.sectionLabel(label)
            box.card {
                for (n in group) {
                    val menu = { options(n) }
                    listRow(
                        NotesView.title(n), meta(n), if (n.pinned) "Pinned" else "",
                        trailing = iconButton(R.drawable.ic_dev_down, "Options for ${NotesView.title(n)}") { menu() },
                        actions = listOf(
                            "Options" to menu,
                            (if (n.pinned) "Unpin" else "Pin") to { pin(n, !n.pinned) },
                            "Share" to { share(n) },
                            "Delete" to { delete(n) },
                        ),
                    ) { startActivity(NoteEditorActivity.note(this@NotesActivity, n.id)) }
                }
            }
        }
    }

    /** "3:41 PM · first words of the rest" (older than today: the date instead of the time). */
    private fun meta(n: Note): String {
        val d = Date(n.updatedAt)
        val whenText = if (DateUtils.isToday(n.updatedAt)) DateFormat.getTimeFormat(this).format(d) else DateFormat.getMediumDateFormat(this).format(d)
        return listOf(whenText, NotesView.preview(n)).filter { it.isNotEmpty() }.joinToString(" · ")
    }

    private fun options(n: Note) {
        val items = arrayOf(if (n.pinned) "Unpin" else "Pin", "Share", "Delete")
        AlertDialog.Builder(this).setTitle(NotesView.title(n)).setItems(items) { _, which ->
            when (which) { 0 -> pin(n, !n.pinned); 1 -> share(n); 2 -> delete(n) }
        }.show()
    }

    private fun pin(n: Note, on: Boolean) = io {
        app.documents.setPinned(n.id, on)
        runOnUiThread { ui.announce(if (on) "Pinned" else "Unpinned") }
        load()
    }

    private fun share(n: Note) {
        if (n.text.isBlank()) { Toast.makeText(this, "Nothing to share yet", Toast.LENGTH_SHORT).show(); return }
        NoteEditorActivity.shareText(this, n.text, NotesView.title(n))
    }

    private fun delete(n: Note) {
        val what = if (n.id == DocumentStore.SCRATCHPAD) "the Quick note" else "this note"
        ui.confirm("Delete $what?", "Its text is deleted from this phone.", "Delete", onYes = {
            io {
                app.documents.delete(n.id)
                runOnUiThread { ui.announce("Deleted") }
                load()
            }
        })
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean =
        if (item.itemId == android.R.id.home) { finish(); true } else super.onOptionsItemSelected(item)

    private companion object {
        const val STATE_QUERY = "query"
    }
}
