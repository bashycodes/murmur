package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import dev.flowclone.FlowApp
import dev.flowclone.data.Insights
import dev.flowclone.data.InsightsPage
import dev.flowclone.data.ShellPages
import kotlin.concurrent.thread

/**
 * Insights (main screen → *Insights*), in the design's look (no Android artboard: the desktop
 * PageInsights.dc.html and the drawer's stat card): top bar with back, the serif stat row
 * (words / wpm / day streak) with how that compares with typing, then words dictated, last 7 days, speaking speed, streaks,
 * time saved vs typing, dictations, top apps and a words-per-day chart. Read from
 * [dev.flowclone.data.InsightsStore] (counts only, on this phone; History can be cleared without
 * touching it). Reloaded on every resume, so a dictation made meanwhile shows when you come back.
 */
class InsightsActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private lateinit var ui: DevUi

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = DevUi(this)
    }

    override fun onResume() {
        super.onResume()
        load()
    }

    /** Off Main: the first open folds History in (a full table read). */
    private fun load() {
        thread(name = "insights-load") {
            val s = runCatching { app.insights.summary() }.getOrNull()
            runOnUiThread { if (!isFinishing && !isDestroyed && s != null) render(s) }
        }
    }

    private fun label(pkg: String): String? = runCatching {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    }.getOrNull()

    private fun render(s: Insights.Summary): Unit = with(ui) {
        rebuild(t.bg) {
            topBar(TITLE, DevUi.TopNav.BACK, { finish() })
            statRow(ShellPages.stats(s))
            InsightsPage.speedVsTyping(s.avgWpm, s.typingWpm)?.let { note(it, p.text) }
            sectionLabel("LAST 7 DAYS")
            card {
                addView(
                    InsightsChartView(context, p, s.last7),
                    LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, (160 * resources.displayMetrics.density).toInt()),
                )
                infoRow("Last 7 days", InsightsPage.number(s.weekWords))
            }
            sectionLabel("TOTALS")
            card {
                infoRow("Words dictated", InsightsPage.number(s.totalWords))
                infoRow("Dictations", InsightsPage.dictations(s.dictations))
                infoRow("Average speed", InsightsPage.wpm(s.avgWpm))
                infoRow("Current streak", InsightsPage.days(s.currentStreak))
                infoRow("Longest streak", InsightsPage.days(s.longestStreak))
                infoRow("Time saved", InsightsPage.saved(s.savedMinutes))
                note(InsightsPage.savedHint(s.typingWpm)).setPadding(dp(20), 0, dp(20), dp(14))
            }
            sectionLabel("TOP APPS")
            card {
                if (s.topApps.isEmpty()) emptyState("No dictations yet", "Dictate with the bubble and your words per app show up here.")
                val most = s.topApps.maxOfOrNull { it.words } ?: 0L
                for (a in s.topApps) appRow(InsightsPage.appName(a.app, ::label), a.words, InsightsPage.share(a.words, most))
            }
            note(
                "Counted on this phone only and never sent anywhere. Clearing History keeps these numbers." +
                    (s.since?.let { " Counting since $it." } ?: ""),
            )
            action("Reset stats", DevUi.Style.DANGER) {
                confirm("Reset stats?", "Every Insights count on this phone goes back to zero. History is not touched.", "Reset", onYes = {
                    thread { runCatching { app.insights.reset() }; runOnUiThread { announce("Stats reset"); load() } }
                })
            }
        }
    }

    /** A top app: name and word count over a thin ink bar as long as its share of the busiest app. */
    private fun LinearLayout.appRow(name: String, words: Long, share: Float): Unit = with(ui) {
        val count = "${InsightsPage.number(words)} words"
        addView(LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(12), dp(20), dp(14))
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                addView(tv(name, TypeScale.ROW, p.text).apply { maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END },
                    LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
                addView(tv(count, TypeScale.SUB, p.muted))
            })
            addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                background = rounded(t.selected, 3)
                clipToOutline = true
                weightSum = 1f
                addView(View(context).apply { background = rounded(p.text, 3) }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, share))
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(6)).apply { topMargin = dp(8) })
            for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            isFocusable = true
            contentDescription = "$name, $count"
        })
    }

    companion object {
        const val TITLE = "Insights"
    }
}

/**
 * The 7-day words-per-day bars (oldest left, today right), drawn on a Canvas: no chart library.
 * TalkBack reads [InsightsPage.chartDescription]. Built in code only (never inflated from XML).
 */
@android.annotation.SuppressLint("ViewConstructor")
internal class InsightsChartView(context: Context, private val p: DevPalette, private val days: List<Insights.DayWords>) : View(context) {
    private val labels = InsightsPage.chartLabels(days)
    private val density = resources.displayMetrics.density
    private val bar = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = p.accent }
    private val empty = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = p.divider }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = p.muted; textSize = android.util.TypedValue.applyDimension(android.util.TypedValue.COMPLEX_UNIT_SP, 12f, resources.displayMetrics); textAlign = Paint.Align.CENTER
    }
    private val rect = RectF()

    init {
        contentDescription = InsightsPage.chartDescription(days)
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        if (days.isEmpty()) return
        val pad = 16 * density
        val w = width - 2 * pad
        val labelH = text.textSize * 1.6f
        val valueH = text.textSize * 1.6f
        val top = pad + valueH
        val bottom = height - pad - labelH
        val max = (days.maxOf { it.words }).coerceAtLeast(1)
        val slot = w / days.size
        val bw = slot * 0.5f
        for ((i, d) in days.withIndex()) {
            val cx = pad + slot * i + slot / 2
            val h = if (d.words == 0L) 2 * density else ((bottom - top) * d.words / max).coerceAtLeast(2 * density)
            rect.set(cx - bw / 2, bottom - h, cx + bw / 2, bottom)
            canvas.drawRoundRect(rect, 4 * density, 4 * density, if (d.words == 0L) empty else bar)
            canvas.drawText(labels[i], cx, height - pad, text)
            if (d.words > 0) canvas.drawText(InsightsPage.number(d.words), cx, bottom - h - text.textSize * 0.5f, text)
        }
    }
}
