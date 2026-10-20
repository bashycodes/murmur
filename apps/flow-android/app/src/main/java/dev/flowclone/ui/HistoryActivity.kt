package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.os.Bundle
import android.os.PersistableBundle
import android.text.Editable
import android.text.TextWatcher
import android.text.format.DateFormat
import android.view.ViewGroup
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.data.HistoryRow
import dev.flowclone.data.HistoryView
import kotlin.concurrent.thread

/**
 * Local history (never leaves the device): search, copy, delete, and raw ASR text vs the
 * formatted text that was inserted. Rows also show per-stage timings, which the device
 * latency checklist (INSTALL.md) reads.
 */
class HistoryActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private var rows: List<HistoryRow> = emptyList()
    private var query = ""
    private var raw = false
    private lateinit var list: LinearLayout
    private lateinit var summary: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        page {
            heading("History")
            summary = body("")
            EditText(context).apply {
                hint = "Search transcripts"; isSingleLine = true
                addTextChangedListener(object : TextWatcher {
                    override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                    override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                    override fun afterTextChanged(s: Editable?) { query = s?.toString().orEmpty(); renderList() }
                })
            }.also(::addView)
            CheckBox(context).apply {
                text = "Show the raw transcript (before cleanup)"
                setOnCheckedChangeListener { _, c -> raw = c; renderList() }
            }.also(::addView)
            button("Clear history") {
                AlertDialog.Builder(this@HistoryActivity).setMessage("Delete all history?")
                    .setPositiveButton("Delete all") { _, _ -> thread { app.history.clear(); runOnUiThread { load() } } }
                    .setNegativeButton("Cancel", null).show()
            }
            list = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
            addView(list, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        }
    }

    override fun onResume() {
        super.onResume()
        load()
    }

    private fun load() {
        thread(name = "history-load") {
            val retention = app.settings.load().retention
            app.history.purge(retention)
            val r = app.history.recent()
            runOnUiThread {
                rows = r
                summary.text = "Retention: ${retention.key} (change in Settings). Audio is never stored. ${r.size} entries."
                renderList()
            }
        }
    }

    private fun renderList() {
        list.removeAllViews()
        val shown = HistoryView.filter(rows, query)
        if (shown.isEmpty()) list.body(if (rows.isEmpty()) "Nothing yet." else "Nothing matches.")
        for (r in shown) {
            val time = DateFormat.format("MMM d, HH:mm", r.createdAt)
            val timing = buildString {
                if (r.audioMs > 0) append(" · ${r.audioMs / 100 / 10.0}s audio")
                if (r.asrMs > 0) append(" · asr ${r.asrMs} ms")
                if (r.llmMs > 0) append(" · llm ${r.llmMs} ms")
                r.fallbackReason?.let { append(" · $it") }
            }
            val variant = if (raw && HistoryView.hasRawVariant(r)) " · raw" else ""
            list.addView(TextView(this).apply {
                text = "$time · ${r.appId ?: "?"} · ${HistoryView.label(r)}$variant${if (r.inserted) "" else " · not inserted"}$timing"
                textSize = 12f; alpha = 0.7f
                setPadding(0, dp(12), 0, 0)
            })
            val text = HistoryView.text(r, raw)
            list.addView(TextView(this).apply { this.text = text; textSize = 15f; setTextIsSelectable(true) })
            list.row(
                Button(this).apply { this.text = "Copy"; isAllCaps = false; setOnClickListener { copy(text) } },
                Button(this).apply { this.text = "Delete"; isAllCaps = false; setOnClickListener { delete(r) } },
            )
        }
    }

    private fun copy(text: String) {
        val clip = ClipData.newPlainText("Flow Clone", text).apply {
            description.extras = PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true) }
        }
        getSystemService(ClipboardManager::class.java)?.setPrimaryClip(clip)
        Toast.makeText(this, "Copied", Toast.LENGTH_SHORT).show()
    }

    private fun delete(r: HistoryRow) {
        thread { app.history.delete(r.id); runOnUiThread { rows = rows.filterNot { it.id == r.id }; renderList() } }
    }
}
