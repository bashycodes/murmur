package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.ViewGroup
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryEdit

/**
 * Dictionary words (spelling / replacements the core applies and feeds to whisper's prompt)
 * and snippets (trigger phrase → expansion). Search, tabs, tap-to-edit, delete with confirm.
 * Import/export use the shared JSON `[{word, replacement, isSnippet}]` (same file the
 * Windows clone reads), via SAF.
 */
class DictionaryActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private var tab = DictionaryEdit.Tab.ALL
    private var query = ""
    private var editing: String? = null

    private lateinit var word: EditText
    private lateinit var repl: EditText
    private lateinit var snip: CheckBox
    private lateinit var save: Button
    private lateinit var formTitle: TextView
    private lateinit var list: LinearLayout
    private lateinit var tabs: List<Pair<DictionaryEdit.Tab, Button>>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        page {
            formTitle = heading("Add")
            word = EditText(context).apply { hint = "Word, name or snippet trigger (≤ 60 characters)"; isSingleLine = true }.also(::addView)
            repl = EditText(context).apply {
                hint = "Replace with / expands to (optional for words)"
                inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE; minLines = 1; maxLines = 6
            }.also(::addView)
            snip = CheckBox(context).apply { text = "Snippet: saying the trigger inserts the text" }.also(::addView)
            body(
                "Words: whisper is primed with them and \"wisper\" → \"Wispr\" style fixes are applied. " +
                    "Snippets: say just the trigger and its text is inserted.",
            )
            save = Button(context).apply { text = "Add"; isAllCaps = false; setOnClickListener { submit() } }
            val cancel = Button(context).apply { text = "Clear"; isAllCaps = false; setOnClickListener { startEdit(null) } }
            row(save, cancel)
            row(
                Button(context).apply { text = "Import JSON"; isAllCaps = false; setOnClickListener { pick() } },
                Button(context).apply { text = "Export JSON"; isAllCaps = false; setOnClickListener { export() } },
            )
            heading("Entries")
            EditText(context).apply {
                hint = "Search"; isSingleLine = true
                addTextChangedListener(object : TextWatcher {
                    override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                    override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                    override fun afterTextChanged(s: Editable?) { query = s?.toString().orEmpty(); renderList() }
                })
            }.also(::addView)
            tabs = DictionaryEdit.Tab.entries.map { t ->
                t to Button(context).apply {
                    isAllCaps = false
                    setOnClickListener { tab = t; renderList() }
                }
            }
            row(*tabs.map { it.second }.toTypedArray())
            list = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
            addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        }
        renderList()
    }

    private fun startEdit(e: DictEntry?) {
        editing = e?.word
        word.setText(e?.word.orEmpty())
        repl.setText(e?.replacement.orEmpty())
        snip.isChecked = e?.isSnippet == true
        formTitle.text = if (e == null) "Add" else "Edit \"${e.word}\""
        save.text = if (e == null) "Add" else "Save"
        if (e != null) word.requestFocus()
    }

    private fun submit() {
        val e = DictionaryEdit.normalize(word.text.toString(), repl.text.toString(), snip.isChecked)
        val all = app.dictionary.all()
        DictionaryEdit.validate(all, editing, e, wantSnippet = snip.isChecked)?.let {
            Toast.makeText(this, it, Toast.LENGTH_LONG).show(); return
        }
        app.dictionary.replaceAll(DictionaryEdit.apply(all, editing, e))
        startEdit(null)
        renderList()
    }

    private fun renderList() {
        val all = app.dictionary.all()
        for ((t, b) in tabs) {
            val n = when (t) {
                DictionaryEdit.Tab.ALL -> all.size
                DictionaryEdit.Tab.WORDS -> all.count { !it.isSnippet }
                DictionaryEdit.Tab.SNIPPETS -> all.count { it.isSnippet }
            }
            val name = t.name.lowercase().replaceFirstChar { it.uppercase() }
            b.text = "$name ($n)"
            b.setTypeface(null, if (t == tab) Typeface.BOLD else Typeface.NORMAL)
            b.isEnabled = t != tab
        }
        list.removeAllViews()
        val shown = DictionaryEdit.filter(all, query, tab)
        if (shown.isEmpty()) list.body(if (all.isEmpty()) "No entries yet." else "Nothing matches.")
        for (e in shown) {
            val label = buildString {
                append(e.word)
                e.replacement?.let { append(if (e.isSnippet) "  ⇒ " else "  → ").append(it.replace('\n', ' ').take(80)) }
            }
            list.row(
                TextView(this).apply {
                    text = label; textSize = 15f
                    setPadding(0, dp(10), dp(8), dp(10))
                    if (e.isSnippet) setTypeface(null, Typeface.ITALIC)
                    setOnClickListener { startEdit(e) }
                },
                Button(this).apply { text = "Delete"; isAllCaps = false; setOnClickListener { confirmDelete(e) } },
            ).also { (it.getChildAt(0).layoutParams as LinearLayout.LayoutParams).weight = 3f }
        }
    }

    private fun confirmDelete(e: DictEntry) {
        AlertDialog.Builder(this)
            .setMessage("Delete \"${e.word}\"?")
            .setPositiveButton("Delete") { _, _ ->
                app.dictionary.remove(e.word)
                if (editing.equals(e.word, ignoreCase = true)) startEdit(null)
                renderList()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

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
        renderList()
    }

    companion object {
        private const val REQ_IMPORT = 20
        private const val REQ_EXPORT = 21
    }
}
