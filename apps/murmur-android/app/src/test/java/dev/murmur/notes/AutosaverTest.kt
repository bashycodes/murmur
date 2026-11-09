package dev.murmur.notes

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Debounced autosave: one write after typing pauses, flush on pause, nothing for unchanged text. */
@OptIn(ExperimentalCoroutinesApi::class)
class AutosaverTest {
    private val writes = mutableListOf<String>()

    @Test fun writesOnceAfterTheEditsPause() = runTest {
        val a = Autosaver(this, initial = "", delayMs = 1_000) { writes += it }
        a.edited("H"); advanceTimeBy(400)
        a.edited("He"); advanceTimeBy(400)
        a.edited("Hello"); advanceTimeBy(999); runCurrent()
        assertEquals(emptyList<String>(), writes)
        assertTrue(a.pending)
        advanceTimeBy(2); runCurrent()
        assertEquals(listOf("Hello"), writes)
        assertFalse(a.pending)
    }

    @Test fun flushWritesThePendingTextAtOnce() = runTest {
        val a = Autosaver(this, initial = "", delayMs = 1_000) { writes += it }
        a.edited("dictated text")
        a.flush(); runCurrent()
        assertEquals(listOf("dictated text"), writes)
        // The cancelled debounce does not write it a second time.
        advanceTimeBy(5_000); runCurrent()
        assertEquals(listOf("dictated text"), writes)
    }

    @Test fun unchangedTextIsNotWritten() = runTest {
        val a = Autosaver(this, initial = "loaded", delayMs = 1_000) { writes += it }
        a.edited("loaded"); advanceTimeBy(2_000); runCurrent()
        a.flush(); runCurrent()
        assertEquals(emptyList<String>(), writes)
        a.edited("loaded!"); a.edited("loaded"); advanceTimeBy(2_000); runCurrent()
        assertEquals(emptyList<String>(), writes)
    }

    @Test fun eachPauseWritesTheLatestTextOnce() = runTest {
        val a = Autosaver(this, initial = "", delayMs = 500) { writes += it }
        a.edited("one"); advanceTimeBy(600); runCurrent()
        a.edited("one two"); advanceTimeBy(600); runCurrent()
        a.flush(); runCurrent()
        assertEquals(listOf("one", "one two"), writes)
    }

    @Test fun aFailedWriteIsRetriedOnTheNextSave() = runTest {
        var fail = true
        val a = Autosaver(this, initial = "", delayMs = 500) { if (fail) error("disk full") else writes += it }
        a.edited("keep me"); advanceTimeBy(600); runCurrent()
        assertEquals(emptyList<String>(), writes)
        fail = false
        a.flush(); runCurrent()
        assertEquals(listOf("keep me"), writes)
    }
}
