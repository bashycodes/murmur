package dev.flowclone.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDateTime
import java.time.ZoneId

/** History list (grouped by day, one line per dictation) and its detail page. */
class HistoryPagesTest {
    private val zone = ZoneId.of("Europe/Berlin")
    private fun at(y: Int, mo: Int, d: Int, h: Int, mi: Int = 0) = LocalDateTime.of(y, mo, d, h, mi).atZone(zone).toInstant().toEpochMilli()
    private val now = at(2026, 10, 24, 15)

    private fun row(id: Long, t: Long, text: String = "hello there", inserted: Boolean = true, status: String = "RAW_TRANSCRIPT", app: String? = "com.whatsapp") =
        HistoryRow(id, t, app, status, text, text, inserted, 2_100, asrMs = 830, llmMs = 0, totalMs = 1_400)

    @Test fun groupsByCalendarDayNewestFirst() {
        val rows = listOf(
            row(1, at(2026, 10, 24, 14)), row(2, at(2026, 10, 24, 0, 5)),
            row(3, at(2026, 10, 23, 23, 59)),
            row(4, at(2026, 10, 20, 9)),
            row(5, at(2026, 9, 1, 9)),
            row(6, at(2025, 12, 31, 9)),
        )
        val g = HistoryPages.groupByDay(rows, now, zone)
        assertEquals(listOf("Today", "Yesterday", "Tuesday", "Tue, Sep 1", "Wed, Dec 31, 2025"), g.map { it.label })
        assertEquals(listOf(1L, 2L), g[0].rows.map { it.id })
        assertEquals(listOf(3L), g[1].rows.map { it.id })
        assertEquals(6, g.sumOf { it.rows.size })
    }

    @Test fun emptyListHasNoGroups() {
        assertEquals(emptyList<HistoryPages.Group>(), HistoryPages.groupByDay(emptyList(), now, zone))
    }

    @Test fun rowTitleIsTheFirstLine() {
        assertEquals("Dear Sam,", HistoryPages.title(row(1, now, "Dear Sam,\n\nThe slides are attached."), raw = false))
        assertEquals("(no speech)", HistoryPages.title(row(1, now, "", status = "EMPTY"), raw = false))
        assertEquals("first", HistoryPages.title(row(1, now, "\n  first\nsecond"), raw = false))
    }

    @Test fun metaLineHasTimeAndTargetApp() {
        assertEquals("14:00 · com.whatsapp", HistoryPages.meta(row(1, now), "14:00"))
        assertEquals("14:00 · Unknown app", HistoryPages.meta(row(1, now, app = null), "14:00"))
    }

    @Test fun badgeIsTheEngineOutcome() {
        assertEquals("Transcript", HistoryPages.badge(row(1, now)))
        assertEquals("AI cleaned", HistoryPages.badge(row(1, now, status = "FORMATTED")))
        assertEquals("Error", HistoryPages.badge(row(1, now, status = "nonsense")))
    }

    /** The recovery state: text that was dictated but never reached the field. */
    @Test fun notInsertedRowsOfferCopy() {
        val r = row(1, now, inserted = false)
        assertTrue(HistoryPages.notInserted(r))
        assertEquals("Not inserted — tap to copy", HistoryPages.flag(r))
        assertFalse(HistoryPages.notInserted(row(2, now)))
        assertNull(HistoryPages.flag(row(2, now)))
        // Nothing to recover from an empty or failed dictation.
        assertFalse(HistoryPages.notInserted(row(3, now, "", inserted = false, status = "EMPTY")))
        assertFalse(HistoryPages.notInserted(row(4, now, "", inserted = false, status = "ERROR")))
    }

    /** fix/route-order stores the insert outcome; the detail page says why it did not go in. */
    @Test fun reasonNamesTheInsertOutcome() {
        val r = row(1, now, inserted = false)
        assertEquals("You switched apps", HistoryPages.reason(r.copy(insertOutcome = "target_changed")))
        assertEquals("Protected field", HistoryPages.reason(r.copy(insertOutcome = "target_protected")))
        assertEquals("Couldn't paste", HistoryPages.reason(r.copy(insertOutcome = "paste_failed")))
        assertNull("rows saved before the outcome was stored", HistoryPages.reason(r))
        assertNull(HistoryPages.reason(r.copy(insertOutcome = "something_new")))
        assertNull("inserted rows need no reason", HistoryPages.reason(row(2, now).copy(insertOutcome = "inserted")))
    }

    @Test fun badgeIsPrefixedWithTheRouteWhenKnown() {
        assertEquals("Phone · Transcript", HistoryPages.badge(row(1, now).copy(route = "local")))
        assertEquals("Server · AI cleaned", HistoryPages.badge(row(1, now, status = "FORMATTED").copy(route = "remote")))
        assertEquals("Cloud · Transcript", HistoryPages.badge(row(1, now).copy(route = "cloud")))
        assertEquals("unknown route: no prefix", "Transcript", HistoryPages.badge(row(1, now).copy(route = "warp")))
        assertEquals("Transcript", HistoryPages.badge(row(1, now)))
    }

    /** History copy == what the "Dictation not inserted" notification copies ([CommitResult.historyText]). */
    @Test fun copyTextIsTheSavedText() {
        val r = row(1, now, "1. eggs\n2. milk", inserted = false).copy(asrText = "first eggs second milk")
        assertEquals("1. eggs\n2. milk", HistoryPages.copyText(r))
        assertEquals("first eggs second milk", HistoryPages.copyText(r.copy(finalText = "")))
    }

    @Test fun timingsLineForTheLatencyChecklist() {
        assertEquals("2.1 s audio · asr 830 ms · total 1400 ms", HistoryPages.timings(row(1, now)))
        val r = row(1, now).copy(llmMs = 410, fallbackReason = "llm_timeout")
        assertEquals("2.1 s audio · asr 830 ms · llm 410 ms · total 1400 ms · AI too slow, formatted text used", HistoryPages.timings(r))
        assertEquals("", HistoryPages.timings(row(1, now).copy(audioMs = 0, asrMs = 0, totalMs = 0)))
    }

    @Test fun summaryAndRetention() {
        assertEquals("No dictations saved · audio is never stored", HistoryPages.summary(0))
        assertEquals("1 dictation · audio is never stored", HistoryPages.summary(1))
        assertEquals("12 dictations · audio is never stored", HistoryPages.summary(12))
    }

    @Test fun emptyStates() {
        assertEquals("No dictations yet", HistoryPages.empty(Retention.KEEP, "", 0).title)
        assertTrue(HistoryPages.empty(Retention.NEVER, "", 0).hint, "Never save" in HistoryPages.empty(Retention.NEVER, "", 0).hint)
        assertEquals("Nothing matches “bus”", HistoryPages.empty(Retention.KEEP, "bus", 3).title)
    }
}
