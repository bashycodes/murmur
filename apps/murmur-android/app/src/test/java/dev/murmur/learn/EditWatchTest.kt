package dev.murmur.learn

import dev.murmur.core.FieldContext
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EditWatchTest {
    private val fast = WatchOpts(windowMs = 10_000, baselineMs = 2_000, pollMs = 500, stableMs = 3_000)
    private fun a(before: String, after: String) = Anchors(before, after)
    private fun t(s: String) = FieldRead.Text(s)

    @Test fun regionBetweenAnchors() {
        val f = "Dear Bob, I use mer mur daily. Thanks"
        assertEquals("I use mer mur daily.", EditText.region(f, a("Dear Bob, ", " Thanks"), 30))
        assertNull(EditText.region(f, a("Dear Alice, ", ""), 10))
        assertEquals("empty anchors = whole field", "hello", EditText.region("hello", a("", ""), 5))
    }

    @Test fun repeatedAnchorsPickTheClosestLength() {
        assertEquals("hi", EditText.region("x. hi there. x. hi. x.", a("x. ", "."), 2))
    }

    @Test fun anchorsComeFromTheCaretContext() {
        val an = Anchors.of(FieldContext(before = "a".repeat(100) + "END", after = "START" + "b".repeat(100)))
        assertEquals(Anchors.CHARS, an.before.length)
        assertTrue(an.before.endsWith("END"))
        assertTrue(an.after.startsWith("START"))
    }

    @Test fun brandNameEditThenTimeoutYieldsTheCorrection() {
        val w = EditWatch("I use mer mur daily. ", a("Hi. ", ""), 0, fast.copy(stableMs = 60_000))
        assertNull(w.observe(0, t("Hi. I use mer mur daily. ")))
        assertNull(w.observe(1_000, t("Hi. I use Murm daily.")))
        assertNull(w.observe(2_000, t("Hi. I use Murmur daily.")))
        assertEquals(Outcome(EndReason.TIMEOUT, "I use Murmur daily."), w.observe(10_000, t("Hi. I use Murmur daily.")))
    }

    @Test fun anEditThatStopsChangingEndsTheWatchEarly() {
        val w = EditWatch("mer mur", a("", ""), 0, fast)
        assertNull(w.observe(0, t("mer mur")))
        assertNull(w.observe(1_000, t("Murmur")))
        assertNull(w.observe(2_000, t("Murmur")))
        assertNull("not yet stable for 3 s", w.observe(3_500, t("Murmur")))
        assertEquals(Outcome(EndReason.STABLE, "Murmur"), w.observe(4_000, t("Murmur")))
    }

    @Test fun anUntouchedInsertNeverStabilisesEarlyAndLearnsNothing() {
        val w = EditWatch("all good ", a("", ""), 0, fast)
        assertNull(w.observe(0, t("all good")))
        assertNull("same as inserted: no early end", w.observe(5_000, t("all good")))
        assertEquals("whitespace differences are not edits", Outcome(EndReason.TIMEOUT, null), w.observe(10_000, t("all  good\n")))
    }

    @Test fun focusChangeKeepsTheLastObservation() {
        val w = EditWatch("teh plan", a("", ""), 0, fast)
        w.observe(0, t("teh plan"))
        w.observe(500, FieldRead.Unavailable)
        w.observe(1_000, t("the plan"))
        assertEquals(Outcome(EndReason.FOCUS_CHANGED, "the plan"), w.observe(1_500, FieldRead.FocusChanged))
    }

    @Test fun focusChangeBeforeAnyReadLearnsNothing() {
        // A field that turned protected answers FocusChanged before its text is ever read.
        val w = EditWatch("secret", a("", ""), 0, fast)
        assertEquals(Outcome(EndReason.FOCUS_CHANGED, null), w.observe(0, FieldRead.FocusChanged))
    }

    @Test fun aClearWithoutAFixLearnsNothing() {
        val w = EditWatch("ship it", a("", ""), 0, fast)
        w.observe(0, t("ship it"))
        assertEquals("the clear itself is not an edit", Outcome(EndReason.CLEARED, null), w.observe(800, t(" \n")))
    }

    @Test fun deletingOnlyTheDictationDoesNotCount() {
        val w = EditWatch("ship it", a("Note: ", ""), 0, fast)
        w.observe(0, t("Note: ship it"))
        assertEquals(Outcome(EndReason.REGION_REMOVED, null), w.observe(800, t("Note: ")))
    }

    @Test fun aClearKeepsTheFixSeenBeforeIt() {
        // The anchors are gone once the field empties: still a clear, not an anchor mismatch.
        val w = EditWatch("teh plan", a("Re: ", " ok"), 0, fast)
        w.observe(0, t("Re: teh plan ok"))
        w.observe(250, t("Re: the plan ok"))
        assertEquals(Outcome(EndReason.CLEARED, "the plan"), w.observe(500, t("")))
    }

    @Test fun backspacingToEmptyIsNotAFix() {
        val w = EditWatch("see you at noon", a("", ""), 0, fast)
        w.observe(0, t("see you at noon"))
        w.observe(250, t("see you at no"))
        w.observe(500, t("see you"))
        assertEquals(Outcome(EndReason.CLEARED, null), w.observe(750, t("")))
    }

    /** Text-changed events at fixed virtual times; pending ones answer at once. */
    private class Events(private val clock: LongArray, times: List<Long>) : FieldChanges {
        private val left = ArrayDeque(times)
        override suspend fun await(timeoutMs: Long): Boolean {
            val now = clock[0]
            if (left.firstOrNull()?.let { it <= now } == true) { while (left.firstOrNull()?.let { it <= now } == true) left.removeFirst(); return true }
            val next = left.firstOrNull()
            if (next != null && next <= now + timeoutMs) { left.removeFirst(); clock[0] = next; return true }
            clock[0] = now + timeoutMs
            return false
        }
        override fun close() = Unit
    }

    /**
     * Default timings, the field as a function of time, a text-changed event at each [events]
     * time, focus leaving at [focusMs]. Returns the outcome and the read times.
     */
    private fun timed(text: (Long) -> String, events: List<Long>, focusMs: Long = Long.MAX_VALUE): Pair<Outcome, List<Long>> = runBlocking {
        val clock = longArrayOf(0)
        val reads = ArrayList<Long>()
        val o = runWatch(
            { val n = clock[0]; reads += n; clock[0] = n + 20; if (n >= focusMs) FieldRead.FocusChanged else t(text(n)) },
            EditWatch("I use cora daily", a("", ""), 0), { clock[0] }, { clock[0] += it },
            changes = Events(clock, events),
        )
        assertTrue("reads ≥ 250 ms apart: $reads", reads.zipWithNext().all { (x, y) -> y - x >= 250 })
        o to reads
    }

    @Test fun aQuickFixThenClearIsLearned() {
        // Fixed at 1.05 s (keystrokes at 1.0 / 1.05), sent 290 ms later; the 1 s timer alone
        // would read at 1.0 and 2.0 and miss it.
        val (o, _) = timed({ n -> when { n < 1_050 -> "I use cora daily"; n < 1_340 -> "I use Korra daily"; else -> "" } }, listOf(1_000, 1_050, 1_340))
        assertEquals(Outcome(EndReason.CLEARED, "I use Korra daily"), o)
    }

    @Test fun aQuickFixThenFocusChangeIsLearned() {
        val (o, _) = timed({ n -> if (n < 1_050) "I use cora daily" else "I use Korra daily" }, listOf(1_050), focusMs = 1_300)
        assertEquals(Outcome(EndReason.FOCUS_CHANGED, "I use Korra daily"), o)
    }

    @Test fun noFixLearnsNothing() {
        val (cleared, _) = timed({ n -> if (n < 1_300) "I use cora daily" else "" }, listOf(1_300))
        assertEquals(Outcome(EndReason.CLEARED, null), cleared)
        val (left, _) = timed({ "I use cora daily" }, emptyList(), focusMs = 1_300)
        assertEquals(Outcome(EndReason.FOCUS_CHANGED, null), left)
    }

    @Test fun aBurstOfEventsIsThrottled() {
        // 40 keystrokes 10 ms apart: reads stay ≥ 250 ms apart (checked in timed) and few.
        val (_, reads) = timed({ n -> if (n < 400) "I use cora daily" else "I use Korra daily" }, (0 until 40).map { 100L + it * 10 }, focusMs = 1_000)
        assertTrue("$reads", reads.size <= 5)
    }

    @Test fun rewrittenSurroundingsAreAnAnchorMismatch() {
        val w = EditWatch("word", a("Hello ", " bye"), 0, fast)
        w.observe(0, t("Hello word bye"))
        assertEquals(Outcome(EndReason.ANCHOR_MISMATCH, null), w.observe(800, t("totally different")))
    }

    @Test fun insertMustAppearWithinTheBaseline() {
        val w = EditWatch("late text", a("", ""), 0, fast)
        assertNull(w.observe(0, t("")))
        assertNull(w.observe(1_000, t("other")))
        assertEquals(Outcome(EndReason.PASTE_NOT_OBSERVED, null), w.observe(2_000, t("other")))
    }

    @Test fun endReasonsThatCount() {
        assertTrue(EndReason.TIMEOUT.counts && EndReason.FOCUS_CHANGED.counts && EndReason.STABLE.counts && EndReason.CLEARED.counts)
        for (r in listOf(EndReason.ANCHOR_MISMATCH, EndReason.REGION_REMOVED, EndReason.PASTE_NOT_OBSERVED, EndReason.CANCELLED)) assertFalse(r.name, r.counts)
    }

    @Test fun runWatchPollsInVirtualTime() = runBlocking {
        var now = 0L
        var sleeps = 0
        val reads = ArrayDeque(listOf(t("Hi kubernetes"), t("Hi Kubernetes")))
        val o = runWatch({ reads.removeFirstOrNull() ?: FieldRead.FocusChanged }, EditWatch("Hi kubernetes", a("", ""), 0, fast), { now }, { sleeps++; now += it })
        assertEquals(Outcome(EndReason.FOCUS_CHANGED, "Hi Kubernetes"), o)
        assertEquals(2, sleeps)
    }

    @Test fun turningTheSettingOffStopsReadingAtOnce() = runBlocking {
        var now = 0L
        var reads = 0
        var enabled = true
        val o = runWatch(
            { reads++; if (reads == 2) enabled = false; t(if (reads == 1) "mer mur" else "Murmur") },
            EditWatch("mer mur", a("", ""), 0, fast), { now }, { now += it }, stillWanted = { enabled },
        )
        assertEquals(Outcome(EndReason.CANCELLED, null), o)
        assertEquals("no read after the switch went off", 2, reads)
    }

    @Test fun boundEditDropsTextTypedAroundTheDictation() {
        assertEquals("I use Tailscale.", EditText.boundEdit("I use tailscale", "I use Tailscale. How are you today"))
        assertEquals("leading insertions trimmed, inner ones kept", "meet at 12 noon", EditText.boundEdit("meet at noon", "Ok so meet at 12 noon"))
        assertEquals("", EditText.boundEdit("x", ""))
    }
}
