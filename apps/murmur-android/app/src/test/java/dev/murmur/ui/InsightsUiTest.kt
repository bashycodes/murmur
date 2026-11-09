package dev.flowclone.ui

import android.app.Activity
import android.content.DialogInterface
import dev.flowclone.FlowApp
import dev.flowclone.data.HistoryRow
import dev.flowclone.data.Retention
import dev.flowclone.ui.UiTestKit.idle
import dev.flowclone.ui.UiTestKit.texts
import dev.flowclone.ui.UiTestKit.walk
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog
import java.io.File

/** Insights page: stats from the aggregate, refresh on return, History independence, Reset. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class InsightsUiTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val app get() = FlowApp.of(ctx)

    @After fun cleanup() { app.history.clear(); File(ctx.filesDir, "insights.json").delete() }

    private fun <T : Activity> settle(a: T, ready: (T) -> Boolean): T {
        repeat(200) { idle(); if (ready(a)) return a; Thread.sleep(10) }
        idle(); return a
    }

    private fun launch() = Robolectric.buildActivity(InsightsActivity::class.java).setup().visible()

    /** The value under the row labelled [label]. */
    private fun value(a: Activity, label: String): String {
        val l = walk(a.window.decorView).first { it is android.widget.TextView && it.text.toString() == label }
        val row = l.parent as android.view.ViewGroup
        return (row.getChildAt(row.indexOfChild(l) + 1) as android.widget.TextView).text.toString()
    }

    @Test fun emptyPageExplainsAndShowsZeroes() {
        val a = settle(launch().get()) { "TOTALS" in texts(it) }
        val t = texts(a)
        for (want in listOf("LAST 7 DAYS", "TOTALS", "TOP APPS", "Reset stats")) assertTrue(want, want in t)
        assertEquals("0", value(a, "Words dictated"))
        assertEquals("0 days", value(a, "Current streak"))
        assertEquals("No recordings yet", value(a, "Average speed"))
        assertTrue(t.toString(), t.any { it.contains("40 words/min") })
        assertTrue(walk(a.window.decorView).any { it is InsightsChartView })
        assertTrue(InsightsActivity.TITLE in t)
        assertFalse("no speed yet, no comparison", t.any { it.contains("your typing speed") })
        UiTestKit.assertTouchTargets(a, "insights")
    }

    @Test fun showsTheRecordedStatsAndRefreshesOnReturn() {
        val now = System.currentTimeMillis()
        app.insights.record(now, "one two three four five six", 6_000, "com.example.chat")
        val c = launch()
        val a = settle(c.get()) { "TOTALS" in texts(it) }
        assertEquals("6", value(a, "Words dictated"))
        assertEquals("6", value(a, "Last 7 days"))
        assertEquals("1 dictation", value(a, "Dictations"))
        assertEquals("60 words/min", value(a, "Average speed"))
        assertEquals("1 day", value(a, "Current streak"))
        assertTrue(texts(a).toString(), "com.example.chat" in texts(a))
        // The stat row (serif numbers, one TalkBack stop each) and how it compares with typing.
        val cds = walk(a.window.decorView).mapNotNull { it.contentDescription?.toString() }
        assertTrue(cds.toString(), "6 words" in cds && "60 wpm" in cds && "1 day streak" in cds)
        assertTrue(texts(a).toString(), "About 1.5× your typing speed" in texts(a))
        assertTrue(cds.toString(), "com.example.chat, 6 words" in cds)
        // Another dictation while the page is in the background: shown when it comes back.
        c.pause()
        app.insights.record(now, "seven eight", 2_000, "com.example.chat")
        c.resume()
        settle(a) { value(it, "Words dictated") == "8" }
        assertEquals("8", value(a, "Words dictated"))
        assertEquals("2 dictations", value(a, "Dictations"))
    }

    @Test fun clearingHistoryKeepsTheStats() {
        app.history.add(HistoryRow(0, System.currentTimeMillis(), "com.mail", "RAW_TRANSCRIPT", "old note here", "old note here", true, 3_000), Retention.KEEP)
        val a = settle(launch().get()) { "TOTALS" in texts(it) }
        assertEquals("backfilled from History", "3", value(a, "Words dictated"))
        app.history.clear()
        val b = settle(launch().get()) { "TOTALS" in texts(it) }
        assertEquals("3", value(b, "Words dictated"))
    }

    @Test fun resetAsksFirstThenClears() {
        app.insights.record(System.currentTimeMillis(), "a b c", 1_000, null)
        val a = settle(launch().get()) { "TOTALS" in texts(it) }
        fun reset() = walk(a.window.decorView).first { it is android.widget.Button && it.text.toString() == "Reset stats" }
        reset().performClick(); idle()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_NEGATIVE).performClick(); idle()
        assertEquals("3", value(a, "Words dictated"))
        reset().performClick(); idle()
        ShadowAlertDialog.getLatestAlertDialog().getButton(DialogInterface.BUTTON_POSITIVE).performClick()
        settle(a) { value(it, "Words dictated") == "0" }
        assertEquals("0", value(a, "Words dictated"))
        assertFalse(texts(a).contains("Unknown app"))
    }
}
