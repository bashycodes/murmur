package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
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
import dev.flowclone.notes.TextStats
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.SupervisorJob

/**
 * One plain-text document to dictate into, edit and copy out. Today that is the Scratchpad
 * ([DocumentStore.SCRATCHPAD], main screen → *Scratchpad*, or the launcher shortcut); Notes will
 * open other ids through [EXTRA_DOC] / [EXTRA_TITLE].
 *
 * Dictation: the field is an ordinary EditText, so the bubble and the accessibility service type
 * into it like into any other app. While this screen is in front it registers itself as
 * [FlowApp.dictationSurface], the one exception to "no bubble on our own screens"
 * ([dev.flowclone.policy.OwnAppGate]). Nothing here inserts dictated text itself, so a
 * dictation goes in once, through the same commitText path as everywhere else.
 *
 * Saving: [Autosaver] writes [DocumentStore] shortly after edits pause and at once on pause.
 */
class NoteEditorActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var docId: String
    private lateinit var edit: EditText
    private lateinit var count: TextView
    private lateinit var saver: Autosaver

    /** On [FlowApp.documentsIo]; not cancelled on destroy: the last flush must still reach the disk. */
    private val io by lazy { CoroutineScope(SupervisorJob() + app.documentsIo.asCoroutineDispatcher()) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        docId = intent.getStringExtra(EXTRA_DOC)?.takeIf(DocumentStore::isValidId) ?: DocumentStore.SCRATCHPAD
        title = intent.getStringExtra(EXTRA_TITLE) ?: TITLE
        actionBar?.setDisplayHomeAsUpEnabled(true)
        val store = app.documents
        // A small private file, read before the field exists so nothing typed can race a late
        // load; on the documents thread, after any save another editor's pause queued.
        val id = docId
        val loaded = runCatching { app.documentsIo.submit<String?> { store.load(id) }.get() }
        if (loaded.isFailure) {
            // Never start empty over a file we could not read: the first autosave would replace it.
            Toast.makeText(this, "Couldn't open the ${title.toString().lowercase()}", Toast.LENGTH_LONG).show()
            finish()
            return
        }
        val text = loaded.getOrNull().orEmpty()
        saver = Autosaver(io, text) { store.save(docId, it) }
        build(text)
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
                saver.edited(s?.toString().orEmpty())
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
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, t)
        runCatching { startActivity(Intent.createChooser(send, "Share ${title}")) }
            .onFailure { Toast.makeText(this, "No app to share with", Toast.LENGTH_SHORT).show() }
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
        /** [DocumentStore] id to open (default: the Scratchpad). */
        const val EXTRA_DOC = "doc"
        /** Screen title (default "Scratchpad"). */
        const val EXTRA_TITLE = "title"
        const val TITLE = "Scratchpad"

        fun scratchpad(c: Context): Intent = Intent(c, NoteEditorActivity::class.java)
    }
}
