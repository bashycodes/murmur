package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.format.DateFormat
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ImageButton
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
import dev.flowclone.data.ShellPages
import java.util.Date
import kotlin.concurrent.thread

/**
 * Local history (never leaves the device), one activity instance per page; the design's top bar
 * with back (no artboard of its own: it follows AndroidHome.dc.html's dictation cards):
 *   list    retention row (→ Settings → History), raw-transcript switch, search once the list is
 *           long, dictations grouped by day ([HistoryPages]) as cards: the text, time · app, route
 *           tag, outcome badge, Copy and ⋮ (Open details, Delete); tapping a card opens its
 *           details, except a dictation that never reached its field ("Not inserted — tap to
 *           copy"), which copies; Clear all in the top bar
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
    private var clearButton: ImageButton? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
        detailId = intent.getLongExtra(EXTRA_ID, -1).takeIf { it >= 0 }
        if (detailId != null) title = DETAIL_TITLE
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
                clearButton?.let { it.isEnabled = rows.isNotEmpty(); it.alpha = if (rows.isNotEmpty()) 1f else 0.4f }
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
        rebuild(t.bg) {
            clearButton = topBar(
                LIST_TITLE, DevUi.TopNav.BACK, { finish() },
                actions = listOf(DevUi.TopAction(R.drawable.ic_lw_delete, CLEAR_ALL) { clearAll() }),
            ).actions.first()
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
        val now = System.currentTimeMillis()
        for (g in HistoryPages.groupByDay(shown, now)) {
            box.heading(g.label)
            for (r in g.rows) box.addView(dictationCard(r, now, retention))
        }
    }

    /**
     * AndroidHome.dc.html's dictation card: the text, "time · app" with the route tag and the
     * outcome badge, the not-inserted flag, then Copy and ⋮ (Open details, Delete). The text
     * block is one touch target and one TalkBack stop (with Copy / Delete as custom actions).
     */
    private fun dictationCard(r: HistoryRow, now: Long, retention: Retention): View = with(ui) {
        val text = HistoryPages.cardText(r, raw)
        val meta = HistoryPages.meta(r, ShellPages.cardTime(r.createdAt, now, time(r)))
        val route = routeOf(ShellPages.route(r))
        val outcome = HistoryView.label(r)
        val flag = HistoryPages.flag(r)
        val recover = HistoryPages.notInserted(r)
        val details = { open(this@HistoryActivity, r.id) }
        val remove = { delete(r) { fillGroups(retention) } }
        val card = itemCard()
        val body = LinearLayout(this@HistoryActivity).apply {
            orientation = LinearLayout.VERTICAL
            minimumHeight = dp(48)
            background = ripple(null, 12)
            isClickable = true; isFocusable = true
            addView(tv(text, TypeScale.ROW, p.text).apply { maxLines = 6; ellipsize = android.text.TextUtils.TruncateAt.END })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                addView(tv(meta, 14f, p.muted).apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END },
                    LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(8) })
                if (route != null) addView(routeTag(route), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginEnd = dp(6) })
                addView(badge(outcome))
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(10) })
            if (flag != null) addView(tv(flag, 14f, p.danger, TypeScale.W_MEDIUM).apply { setPadding(0, dp(6), 0, 0) })
            for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            contentDescription = HistoryPages.cardDescription(text, meta, route?.label, outcome, flag)
            setOnClickListener { if (recover) copy(HistoryPages.copyText(r)) else details() }
            customActions(this, listOf(
                if (recover) "Open details" to details else "Copy" to { copy(HistoryPages.copyText(r)) },
                "Delete" to remove,
            ))
        }
        card.addView(body)
        val whenText = ShellPages.cardTime(r.createdAt, now, time(r))
        card.addView(cardActions(
            copyPill("Copy dictation from $whenText") { copy(HistoryPages.copyText(r)) },
            moreButton("More actions for dictation from $whenText", listOf("Open details" to details, "Delete" to remove)),
        ))
        card
    }

    // =====================================================================================
    // Detail
    // =====================================================================================

    private fun renderDetail(r: HistoryRow): Unit = with(ui) {
        rebuild {
            topBar(DETAIL_TITLE, DevUi.TopNav.BACK, { finish() })
            if (HistoryPages.notInserted(r)) {
                banner().show(
                    listOfNotNull("Not inserted: this text never reached the field. Copy it and paste it where you meant.", HistoryPages.reason(r)).joinToString(" "),
                    Tone.WARN,
                )
            }
            if (r.isCommand) {
                // Command Mode: what was said, what it acted on, what came back.
                sectionLabel("INSTRUCTION")
                card { selectableText(HistoryView.text(r, raw = true)) }
                sectionLabel(if (r.commandOriginal != null) "ORIGINAL SELECTION" else "ORIGINAL")
                card { selectableText(r.commandOriginal ?: "(nothing selected: written at the cursor)", 14f, p.muted) }
                sectionLabel(if (r.inserted) "RESULT (REPLACED THE SELECTION)" else "RESULT")
                card { selectableText(HistoryView.text(r, raw = false)) }
            } else {
                sectionLabel(if (r.inserted) "INSERTED TEXT" else "TEXT")
                card { selectableText(HistoryView.text(r, raw = false)) }
            }
            if (!r.isCommand && HistoryView.hasRawVariant(r)) {
                sectionLabel("RAW TRANSCRIPT")
                card { selectableText(HistoryView.text(r, raw = true), 14f, p.muted) }
            }
            action("Copy", DevUi.Style.PRIMARY) { copy(HistoryPages.copyText(r)) }
            if (HistoryPages.notInserted(r)) action(REINSERT) { offerInsert(r) }
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

    /**
     * "Insert with the bubble": this page has no field to type into, so the text cannot go in
     * from here. It arms the bubble's ✓ insert instead (same path as a not-inserted dictation:
     * the field focused when ✓ is tapped, never a protected one); the user goes back to the app,
     * taps the field and then ✓. Needs the running service (the bubble lives there).
     */
    private fun offerInsert(r: HistoryRow) {
        if (!dev.flowclone.service.FlowService.running) {
            Toast.makeText(this, REINSERT_NEEDS_SERVICE, Toast.LENGTH_LONG).show()
            return
        }
        startService(
            Intent(this, dev.flowclone.service.FlowService::class.java)
                .setAction(dev.flowclone.service.FlowService.ACTION_OFFER_INSERT)
                .putExtra(dev.flowclone.service.FlowService.EXTRA_HISTORY_ID, r.id),
        )
    }

    // ---- top bar ----

    private fun clearAll() {
        if (loaded && rows.isEmpty()) return
        ui.confirm("Delete all history?", "Every saved dictation is deleted from this phone.", "Delete all", onYes = {
            thread { app.history.clear(); runOnUiThread { ui.announce("History cleared"); load() } }
        })
    }

    companion object {
        const val EXTRA_ID = "id"
        const val REINSERT = "Insert with the bubble"
        const val REINSERT_NEEDS_SERVICE = "Turn on the bubble first (menu → Start / show bubble), then try again"
        const val LIST_TITLE = "History"
        const val DETAIL_TITLE = "Dictation"
        /** The top bar's Clear all button (TalkBack label). */
        const val CLEAR_ALL = "Clear all history"

        fun detailIntent(c: Context, id: Long): Intent = Intent(c, HistoryActivity::class.java).putExtra(EXTRA_ID, id)

        fun open(c: Context, id: Long) = c.startActivity(detailIntent(c, id))
    }
}
