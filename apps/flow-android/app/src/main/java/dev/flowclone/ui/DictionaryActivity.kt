package dev.flowclone.ui

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.core.DictEntry

/**
 * Dictionary words, replacements and snippets. Import/export use the shared JSON
 * `[{word, replacement, isSnippet}]` (same file the Windows clone reads), via SAF.
 */
class DictionaryActivity : Activity() {
    private val app get() = FlowApp.of(this)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        render()
    }

    private fun render() {
        page {
            heading("Add")
            val word = EditText(context).apply { hint = "Word or trigger phrase" }.also(::addView)
            val repl = EditText(context).apply { hint = "Replacement / snippet text (optional)" }.also(::addView)
            val snip = CheckBox(context).apply { text = "Snippet (expand the trigger into the text)" }.also(::addView)
            button("Add") {
                val w = word.text.toString().trim()
                if (w.isEmpty()) return@button
                val r = repl.text.toString().takeIf { it.isNotBlank() }
                app.dictionary.upsert(DictEntry(w, r, snip.isChecked && r != null))
                render()
            }
            row(
                Button(context).apply { text = "Import JSON"; isAllCaps = false; setOnClickListener { pick() } },
                Button(context).apply { text = "Export JSON"; isAllCaps = false; setOnClickListener { export() } },
            )
            val all = app.dictionary.all()
            heading("Entries (${all.size})")
            for (e in all) {
                val label = buildString {
                    append(e.word)
                    e.replacement?.let { append(if (e.isSnippet) "  ⇒ snippet: " else "  → ").append(it.take(80)) }
                }
                row(
                    android.widget.TextView(context).apply { text = label },
                    Button(context).apply { text = "Remove"; isAllCaps = false; setOnClickListener { app.dictionary.remove(e.word); render() } },
                )
            }
        }
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
        render()
    }

    companion object {
        private const val REQ_IMPORT = 20
        private const val REQ_EXPORT = 21
    }
}
