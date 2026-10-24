package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Menu
import android.view.MenuItem
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryDraft
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind

/**
 * Dictionary words (spelling / replacements the core applies and feeds to whisper's prompt) and
 * snippets (trigger phrase → expansion), in the Settings look, one activity instance per page:
 *   hub     Dictionary + Snippets rows with their counts; Import / Export JSON
 *   list    one kind, Add in the top bar, search once the list is long, tap = edit,
 *           TalkBack actions / long-press = delete (asks first); empty state
 *   edit    word / trigger, replacement / expansion, snippet switch; Save in the top bar when
 *           changed + valid ([DictionaryDraft], inline errors), Discard prompt, Delete
 * Import/export use the shared JSON `[{word, replacement, isSnippet}]` (same file the Windows
 * clone reads), via SAF. Same [dev.flowclone.data.DictionaryStore] calls as before.
 */
class DictionaryActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi
    private var page = PAGE_HUB
    private var kind = Kind.WORDS
    private val refreshers = ArrayList<() -> Unit>()

    // list page
    private var query = ""
    private var listCard: LinearLayout? = null

    // edit page
    private var draft: DictionaryDraft? = null
    private var wordRow: DevUi.FieldRow? = null
    private var replRow: DevUi.FieldRow? = null
    private var snippetToggle: DevUi.ToggleRow? = null
    private var saveItem: MenuItem? = null
    private var backCallback: OnBackInvokedCallback? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        page = intent.getStringExtra(EXTRA_PAGE) ?: PAGE_HUB
        kind = intent.getStringExtra(EXTRA_KIND)?.let { k -> Kind.entries.firstOrNull { it.name == k } } ?: Kind.WORDS
        if (page != PAGE_HUB) actionBar?.setDisplayHomeAsUpEnabled(true)
        when (page) {
            PAGE_LIST -> { title = DictionaryPages.title(kind); renderList() }
            PAGE_EDIT -> if (!renderEdit()) { finish(); return }
            else -> { page = PAGE_HUB; renderHub() }
        }
    }

    override fun onResume() {
        super.onResume()
        for (r in refreshers) r()
    }

    override fun onDestroy() {
        backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
        super.onDestroy()
    }

    // =====================================================================================
    // Hub
    // =====================================================================================

    private fun renderHub(): Unit = with(ui) {
        screen {
            note("Words: the speech engine is primed with them, and \"wisper\" → \"Wispr\" style fixes are applied. Snippets: say just the trigger and its text is inserted.")
            sectionLabel("LISTS")
            card {
                val words = navRow(R.drawable.ic_scr_book, "Dictionary") { open(this@DictionaryActivity, PAGE_LIST, Kind.WORDS) }
                val snippets = navRow(R.drawable.ic_scr_snippet, "Snippets") { open(this@DictionaryActivity, PAGE_LIST, Kind.SNIPPETS) }
                refreshers += {
                    val all = app.dictionary.all()
                    words.setSubtitle(DictionaryPages.hubSubtitle(all, Kind.WORDS))
                    snippets.setSubtitle(DictionaryPages.hubSubtitle(all, Kind.SNIPPETS))
                }
            }
            sectionLabel("IMPORT & EXPORT")
            card {
                navRow(R.drawable.ic_scr_import, "Import JSON", "Adds a file's entries; an entry with the same word is replaced") { pick() }
                navRow(R.drawable.ic_scr_share, "Export JSON", "Saves flow-dictionary.json (the Windows app reads the same file)") { export() }
            }
        }
    }

    // =====================================================================================
    // List (one kind)
    // =====================================================================================

    private fun renderList(): Unit = with(ui) {
        screen {
            note(
                if (kind == Kind.SNIPPETS) "Say the trigger phrase on its own and its text is inserted. Tap one to edit it."
                else "Names and jargon are spelled right; a replacement fixes a word the engine keeps getting wrong. Tap one to edit it.",
            )
            val all = app.dictionary.all()
            if (DictionaryPages.showSearch(DictionaryPages.of(all, kind).size)) {
                searchField(if (kind == Kind.SNIPPETS) "Search snippets" else "Search words", query) { q -> query = q; fillList() }
            }
            listCard = card { }
        }
        refreshers += { fillList() }
    }

    private fun fillList(): Unit = with(ui) {
        val card = listCard ?: return@with
        card.removeAllViews()
        val all = app.dictionary.all()
        val shown = DictionaryPages.entries(all, kind, query)
        if (shown.isEmpty()) {
            val e = DictionaryPages.empty(kind, query, DictionaryPages.of(all, kind).size)
            card.emptyState(e.title, e.hint)
        }
        for (e in shown) {
            card.listRow(
                e.word, DictionaryPages.rowSubtitle(e),
                actions = listOf("Delete" to { confirmDelete(e) { fillList() } }),
            ) { edit(this@DictionaryActivity, kind, e.word) }
        }
    }

    private fun confirmDelete(e: DictEntry, then: () -> Unit) {
        ui.confirm("Delete \"${e.word}\"?", if (e.isSnippet) "The snippet and its text are removed." else "The word is removed from the dictionary.", "Delete", onYes = {
            app.dictionary.remove(e.word)
            ui.announce("Deleted ${e.word}")
            then()
        })
    }

    // =====================================================================================
    // Edit
    // =====================================================================================

    /** False when the entry to edit no longer exists. */
    private fun renderEdit(): Boolean = with(ui) {
        val word = intent.getStringExtra(EXTRA_WORD)
        val existing = word?.let { w -> app.dictionary.all().firstOrNull { it.word.equals(w, ignoreCase = true) } }
        if (word != null && existing == null) return false
        val d = existing?.let(DictionaryDraft::of) ?: DictionaryDraft.new(kind)
        draft = d
        title = (if (existing == null) "Add " else "Edit ") + if (d.snippet) "snippet" else "word"
        screen {
            card {
                wordRow = fieldRow(
                    if (d.snippet) "Trigger phrase" else "Word or name", d.word,
                    InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
                    help = "Up to ${dev.flowclone.data.DictionaryEdit.MAX_WORD} characters", hint = if (d.snippet) "my address" else "Wispr",
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
            note("Save is in the top bar.")
            if (existing != null) {
                action(if (existing.isSnippet) "Delete snippet" else "Delete word", DevUi.Style.DANGER) {
                    confirmDelete(existing) { draft = null; finish() }
                }
            }
        }
        backCallback = OnBackInvokedCallback { confirmLeave() }.also {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, it)
        }
        validate()
        true
    }

    private fun replLabel(snippet: Boolean) = if (snippet) "Expands to" else "Replace with (optional)"
    private fun replHelp(snippet: Boolean) =
        if (snippet) "Up to ${dev.flowclone.data.DictionaryEdit.MAX_EXPANSION} characters; line breaks are kept"
        else "Leave empty to only teach the spelling; fill in to replace what the engine hears"

    private fun DevUi.FieldRow.label(): TextView = (edit.parent as ViewGroup).getChildAt(0) as TextView

    private fun relabel(snippet: Boolean) {
        wordRow?.label()?.text = if (snippet) "Trigger phrase" else "Word or name"
        replRow?.let { it.label().text = replLabel(snippet); it.help.text = replHelp(snippet) }
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
        saveItem?.let { if (it.isEnabled != can) it.isEnabled = can }
    }

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        when (page) {
            PAGE_LIST -> menu.add(Menu.NONE, MENU_ADD, Menu.NONE, "Add").apply {
                setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS or MenuItem.SHOW_AS_ACTION_WITH_TEXT)
                contentDescription = if (kind == Kind.SNIPPETS) "Add snippet" else "Add word"
            }
            PAGE_EDIT -> {
                val d = draft ?: return false
                saveItem = menu.add(Menu.NONE, MENU_SAVE, Menu.NONE, "Save").apply {
                    setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS or MenuItem.SHOW_AS_ACTION_WITH_TEXT)
                    isEnabled = d.canSave(app.dictionary.all())
                    contentDescription = "Save " + if (d.snippet) "snippet" else "word"
                }
            }
            else -> return false
        }
        return true
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean = when (item.itemId) {
        MENU_ADD -> { edit(this, kind, null); true }
        MENU_SAVE -> { onSave(); true }
        android.R.id.home -> { if (page == PAGE_EDIT) confirmLeave() else finish(); true }
        else -> super.onOptionsItemSelected(item)
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
        val uri = data?.data ?: return
        if (resultCode != RESULT_OK) return
        val msg = runCatching {
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
        msg?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show() }
        for (r in refreshers) r()
    }

    companion object {
        private const val REQ_IMPORT = 20
        private const val REQ_EXPORT = 21
        private const val MENU_ADD = 1
        private const val MENU_SAVE = 2
        const val EXTRA_PAGE = "page"
        const val EXTRA_KIND = "kind"
        const val EXTRA_WORD = "word"
        const val PAGE_HUB = "hub"
        const val PAGE_LIST = "list"
        const val PAGE_EDIT = "edit"

        fun intent(c: Context, page: String = PAGE_HUB, kind: Kind = Kind.WORDS): Intent =
            Intent(c, DictionaryActivity::class.java).putExtra(EXTRA_PAGE, page).putExtra(EXTRA_KIND, kind.name)

        fun open(c: Context, page: String, kind: Kind) = c.startActivity(intent(c, page, kind))

        /** [word] null = add a new entry of [kind]. */
        fun editIntent(c: Context, kind: Kind, word: String?): Intent = intent(c, PAGE_EDIT, kind).apply { if (word != null) putExtra(EXTRA_WORD, word) }

        fun edit(c: Context, kind: Kind, word: String?) = c.startActivity(editIntent(c, kind, word))
    }
}
