package dev.flowclone.learn

import dev.flowclone.core.FieldContext
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
        val f = "Dear Bob, I use whisper flow daily. Thanks"
        assertEquals("I use whisper flow daily.", EditText.region(f, a("Dear Bob, ", " Thanks"), 30))
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

    @Test fun floCloneEditThenTimeoutYieldsTheCorrection() {
        val w = EditWatch("I use local whisper daily. ", a("Hi. ", ""), 0, fast.copy(stableMs = 60_000))
        assertNull(w.observe(0, t("Hi. I use local whisper daily. ")))
        assertNull(w.observe(1_000, t("Hi. I use Murmur daily.")))
        assertNull(w.observe(2_000, t("Hi. I use Murmur daily.")))
        assertEquals(Outcome(EndReason.TIMEOUT, "I use Murmur daily."), w.observe(10_000, t("Hi. I use Murmur daily.")))
    }

    @Test fun anEditThatStopsChangingEndsTheWatchEarly() {
        val w = EditWatch("local whisper", a("", ""), 0, fast)
        assertNull(w.observe(0, t("local whisper")))
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

    @Test fun sentOrDeletedTextDoesNotCount() {
        val w = EditWatch("ship it", a("", ""), 0, fast)
        w.observe(0, t("ship it"))
        assertEquals(Outcome(EndReason.REGION_REMOVED, null), w.observe(800, t("")))
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
        assertTrue(EndReason.TIMEOUT.counts && EndReason.FOCUS_CHANGED.counts && EndReason.STABLE.counts)
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
            { reads++; if (reads == 2) enabled = false; t(if (reads == 1) "local whisper" else "Murmur") },
            EditWatch("local whisper", a("", ""), 0, fast), { now }, { now += it }, stillWanted = { enabled },
        )
        assertEquals(Outcome(EndReason.CANCELLED, null), o)
        assertEquals("no read after the switch went off", 2, reads)
    }

    @Test fun boundEditDropsTextTypedAroundTheDictation() {
        assertEquals("I use Wispr.", EditText.boundEdit("I use wispr", "I use Wispr. How are you today"))
        assertEquals("leading insertions trimmed, inner ones kept", "meet at 12 noon", EditText.boundEdit("meet at noon", "Ok so meet at 12 noon"))
        assertEquals("", EditText.boundEdit("x", ""))
    }
}
