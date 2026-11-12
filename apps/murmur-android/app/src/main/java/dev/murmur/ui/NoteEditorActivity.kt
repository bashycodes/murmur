package dev.murmur.ui

import android.app.Activity
import dev.murmur.bubble.DictationSurface
import android.content.Context
import android.content.Intent
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
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.notes.Autosaver
import dev.murmur.notes.DocumentStore
import dev.murmur.notes.NotesView
import dev.murmur.notes.TextStats
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.SupervisorJob

/**
 * One plain-text note to dictate into, edit and copy out: the Quick note
 * ([DocumentStore.SCRATCHPAD], main screen → *Quick note*, or its launcher shortcut) or any note
 * from [NotesActivity] ([EXTRA_DOC]). [ACTION_NEW_NOTE] (*New note* in Notes and the launcher
 * shortcut) opens a fresh id from [dev.murmur.notes.NotesStore.newId]; nothing is written until
 * there is text, so backing out of an untouched new note leaves no empty note behind. The id is
 * kept across recreation (rotation, process death).
 *
 * Dictation: the field is an ordinary EditText, so the bubble and the accessibility service type
 * into it like into any other app. It is a [DictationSurface], the one exception to "no bubble
 * on our own screens" ([dev.murmur.bubble.OwnScreens]), and while in front it registers itself as
 * [MurmurApp.dictationSurface] so the accessibility service treats its field like any other app's
 * ([dev.murmur.policy.OwnAppGate]). Nothing here inserts dictated text itself, so a
 * dictation goes in once, through the same commitText path as everywhere else. A new note does
 * not start recording by itself: the mic session belongs to the bubble's service and starts from
 * a bubble tap (starting it from here would need a new service entry point and a mic start
 * without the user's tap); the field is focused with the keyboard up, so the bubble is one tap away.
 *
 * Saving: [Autosaver] writes [DocumentStore] (per note) shortly after edits pause and at once on pause.
 *
 * Look: the design's top bar with back and the note's name, Copy all / Share / Clear pills, the
 * word count, and the text on a white 24 dp card (no artboard of its own; Android*.dc.html parts).
 */
class NoteEditorActivity : Activity(), DictationSurface {
    private val app get() = MurmurApp.of(this)
    private lateinit var docId: String
    private lateinit var edit: EditText
    private lateinit var count: TextView
    private var bar: DevUi.TopBar? = null
    private lateinit var saver: Autosaver
    /** Swapping in another note's text ([onNewIntent]): not an edit. */
    private var loading = false

    /** On [MurmurApp.documentsIo]; not cancelled on destroy: the last flush must still reach the disk. */
    private val io by lazy { CoroutineScope(SupervisorJob() + app.documentsIo.asCoroutineDispatcher()) }

    /** The note open right now (tests). */
    val noteId: String get() = docId

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
        bar?.title?.text = title
        showCount()
    }

    override fun onSaveInstanceState(out: Bundle) {
        super.onSaveInstanceState(out)
        if (::docId.isInitialized) out.putString(STATE_DOC, docId)
    }

    private fun build(initial: String) {
        val ui = DevUi(this)
        val p = ui.p
        val bg = ui.t.bg
        fun dp(v: Int) = ui.dp(v)
        count = ui.tv("", TypeScale.META, p.muted).apply {
            setPadding(dp(20), 0, dp(20), dp(8))
        }
        edit = EditText(this).apply {
            id = R.id.note_editor
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES or InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
            isSingleLine = false
            gravity = Gravity.TOP or Gravity.START
            hint = "Tap here, then the bubble, and talk. Saved as you go."
            setHintTextColor(p.muted)
            textSize = TypeScale.ROW; setTextColor(p.text)
            typeface = Fonts.sans(this@NoteEditorActivity, TypeScale.W_REGULAR)
            setLineSpacing(0f, 1.15f)
            background = ui.rounded(p.surface, 24)
            setPadding(dp(20), dp(18), dp(20), dp(18))
            setText(initial)
            setSelection(initial.length)
        }
        val topBar = ui.buildTopBar(title.toString(), DevUi.TopNav.BACK, { finish() }).also { bar = it }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(bg)
            addView(topBar.view, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)))
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
                v.setPadding(i.left, i.top, i.right, i.bottom + dp(8))
                insets
            }
        }
        window.decorView.setBackgroundColor(bg)
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

    companion object {
        /** [DocumentStore] id to open (default: the Quick note). */
        const val EXTRA_DOC = "doc"
        /** Screen title (default [TITLE] for the Quick note, [NOTE_TITLE] for other notes). */
        const val EXTRA_TITLE = "title"
        /** Open a fresh note (res/xml/shortcuts.xml names this action too). */
        const val ACTION_NEW_NOTE = "dev.murmur.action.NEW_NOTE"
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
