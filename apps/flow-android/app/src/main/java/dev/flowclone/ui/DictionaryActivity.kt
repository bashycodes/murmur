package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.ViewGroup
import android.widget.ImageButton
import android.widget.TextView
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryDraft
import dev.flowclone.data.DictionaryEdit
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind

/**
 * Add / edit one dictionary word or snippet, opened from the Dictionary and Snippets tabs
 * ([dev.flowclone.ui.tabs.DictionaryTab]): a top bar with Back and, trailing, Save (enabled when
 * changed + valid, [DictionaryDraft]) and Delete (existing entries; asks first); word / trigger,
 * replacement / expansion and the snippet switch, with inline errors. Back on a changed form asks
 * to discard.
 *
 * [PAGE_IMPORT] / [PAGE_EXPORT]: no page of their own. They run the system file picker for the
 * shared JSON `[{word, replacement, isSnippet}]` (the file the Windows clone reads), report in a
 * toast and close, so the shell's tabs need no activity-result plumbing.
 */
class DictionaryActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi
    private var page = PAGE_EDIT
    private var kind = Kind.WORDS

    private var existing: DictEntry? = null
    private var draft: DictionaryDraft? = null
    private var titleView: TextView? = null
    private var wordRow: DevUi.FieldRow? = null
    private var replRow: DevUi.FieldRow? = null
    private var snippetToggle: DevUi.ToggleRow? = null
    /** Top-bar Save (tests). */
    internal var saveButton: ImageButton? = null; private set
    private var backCallback: OnBackInvokedCallback? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        page = intent.getStringExtra(EXTRA_PAGE) ?: PAGE_EDIT
        kind = intent.getStringExtra(EXTRA_KIND)?.let { k -> Kind.entries.firstOrNull { it.name == k } } ?: Kind.WORDS
        when (page) {
            PAGE_IMPORT, PAGE_EXPORT -> {
                window.decorView.setBackgroundColor(ui.t.bg)
                if (savedInstanceState == null) { if (page == PAGE_IMPORT) pick() else export() }
            }
            else -> { page = PAGE_EDIT; if (!renderEdit()) finish() }
        }
    }

    override fun onDestroy() {
        backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
        super.onDestroy()
    }

    // =====================================================================================
    // Edit
    // =====================================================================================

    /** False when the entry to edit no longer exists. */
    private fun renderEdit(): Boolean = with(ui) {
        val word = intent.getStringExtra(EXTRA_WORD)
        existing = word?.let { w -> app.dictionary.all().firstOrNull { it.word.equals(w, ignoreCase = true) } }
        if (word != null && existing == null) return false
        val ex = existing
        val d = ex?.let(DictionaryDraft::of) ?: DictionaryDraft.new(kind)
        draft = d
        title = DictionaryPages.editTitle(ex == null, d.snippet)
        screen(t.bgSettings) {
            val actions = ArrayList<DevUi.TopAction>()
            actions += DevUi.TopAction(R.drawable.ic_lw_check, saveLabel(d.snippet)) { onSave() }
            if (ex != null) actions += DevUi.TopAction(R.drawable.ic_lw_delete, deleteLabel(ex.isSnippet)) { confirmDelete(ex) }
            val bar = topBar(title.toString(), DevUi.TopNav.BACK, { confirmLeave() }, actions)
            titleView = bar.title
            saveButton = bar.actions.first()
            card {
                wordRow = fieldRow(
                    if (d.snippet) "Trigger phrase" else "Word or name", d.word,
                    InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
                    help = "Up to ${DictionaryEdit.MAX_WORD} characters", hint = if (d.snippet) "my address" else "Tailscale",
                ) { validate() }
                replRow = fieldRow(
                    replLabel(d.snippet), d.replacement,
                    InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                    help = replHelp(d.snippet), minLines = if (d.snippet) 3 else 1,
                ) { validate() }
                snippetToggle = toggleRow("Snippet", "Saying the trigger on its own inserts the text", d.snippet).also {
                    it.onChange { on -> relabel(on); validate() }
                }
            }
        }
        backCallback = OnBackInvokedCallback { confirmLeave() }.also {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, it)
        }
        validate()
        true
    }

    private fun saveLabel(snippet: Boolean) = "Save " + if (snippet) "snippet" else "word"
    private fun deleteLabel(snippet: Boolean) = "Delete " + if (snippet) "snippet" else "word"
    private fun replLabel(snippet: Boolean) = if (snippet) "Expands to" else "Replace with (optional)"
    private fun replHelp(snippet: Boolean) =
        if (snippet) "Up to ${DictionaryEdit.MAX_EXPANSION} characters; line breaks are kept"
        else "Leave empty to only teach the spelling; fill in to replace what the engine hears"

    private fun DevUi.FieldRow.label(): TextView = (edit.parent as ViewGroup).getChildAt(0) as TextView

    private fun relabel(snippet: Boolean) {
        wordRow?.label()?.text = if (snippet) "Trigger phrase" else "Word or name"
        replRow?.let { it.label().text = replLabel(snippet); it.help.text = replHelp(snippet) }
        title = DictionaryPages.editTitle(existing == null, snippet)
        titleView?.text = title
        saveButton?.contentDescription = saveLabel(snippet)
    }

    private fun read(): DictionaryDraft? = draft?.copy(
        word = wordRow?.text ?: "", replacement = replRow?.text ?: "", snippet = snippetToggle?.isChecked ?: false,
    )

    private fun validate() {
        val d = read() ?: return
        draft = d
        val all = app.dictionary.all()
        val e = d.visibleErrors(all)
        wordRow?.setError(e[DictionaryDraft.Field.WORD])
        replRow?.setError(e[DictionaryDraft.Field.REPLACEMENT])
        val can = d.canSave(all)
        saveButton?.let { it.isEnabled = can; it.alpha = if (can) 1f else 0.38f }
    }

    private fun onSave() {
        validate()
        val d = draft ?: return
        val all = app.dictionary.all()
        if (!d.canSave(all)) return
        app.dictionary.replaceAll(d.save(all))
        Toast.makeText(this, "Saved", Toast.LENGTH_SHORT).show()
        draft = null
        finish()
    }

    private fun confirmDelete(e: DictEntry) {
        ui.confirm("Delete “${e.word}”?", if (e.isSnippet) "The snippet and its text are removed." else "The word is removed from the dictionary.", "Delete", onYes = {
            app.dictionary.remove(e.word)
            Toast.makeText(this, "Deleted", Toast.LENGTH_SHORT).show()
            draft = null
            finish()
        })
    }

    private fun confirmLeave() {
        if (draft?.dirty != true) { finish(); return }
        AlertDialog.Builder(this)
            .setTitle("Discard changes?")
            .setMessage("Your changes on this page are not saved.")
            .setNegativeButton("Keep editing", null)
            .setPositiveButton("Discard") { _, _ -> finish() }
            .show()
    }

    // =====================================================================================
    // Import / export (SAF)
    // =====================================================================================

    @Suppress("DEPRECATION")
    private fun pick() = startActivityForResult(
        Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*"), REQ_IMPORT,
    )

    @Suppress("DEPRECATION")
    private fun export() = startActivityForResult(
        Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
            .putExtra(Intent.EXTRA_TITLE, "flow-dictionary.json"),
        REQ_EXPORT,
    )

    @Deprecated("platform Activity result API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data
        val msg = if (resultCode != RESULT_OK || uri == null) null else runCatching {
            when (requestCode) {
                REQ_IMPORT -> {
                    val json = contentResolver.openInputStream(uri)!!.use { it.readBytes().toString(Charsets.UTF_8) }
                    "Imported ${app.dictionary.importJson(json)} entries"
                }
                REQ_EXPORT -> {
                    contentResolver.openOutputStream(uri, "wt")!!.use { it.write(app.dictionary.exportJson().toByteArray()) }
                    "Exported ${app.dictionary.all().size} entries"
                }
                else -> null
            }
        }.getOrElse { "Failed: ${it.message}" }
        msg?.let { Toast.makeText(applicationContext, it, Toast.LENGTH_LONG).show() }
        if (page != PAGE_EDIT) finish()
    }

    companion object {
        private const val REQ_IMPORT = 20
        private const val REQ_EXPORT = 21
        const val EXTRA_PAGE = "page"
        const val EXTRA_KIND = "kind"
        const val EXTRA_WORD = "word"
        const val PAGE_EDIT = "edit"
        const val PAGE_IMPORT = "import"
        const val PAGE_EXPORT = "export"

        fun intent(c: Context, page: String = PAGE_EDIT, kind: Kind = Kind.WORDS): Intent =
            Intent(c, DictionaryActivity::class.java).putExtra(EXTRA_PAGE, page).putExtra(EXTRA_KIND, kind.name)

        /** [word] null = add a new entry of [kind]. */
        fun editIntent(c: Context, kind: Kind, word: String?): Intent = intent(c, PAGE_EDIT, kind).apply { if (word != null) putExtra(EXTRA_WORD, word) }

        fun edit(c: Context, kind: Kind, word: String?) = c.startActivity(editIntent(c, kind, word))

        /** Import ([import] true) or export the dictionary file through the system picker. */
        fun transfer(c: Context, import: Boolean) = c.startActivity(intent(c, if (import) PAGE_IMPORT else PAGE_EXPORT))
    }
}
