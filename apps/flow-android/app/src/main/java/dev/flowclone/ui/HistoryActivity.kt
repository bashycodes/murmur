package dev.flowclone.ui

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.os.Bundle
import android.text.format.DateFormat
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import kotlin.concurrent.thread

/** Local history (never leaves the device). Tap an entry to copy its text. */
class HistoryActivity : Activity() {
    private val app get() = FlowApp.of(this)

    override fun onResume() {
        super.onResume()
        thread(name = "history-load") {
            val retention = app.settings.load().retention
            app.history.purge(retention)
            val rows = app.history.recent()
            runOnUiThread { render(rows, retention.key) }
        }
    }

    private fun render(rows: List<dev.flowclone.data.HistoryRow>, retention: String) {
        page {
            heading("History")
            body("Retention: $retention (change in Settings). Audio is never stored.")
            button("Clear history") { thread { app.history.clear(); runOnUiThread { render(emptyList(), retention) } } }
            if (rows.isEmpty()) body("Nothing yet.")
            for (r in rows) {
                val text = r.finalText.ifEmpty { "(empty)" }
                TextView(context).apply {
                    val time = DateFormat.format("MMM d, HH:mm", r.createdAt)
                    this.text = "$time · ${r.appId ?: "?"} · ${r.status}${if (r.inserted) "" else " · not inserted"}\n$text"
                    setPadding(0, dp(10), 0, dp(10))
                    setOnClickListener {
                        getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Flow Clone", r.finalText))
                        Toast.makeText(this@HistoryActivity, "Copied", Toast.LENGTH_SHORT).show()
                    }
                }.also(::addView)
            }
        }
    }
}
