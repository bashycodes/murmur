package dev.murmur.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.util.TimeZone

/** The per-day Insights aggregate on disk: record, one-time backfill, reset, local days. */
class InsightsStoreTest {
    @get:Rule val tmp = TemporaryFolder()

    private val berlin = TimeZone.getTimeZone("Europe/Berlin")
    private var history = mutableListOf<InsightsStore.Row>()
    private var historyCalls = 0
    private var historyFails = false

    private fun store(file: File = File(tmp.root, "insights.json"), zone: TimeZone = berlin) = InsightsStore(file, { zone }) {
        historyCalls++
        if (historyFails) error("db locked")
        history.toList()
    }

    /** 2026-10-28 at [h]:[m] local Berlin time (CET, UTC+1 after the 25 October switch). */
    private fun at(day: Int, h: Int, m: Int = 0): Long =
        java.time.ZonedDateTime.of(2026, 10, day, h, m, 0, 0, berlin.toZoneId()).toInstant().toEpochMilli()

    @Test fun wordCountSplitsOnAnyWhitespaceLikeWindows() {
        assertEquals(0, Insights.words(""))
        assertEquals(0, Insights.words(" \n\t "))
        assertEquals(2, Insights.words("  hello   world \n"))
        assertEquals(3, Insights.words("one\ttwo three"))
        assertEquals(2, Insights.words("- ok"))
    }

    @Test fun recordCountsWordsAudioAndAppOnTheLocalDay() {
        val s = store()
        assertTrue(s.record(at(28, 0, 30), "hello there world", 3_000, "com.slack"))
        assertFalse("empty transcript is not a dictation", s.record(at(28, 9), "   ", 1_000, "com.slack"))
        s.record(at(28, 23, 50), "two words", 1_000, null)
        val day = s.load().days.getValue("2026-10-28")
        assertEquals(5L, day.words); assertEquals(4_000L, day.ms); assertEquals(2L, day.count)
        assertEquals(mapOf("" to 2L, "com.slack" to 3L), day.apps)
        // 00:30 Berlin is still the 27th in UTC: the local day wins.
        assertEquals(setOf("2026-10-28"), s.load().days.keys)
        val sum = s.summary(at(28, 23, 59))
        assertEquals(5L, sum.totalWords)
        assertEquals(1, sum.currentStreak)
    }

    @Test fun persistsAcrossInstancesInTheCoreJsonShape() {
        val f = File(tmp.root, "insights.json")
        store(f).record(at(28, 10), "a b c", 1_000, "x")
        val j = org.json.JSONObject(f.readText())
        assertTrue(j.getBoolean("backfilled"))
        assertEquals(3, j.getJSONObject("days").getJSONObject("2026-10-28").getInt("words"))
        assertEquals(3, j.getJSONObject("days").getJSONObject("2026-10-28").getJSONObject("apps").getInt("x"))
        assertEquals(3L, store(f).summary(at(28, 11)).totalWords)
    }

    @Test fun backfillsFromHistoryOnceThenCountsOnlyNewDictations() {
        history += InsightsStore.Row(at(27, 9), "com.mail", "four words right here", 2_000)
        history += InsightsStore.Row(at(26, 9), "com.mail", "two words", 1_000)
        val s = store()
        assertEquals(6L, s.summary(at(28, 8)).totalWords)
        // Streak survives: nothing today yet, but yesterday and the day before had dictation.
        assertEquals(2, s.summary(at(28, 8)).currentStreak)
        s.record(at(28, 9), "one more", 500, "com.chat")
        // The live dictation also lands in History; a second instance must not re-add History.
        history += InsightsStore.Row(at(28, 9), "com.chat", "one more", 500)
        val again = store()
        assertEquals(8L, again.summary(at(28, 10)).totalWords)
        assertEquals(3, again.summary(at(28, 10)).currentStreak)
        assertEquals(1, historyCalls)
    }

    @Test fun backfillFailureStillMarksDoneSoNothingIsCountedTwice() {
        historyFails = true
        val s = store()
        assertEquals(0L, s.summary(at(28, 8)).totalWords)
        assertTrue(s.load().backfilled)
        historyFails = false
        history += InsightsStore.Row(at(27, 9), null, "old words", 1_000)
        assertEquals(0L, store().summary(at(28, 8)).totalWords)
    }

    @Test fun clearingHistoryKeepsTheStats() {
        history += InsightsStore.Row(at(27, 9), "com.mail", "kept words", 1_000)
        val s = store()
        s.record(at(28, 9), "and these", 1_000, "com.mail")
        history.clear()
        assertEquals(4L, store().summary(at(28, 10)).totalWords)
    }

    @Test fun resetClearsAndDoesNotBringHistoryBack() {
        history += InsightsStore.Row(at(27, 9), "com.mail", "old words", 1_000)
        val s = store()
        s.record(at(28, 9), "new", 1_000, null)
        s.reset()
        val sum = store().summary(at(28, 10))
        assertEquals(0L, sum.totalWords)
        assertEquals(0, sum.currentStreak)
        assertNull(sum.since)
    }

    @Test fun unreadableFileStartsEmptyAndBackfills() {
        val f = File(tmp.root, "insights.json").apply { writeText("{not json") }
        history += InsightsStore.Row(at(27, 9), null, "old words", 1_000)
        assertEquals(2L, store(f).summary(at(28, 8)).totalWords)
    }

    @Test fun streakFollowsTheDeviceZoneAcrossMidnight() {
        // 23:50 and 00:10 Berlin time: two local days (in UTC they are both on the 27th).
        val s = store()
        s.record(at(27, 23, 50), "late", 1_000, null)
        s.record(at(28, 0, 10), "early", 1_000, null)
        val sum = s.summary(at(28, 12))
        assertEquals(2 to 2, sum.currentStreak to sum.longestStreak)
        val utc = store(File(tmp.root, "utc.json"), TimeZone.getTimeZone("UTC"))
        utc.record(at(27, 23, 50), "late", 1_000, null)
        utc.record(at(28, 0, 10), "early", 1_000, null)
        assertEquals(setOf("2026-10-27"), utc.load().days.keys)
    }

    @Test fun streakAcrossDayBoundaries() {
        val s = store()
        s.record(at(26, 12), "a", 0, null)
        s.record(at(27, 12), "b", 0, null)
        // Today (28th) empty: yesterday keeps the streak alive.
        assertEquals(2, s.summary(at(28, 23, 59)).currentStreak)
        // On the 29th yesterday (28th) is empty: broken.
        val next = s.summary(at(29, 0, 1))
        assertEquals(0 to 2, next.currentStreak to next.longestStreak)
        s.record(at(29, 0, 2), "c", 0, null)
        assertEquals(1 to 2, s.summary(at(29, 0, 3)).let { it.currentStreak to it.longestStreak })
    }
}
