package dev.flowclone.ui

import android.app.Activity
import android.text.format.DateFormat
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.PopupMenu
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.data.HistoryPages
import dev.flowclone.data.HistoryRow
import dev.flowclone.data.HistoryVersion
import dev.flowclone.data.RetryLogic
import dev.flowclone.ui.RetryUi.versionPills
import dev.flowclone.data.ShellPages
import java.util.Date
import kotlin.concurrent.thread

/**
 * Home tab (AndroidHome.dc.html): recent dictations as cards grouped by day ("Today",
 * "Yesterday", …): the text, time + route tag, Copy, and ⋮ (Open details → HistoryActivity
 * detail, Retry with current settings while the recording is kept (D35), Delete). A retried
 * card shows numbered version pills (0 = the original; view only). Above the cards, problem
 * notices (setup missing, engine down) that open where they are fixed. Reloads on every [onShow].
 */
internal class HomeTab(private val activity: Activity, private val ui: DevUi) : ShellTab {
    private val app get() = FlowApp.of(activity)
    override val title = ShellPages.APP_NAME

    /** A peach notice above the cards; tapping it runs [onClick]. */
    class Notice(val lead: String, val text: String, val onClick: () -> Unit)

    private lateinit var notices: LinearLayout
    private lateinit var list: LinearLayout
    private var view: View? = null
    private var loadSeq = 0
    /** True once the first load has rendered (tests wait for it). */
    var loaded = false; private set

    override fun createView(): View = view ?: ui.scrollColumn(ui.t.bg) {
        notices = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
        list = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }.also { addView(it) }
    }.first.also { v -> view = v; RetryUi.watch(activity, v) { load() } }

    override fun onShow() = load()

    fun setNotices(items: List<Notice>): Unit = with(ui) {
        if (view == null) createView()
        notices.removeAllViews()
        for (n in items) notices.callout(n.text, n.lead.ifEmpty { null }).apply {
            isClickable = true; isFocusable = true
            minHeight = dp(48)
            foreground = ripple(null, 16)
            setOnClickListener { n.onClick() }
        }
    }

    private fun load() {
        val seq = ++loadSeq
        thread(name = "home-load") {
            val retention = app.settings.load().retention
            val rows = runCatching { app.history.purge(retention); app.history.recent(ShellPages.HOME_LIMIT) }.getOrDefault(emptyList())
            val ids = rows.map { it.id }
            val audio = runCatching { app.history.withAudio(ids) }.getOrDefault(emptySet())
            val versions = runCatching { app.history.versionsFor(ids) }.getOrDefault(emptyMap())
            activity.runOnUiThread {
                if (activity.isFinishing || activity.isDestroyed || seq != loadSeq) return@runOnUiThread
                this.audio = audio; this.versions = versions
                render(rows, retention)
            }
        }
    }

    /** D35: rows whose recording is kept, and the versions of retried rows (from the last load). */
    private var audio: Set<Long> = emptySet()
    private var versions: Map<Long, List<HistoryVersion>> = emptyMap()

    private fun time(r: HistoryRow) = DateFormat.getTimeFormat(activity).format(Date(r.createdAt))

    private fun render(rows: List<HistoryRow>, retention: dev.flowclone.data.Retention): Unit = with(ui) {
        if (view == null) createView()
        list.removeAllViews()
        loaded = true
        if (rows.isEmpty()) {
            val e = HistoryPages.empty(retention, "", 0)
            list.card { emptyState(e.title, e.hint) }
            return@with
        }
        val now = System.currentTimeMillis()
        for (g in HistoryPages.groupByDay(rows, now)) {
            list.heading(g.label)
            for (r in g.rows) list.historyCard(r, now)
        }
    }

    private fun route(r: HistoryRow): DevUi.Route? = when (ShellPages.route(r)) {
        ShellPages.RouteKind.PHONE -> DevUi.Route.PHONE
        ShellPages.RouteKind.SERVER -> DevUi.Route.SERVER
        ShellPages.RouteKind.CLOUD -> DevUi.Route.CLOUD
        null -> null
    }

    private fun LinearLayout.historyCard(r: HistoryRow, now: Long): Unit = with(ui) {
        val text = ShellPages.cardText(r)
        val whenText = ShellPages.cardTime(r.createdAt, now, time(r))
        val rt = route(r)
        val notInserted = HistoryPages.notInserted(r)
        val card = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            background = rounded(p.surface, 24)
            setPadding(dp(20), dp(18), dp(20), dp(8))
        }
        val textView = tv(text, TypeScale.ROW, p.text)
        val busy = r.id in app.retrying.value
        val body = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            addView(textView)
            addView(LinearLayout(activity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                addView(tv(whenText, 14f, p.muted))
                if (busy) addView(tv("Retrying…", 14f, p.muted, TypeScale.W_MEDIUM), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) })
                if (rt != null) addView(routeTag(rt), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) })
                if (notInserted) addView(tv("Not inserted", 14f, p.danger, TypeScale.W_MEDIUM), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) })
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(10) })
            // One TalkBack stop for the dictation: "text. 9:37am, Your server".
            isFocusable = true
            contentDescription = listOfNotNull(text, listOfNotNull(whenText, rt?.label, if (notInserted) "Not inserted" else null).joinToString(", ")).joinToString(". ")
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        }
        for (i in 0 until body.childCount) body.getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        card.addView(body)
        val meta = listOfNotNull(whenText, rt?.label, if (notInserted) "Not inserted" else null).joinToString(", ")
        versionPills(versions[r.id].orEmpty(), textView, text) { shown -> body.contentDescription = "$shown. $meta" }?.let {
            card.addView(it, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(10) })
        }

        val copy = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(48)
            setPadding(dp(12), 0, dp(12), 0)
            background = ripple(null, 24)
            isClickable = true; isFocusable = true
            contentDescription = "Copy dictation from $whenText"
            addView(icon(R.drawable.ic_lw_copy, p.text, 18).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(8) })
            addView(tv("Copy", 15f, p.text, TypeScale.W_MEDIUM))
            setOnClickListener { copy(HistoryPages.copyText(r)) }
        }
        lateinit var more: View
        more = iconButton(R.drawable.ic_lw_more, "More actions for dictation from $whenText") { menu(r, more) }
        card.addView(LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            addView(copy, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            addView(View(activity), LinearLayout.LayoutParams(0, 1, 1f))
            addView(more)
        }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(6); marginStart = -dp(8); marginEnd = -dp(8)
        })
        addView(card, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(6); bottomMargin = dp(6) })
    }

    private fun menu(r: HistoryRow, anchor: View) {
        val items = RetryLogic.menu(r, r.id in audio)
        PopupMenu(activity, anchor).apply {
            items.forEachIndexed { i, item ->
                menu.add(0, item.ordinal, i, item.label).isEnabled = item != RetryLogic.MenuItem.RETRY || r.id !in app.retrying.value
            }
            setOnMenuItemClickListener {
                when (RetryLogic.MenuItem.entries[it.itemId]) {
                    RetryLogic.MenuItem.DETAILS -> HistoryActivity.open(activity, r.id)
                    RetryLogic.MenuItem.RETRY -> RetryUi.start(activity, r.id)
                    RetryLogic.MenuItem.DELETE -> delete(r)
                }
                true
            }
        }.show()
    }

    /** S16: a sensitive clip, removed again after a minute (same as History). */
    private fun copy(text: String) {
        app.clipboard.copy(text)
        Toast.makeText(activity, "Copied (cleared after 1 minute)", Toast.LENGTH_SHORT).show()
        ui.announce("Copied")
    }

    private fun delete(r: HistoryRow) {
        ui.confirm("Delete this dictation?", "It is removed from this phone.", "Delete", onYes = {
            thread(name = "home-delete") {
                app.history.delete(r.id)
                activity.runOnUiThread { ui.announce("Deleted"); load() }
            }
        })
    }
}
