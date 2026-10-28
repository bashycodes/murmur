package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowInsets
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.notes.Autosaver
import dev.flowclone.notes.DocumentStore
import dev.flowclone.notes.NotesView
import dev.flowclone.notes.TextStats
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.SupervisorJob

/**
 * One plain-text note to dictate into, edit and copy out: the Quick note
 * ([DocumentStore.SCRATCHPAD], main screen → *Quick note*, or its launcher shortcut) or any note
 * from [NotesActivity] ([EXTRA_DOC]). [ACTION_NEW_NOTE] (*New note* in Notes and the launcher
 * shortcut) opens a fresh id from [dev.flowclone.notes.NotesStore.newId]; nothing is written until
 * there is text, so backing out of an untouched new note leaves no empty note behind. The id is
 * kept across recreation (rotation, process death).
 *
 * Dictation: the field is an ordinary EditText, so the bubble and the accessibility service type
 * into it like into any other app. While this screen is in front it registers itself as
 * [FlowApp.dictationSurface], the one exception to "no bubble on our own screens"
 * ([dev.flowclone.policy.OwnAppGate]). Nothing here inserts dictated text itself, so a
 * dictation goes in once, through the same commitText path as everywhere else. A new note does
 * not start recording by itself: the mic session belongs to the bubble's service and starts from
 * a bubble tap (starting it from here would need a new service entry point and a mic start
 * without the user's tap); the field is focused with the keyboard up, so the bubble is one tap away.
 *
 * Saving: [Autosaver] writes [DocumentStore] (per note) shortly after edits pause and at once on pause.
 */
class NoteEditorActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var docId: String
    private lateinit var edit: EditText
    private lateinit var count: TextView
    private lateinit var saver: Autosaver
    /** Swapping in another note's text ([onNewIntent]): not an edit. */
    private var loading = false

    /** On [FlowApp.documentsIo]; not cancelled on destroy: the last flush must still reach the disk. */
    private val io by lazy { CoroutineScope(SupervisorJob() + app.documentsIo.asCoroutineDispatcher()) }

    /** The note open right now (tests). */
    val noteId: String get() = docId

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        actionBar?.setDisplayHomeAsUpEnabled(true)
        val text = open(intent, savedInstanceState?.getString(STATE_DOC)) ?: return
        build(text)
    }

    /**
     * Picks the note [i] asks for ([restored]: the id this screen had before a recreation) and
     * reads it. Null (after finishing) when the file could not be read.
     */
    private fun open(i: Intent, restored: String?): String? {
        val store = app.documents
        docId = restored?.takeIf(DocumentStore::isValidId)
            ?: i.getStringExtra(EXTRA_DOC)?.takeIf(DocumentStore::isValidId)
            ?: (if (i.action == ACTION_NEW_NOTE) runCatching { app.documentsIo.submit<String> { store.newId() }.get() }.getOrNull() else null)
            ?: DocumentStore.SCRATCHPAD
        title = i.getStringExtra(EXTRA_TITLE) ?: if (docId == DocumentStore.SCRATCHPAD) TITLE else NOTE_TITLE
        // A small private file, read before the field exists so nothing typed can race a late
        // load; on the documents thread, after any save another editor's pause queued.
        val id = docId
        val loaded = runCatching { app.documentsIo.submit<String?> { store.load(id) }.get() }
        if (loaded.isFailure) {
            // Never start empty over a file we could not read: the first autosave would replace it.
            Toast.makeText(this, "Couldn't open the ${title.toString().lowercase()}", Toast.LENGTH_LONG).show()
            finish()
            return null
        }
        val text = loaded.getOrNull().orEmpty()
        saver = Autosaver(io, text) { store.save(id, it) }
        return text
    }

    /** singleTop: another note asked for while this one is in front (e.g. *New note* again). */
    override fun onNewIntent(i: Intent) {
        super.onNewIntent(i)
        if (!::edit.isInitialized) return
        val same = i.action != ACTION_NEW_NOTE && (i.getStringExtra(EXTRA_DOC) ?: DocumentStore.SCRATCHPAD) == docId
        if (same) return
        saver.flush()
        setIntent(i)
        val text = open(i, null) ?: return
        loading = true
        edit.setText(text)
        edit.setSelection(text.length)
        loading = false
        showCount()
    }

    override fun onSaveInstanceState(out: Bundle) {
        super.onSaveInstanceState(out)
        if (::docId.isInitialized) out.putString(STATE_DOC, docId)
    }

    private fun build(initial: String) {
        val ui = DevUi(this)
        val p = ui.p
        fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()
        count = TextView(this).apply {
            textSize = 13f; setTextColor(p.muted)
            setPadding(dp(16), dp(4), dp(16), dp(4))
        }
        edit = EditText(this).apply {
            id = R.id.note_editor
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
            isSingleLine = false
            gravity = Gravity.TOP or Gravity.START
            hint = "Tap here, then the bubble, and talk. Saved as you go."
            setHintTextColor((p.muted and 0x00FFFFFF) or 0x99000000.toInt())
            textSize = 17f; setTextColor(p.text)
            background = GradientDrawable().apply { setColor(p.surface); cornerRadius = dp(20).toFloat() }
            setPadding(dp(16), dp(14), dp(16), dp(14))
            setText(initial)
            setSelection(initial.length)
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(p.bg)
            with(ui) {
                chipRow(
                    chip("Copy all") { copyAll() },
                    chip("Share") { share() },
                    chip("Clear") { clear() },
                )
            }
            addView(count)
            addView(edit, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f).apply {
                marginStart = dp(16); marginEnd = dp(16); bottomMargin = dp(8)
            })
            setOnApplyWindowInsetsListener { v, insets ->
                val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                v.setPadding(i.left, i.top + dp(4), i.right, i.bottom + dp(8))
                insets
            }
        }
        window.decorView.setBackgroundColor(p.bg)
        setContentView(root)
        root.requestApplyInsets()
        showCount()
        edit.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
            override fun afterTextChanged(s: Editable?) {
                showCount()
                if (!loading) saver.edited(s?.toString().orEmpty())
            }
        })
        edit.requestFocus()
    }

    private fun current() = edit.text.toString()

    private fun showCount() { count.text = TextStats.label(current()) }

    override fun onResume() {
        super.onResume()
        app.dictationSurface = this
    }

    override fun onPause() {
        if (app.dictationSurface === this) app.dictationSurface = null
        if (::saver.isInitialized) saver.flush()
        super.onPause()
    }

    /** S16 like every transcript the app copies: removed from the clipboard after a minute. */
    private fun copyAll() {
        val t = current()
        if (t.isBlank()) { Toast.makeText(this, "Nothing to copy yet", Toast.LENGTH_SHORT).show(); return }
        app.clipboard.copy(t)
        Toast.makeText(this, "Copied (cleared after 1 minute)", Toast.LENGTH_SHORT).show()
        window.decorView.announceForAccessibility("Copied")
    }

    private fun share() {
        val t = current()
        if (t.isBlank()) { Toast.makeText(this, "Nothing to share yet", Toast.LENGTH_SHORT).show(); return }
        shareText(this, t, title.toString())
    }

    private fun clear() {
        if (current().isEmpty()) return
        DevUi(this).confirm("Clear the ${title.toString().lowercase()}?", "All of its text is deleted from this phone.", "Clear", onYes = {
            edit.setText("")
            saver.flush()
            window.decorView.announceForAccessibility("Cleared")
        })
    }

    override fun onNavigateUp(): Boolean { finish(); return true }

    companion object {
        /** [DocumentStore] id to open (default: the Quick note). */
        const val EXTRA_DOC = "doc"
        /** Screen title (default [TITLE] for the Quick note, [NOTE_TITLE] for other notes). */
        const val EXTRA_TITLE = "title"
        /** Open a fresh note (res/xml/shortcuts.xml names this action too). */
        const val ACTION_NEW_NOTE = "dev.flowclone.action.NEW_NOTE"
        const val TITLE = NotesView.QUICK_NOTE
        const val NOTE_TITLE = "Note"
        private const val STATE_DOC = "doc_id"

        fun scratchpad(c: Context): Intent = Intent(c, NoteEditorActivity::class.java)

        fun note(c: Context, id: String): Intent = Intent(c, NoteEditorActivity::class.java).putExtra(EXTRA_DOC, id)

        fun newNote(c: Context): Intent = Intent(c, NoteEditorActivity::class.java).setAction(ACTION_NEW_NOTE)

        /** Plain-text share sheet (editor and Notes list). */
        fun shareText(a: Activity, text: String, what: String) {
            val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
            runCatching { a.startActivity(Intent.createChooser(send, "Share $what")) }
                .onFailure { Toast.makeText(a, "No app to share with", Toast.LENGTH_SHORT).show() }
        }
    }
}
