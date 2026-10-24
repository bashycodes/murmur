package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.format.DateFormat
import android.view.Menu
import android.view.MenuItem
import android.widget.LinearLayout
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.data.HistoryPages
import dev.flowclone.data.HistoryRow
import dev.flowclone.data.HistoryView
import dev.flowclone.data.Retention
import dev.flowclone.data.SettingsPages
import dev.flowclone.data.SettingsSummary
import java.util.Date
import kotlin.concurrent.thread

/**
 * Local history (never leaves the device) in the Settings look, one activity instance per page:
 *   list    retention row (→ Settings → History), raw-transcript switch, search once the list is
 *           long, dictations grouped by day ([HistoryPages]): first line, time · app, outcome
 *           badge; a dictation that never reached its field shows "Not inserted — tap to copy"
 *           and copies on tap (details from its trailing button); Clear all in the top bar
 *   detail  full text (+ raw transcript when it differs), when / app / result / per-stage
 *           timings (the device latency checklist in INSTALL.md reads them), Copy, Delete
 * Same [dev.flowclone.data.HistoryStore] calls as before (purge on open, recent, delete, clear).
 */
class HistoryActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi
    private var rows: List<HistoryRow> = emptyList()
    private var loaded = false
    private var query = ""
    private var raw = false
    private var detailId: Long? = null
    private var clearItem: MenuItem? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        detailId = intent.getLongExtra(EXTRA_ID, -1).takeIf { it >= 0 }
        if (detailId != null) { title = "Dictation"; actionBar?.setDisplayHomeAsUpEnabled(true) }
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
                if (isFinishing || isDestroyed) return@runOnUiThread
                rows = r; loaded = true
                val id = detailId
                if (id == null) renderList(retention) else r.firstOrNull { it.id == id }?.let(::renderDetail) ?: finish()
                clearItem?.isEnabled = rows.isNotEmpty()
            }
        }
    }

    private fun time(r: HistoryRow) = DateFormat.getTimeFormat(this).format(Date(r.createdAt))

    /** S16: sensitive clip, removed again after a minute. */
    private fun copy(text: String) {
        app.clipboard.copy(text)
        Toast.makeText(this, "Copied (cleared after 1 minute)", Toast.LENGTH_SHORT).show()
        ui.announce("Copied")
    }

    private fun delete(r: HistoryRow, then: () -> Unit) {
        ui.confirm("Delete this dictation?", "It is removed from this phone.", "Delete", onYes = {
            thread { app.history.delete(r.id); runOnUiThread { rows = rows.filterNot { it.id == r.id }; ui.announce("Deleted"); then() } }
        })
    }

    // =====================================================================================
    // List
    // =====================================================================================

    private var groups: LinearLayout? = null

    private fun renderList(retention: Retention): Unit = with(ui) {
        rebuild {
            sectionLabel("STATUS")
            card {
                navRow(R.drawable.ic_set_history, "Keep dictations", SettingsSummary.history(retention)) {
                    SettingsActivity.open(this@HistoryActivity, SettingsPages.Page.HISTORY)
                }
                infoRow("Saved", HistoryPages.summary(rows.size))
            }
            card {
                toggleRow("Show the raw transcript", "What the speech engine heard, before cleanup and formatting", raw).onChange { on -> raw = on; fillGroups(retention) }
            }
            if (rows.size >= HistoryPages.SEARCH_FROM) searchField("Search transcripts", query) { q -> query = q; fillGroups(retention) }
            groups = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
        }
        fillGroups(retention)
    }

    private fun fillGroups(retention: Retention): Unit = with(ui) {
        val box = groups ?: return@with
        box.removeAllViews()
        val shown = HistoryView.filter(rows, query)
        if (shown.isEmpty()) {
            val e = HistoryPages.empty(retention, query, rows.size)
            box.card { emptyState(e.title, e.hint) }
            return@with
        }
        for (g in HistoryPages.groupByDay(shown, System.currentTimeMillis())) {
            box.sectionLabel(g.label.uppercase())
            box.card {
                for (r in g.rows) {
                    val text = HistoryView.text(r, raw)
                    val details = { open(this@HistoryActivity, r.id) }
                    val recover = HistoryPages.notInserted(r)
                    listRow(
                        HistoryPages.title(r, raw), HistoryPages.meta(r, time(r)), HistoryPages.badge(r),
                        flag = HistoryPages.flag(r), flagTone = Tone.BAD,
                        trailing = if (recover) iconButton(R.drawable.ic_dev_chevron, "Open details") { details() } else null,
                        actions = listOf(
                            if (recover) "Open details" to details else "Copy" to { copy(text) },
                            "Delete" to { delete(r) { fillGroups(retention) } },
                        ),
                    ) { if (recover) copy(text) else details() }
                }
            }
        }
    }

    // =====================================================================================
    // Detail
    // =====================================================================================

    private fun renderDetail(r: HistoryRow): Unit = with(ui) {
        rebuild {
            if (HistoryPages.notInserted(r)) {
                banner().show(
                    listOfNotNull("Not inserted: this text never reached the field. Copy it and paste it where you meant.", HistoryPages.reason(r)).joinToString(" "),
                    Tone.WARN,
                )
            }
            sectionLabel(if (r.inserted) "INSERTED TEXT" else "TEXT")
            card { selectableText(HistoryView.text(r, raw = false)) }
            if (HistoryView.hasRawVariant(r)) {
                sectionLabel("RAW TRANSCRIPT")
                card { selectableText(HistoryView.text(r, raw = true), 14f, p.muted) }
            }
            action("Copy", DevUi.Style.PRIMARY) { copy(HistoryView.text(r, raw = false)) }
            // HOOK(fix/route-order): a "Re-insert" action goes here once the service can insert a history entry.
            sectionLabel("DETAILS")
            card {
                infoRow("When", DateFormat.getLongDateFormat(this@HistoryActivity).format(Date(r.createdAt)) + ", " + time(r))
                infoRow("App", HistoryPages.app(r))
                infoRow("Result", listOfNotNull(HistoryPages.badge(r), dev.flowclone.data.LlmOutcome.label(r.fallbackReason)).joinToString(" · "))
                infoRow("Inserted", if (r.inserted) "Yes" else "No")
                HistoryPages.timings(r).takeIf { it.isNotEmpty() }?.let { infoRow("Timings", it) }
            }
            action("Delete dictation", DevUi.Style.DANGER) { delete(r) { finish() } }
        }
    }

    // ---- top bar ----

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        if (detailId != null) return false
        clearItem = menu.add(Menu.NONE, MENU_CLEAR, Menu.NONE, "Clear all").apply {
            setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM or MenuItem.SHOW_AS_ACTION_WITH_TEXT)
            contentDescription = "Clear all history"
            isEnabled = !loaded || rows.isNotEmpty()
        }
        return true
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean = when (item.itemId) {
        MENU_CLEAR -> {
            ui.confirm("Delete all history?", "Every saved dictation is deleted from this phone.", "Delete all", onYes = {
                thread { app.history.clear(); runOnUiThread { ui.announce("History cleared"); load() } }
            })
            true
        }
        android.R.id.home -> { finish(); true }
        else -> super.onOptionsItemSelected(item)
    }

    companion object {
        const val EXTRA_ID = "id"
        private const val MENU_CLEAR = 1

        fun detailIntent(c: Context, id: Long): Intent = Intent(c, HistoryActivity::class.java).putExtra(EXTRA_ID, id)

        fun open(c: Context, id: Long) = c.startActivity(detailIntent(c, id))
    }
}
