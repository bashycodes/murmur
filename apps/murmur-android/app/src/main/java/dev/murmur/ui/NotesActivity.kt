package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import android.os.Bundle
import android.text.format.DateFormat
import android.text.format.DateUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.notes.DocumentStore
import dev.murmur.notes.Note
import dev.murmur.notes.NotesView
import java.util.Date

/**
 * Notes (main screen → *Notes*): the design's top bar with back, search, then every saved note
 * as a card (AndroidHome.dc.html's card look; the desktop Scratchpad's title + preview + when),
 * pinned first then most recently changed. The *New note* FAB (`.a-fab`), search by content, and
 * per note (the ⋮ button, long-press, or TalkBack actions) Pin / Unpin, Share and Delete (asks
 * first). Tapping a note opens it in [NoteEditorActivity]. The Quick note (the Scratchpad from
 * before Notes) is one of them, pinned by default. Reloaded on every resume, so an edit shows
 * when you come back. All store calls run on [MurmurApp.documentsIo], in order with the editor's saves.
 *
 * Not a dictation screen: like every other screen of ours the bubble stays hidden here (the
 * search box included); it appears in the editor.
 */
class NotesActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private lateinit var ui: DevUi
    private var notes: List<Note> = emptyList()
    private var query = ""
    private var listBox: LinearLayout? = null
    private var built = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
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
        fabPage(R.drawable.ic_lw_plus, NEW_NOTE, { startActivity(NoteEditorActivity.newNote(this@NotesActivity)) }, t.bg) {
            topBar(TITLE, DevUi.TopNav.BACK, { finish() })
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
                if (notes.isEmpty()) emptyState("No notes yet", "Tap + for a new note, then the bubble, and talk.")
                else emptyState("No matching notes", "Nothing contains \"${query.trim()}\".")
            }
            return@with
        }
        val (pinned, rest) = shown.partition { it.pinned }
        for ((label, group) in listOf(PINNED to pinned, RECENT to rest)) {
            if (group.isEmpty()) continue
            box.heading(label)
            for (n in group) box.addView(noteCard(n))
        }
    }

    /**
     * One note: its title (first line), the start of the rest, when it was last changed with a
     * Pinned badge, and ⋮ (Pin / Share / Delete). The card body opens the editor and is one
     * TalkBack stop with those as custom actions.
     */
    private fun noteCard(n: Note): View = with(ui) {
        val title = NotesView.title(n)
        val preview = NotesView.preview(n)
        val whenText = whenText(n)
        val menu = { options(n) }
        val card = itemCard().apply { setPadding(dp(20), dp(8), dp(8), dp(8)) }
        val body = LinearLayout(this@NotesActivity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(0, dp(10), dp(8), dp(10))
            addView(tv(title, TypeScale.ROW, p.text, TypeScale.W_SEMIBOLD).apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END })
            if (preview.isNotEmpty()) addView(tv(preview, TypeScale.SUB, p.muted).apply {
                maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END; setPadding(0, dp(2), 0, 0)
            })
            addView(LinearLayout(this@NotesActivity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                addView(tv(whenText, TypeScale.META, p.muted))
                if (n.pinned) addView(badge("Pinned"), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) })
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(8) })
            for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            contentDescription = listOf(title, preview, whenText, if (n.pinned) "Pinned" else "").filter { it.isNotEmpty() }.joinToString(", ")
        }
        val row = LinearLayout(this@NotesActivity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            addView(body, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(iconButton(R.drawable.ic_lw_more, "Options for $title") { menu() })
        }
        body.apply {
            minimumHeight = dp(48)
            background = ripple(null, 12)
            isClickable = true; isFocusable = true
            setOnClickListener { startActivity(NoteEditorActivity.note(this@NotesActivity, n.id)) }
            customActions(this, listOf(
                "Options" to menu,
                (if (n.pinned) "Unpin" else "Pin") to { pin(n, !n.pinned) },
                "Share" to { share(n) },
                "Delete" to { delete(n) },
            ))
        }
        card.addView(row)
        card
    }

    /** When it was last changed: "3:41 PM" today, the date before. */
    private fun whenText(n: Note): String {
        val d = Date(n.updatedAt)
        return if (DateUtils.isToday(n.updatedAt)) DateFormat.getTimeFormat(this).format(d) else DateFormat.getMediumDateFormat(this).format(d)
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

    companion object {
        const val TITLE = "Notes"
        /** The FAB's TalkBack label. */
        const val NEW_NOTE = "New note"
        const val PINNED = "Pinned"
        const val RECENT = "Recent"
        private const val STATE_QUERY = "query"
    }
}
