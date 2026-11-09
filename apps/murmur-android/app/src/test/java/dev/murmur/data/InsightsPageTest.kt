package dev.flowclone.data

import org.junit.Assert.assertEquals
import org.junit.Test
import java.util.Locale

/** Insights page wording (numbers, durations, streaks, chart labels). */
class InsightsPageTest {
    private val en = Locale.US

    @Test fun counts() {
        assertEquals("0", InsightsPage.number(0, en))
        assertEquals("12,345", InsightsPage.number(12_345, en))
        assertEquals("1 day", InsightsPage.days(1))
        assertEquals("0 days", InsightsPage.days(0))
        assertEquals("4 days", InsightsPage.days(4))
        assertEquals("1 dictation", InsightsPage.dictations(1, en))
        assertEquals("1,200 dictations", InsightsPage.dictations(1_200, en))
    }

    @Test fun speed() {
        assertEquals("No recordings yet", InsightsPage.wpm(null))
        assertEquals("150 words/min", InsightsPage.wpm(150.0))
        assertEquals("43 words/min", InsightsPage.wpm(42.6))
    }

    @Test fun speedComparedWithTyping() {
        assertEquals(null, InsightsPage.speedVsTyping(null, 40.0))
        assertEquals(null, InsightsPage.speedVsTyping(42.0, 40.0))
        assertEquals("About 1.5× your typing speed", InsightsPage.speedVsTyping(60.0, 40.0))
        assertEquals("About 2× your typing speed", InsightsPage.speedVsTyping(80.0, 40.0))
        assertEquals("About 3× your typing speed", InsightsPage.speedVsTyping(136.0, 40.0))
        assertEquals(null, InsightsPage.speedVsTyping(60.0, 0.0))
    }

    @Test fun appShare() {
        assertEquals(1f, InsightsPage.share(50, 50))
        assertEquals(0.5f, InsightsPage.share(25, 50))
        assertEquals(0f, InsightsPage.share(5, 0))
    }

    @Test fun savedTime() {
        assertEquals("None yet", InsightsPage.saved(0.0))
        assertEquals("Under a minute", InsightsPage.saved(0.4))
        assertEquals("1 min", InsightsPage.saved(1.1))
        assertEquals("59 min", InsightsPage.saved(59.4))
        assertEquals("1 h", InsightsPage.saved(60.2))
        assertEquals("2 h 5 min", InsightsPage.saved(125.0))
        assertEquals("Compared with typing at 40 words/min", InsightsPage.savedHint(40.0))
    }

    @Test fun chartLabelsAreWeekdaysWithTodayLast() {
        // 2026-10-22 is a Thursday.
        val last7 = (22..28).map { Insights.DayWords("2026-10-$it", if (it == 28) 12L else 0L) }
        assertEquals(listOf("Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Today"), InsightsPage.chartLabels(last7, en))
        assertEquals(
            "Words per day, last 7 days: Thu 0, Fri 0, Sat 0, Sun 0, Mon 0, Tue 0, Today 12",
            InsightsPage.chartDescription(last7, en),
        )
    }

    @Test fun appNamesFallBackToThePackage() {
        assertEquals("Unknown app", InsightsPage.appName("") { null })
        assertEquals("com.x", InsightsPage.appName("com.x") { null })
        assertEquals("Slack", InsightsPage.appName("com.slack") { "Slack" })
        assertEquals("com.y", InsightsPage.appName("com.y") { " " })
    }
}
