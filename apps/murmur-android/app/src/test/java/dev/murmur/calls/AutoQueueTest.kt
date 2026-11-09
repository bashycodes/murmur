package dev.murmur.calls

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

/** D49: opt-in floor, queue once, never twice, outcomes, persistence — against a fake media index. */
class AutoQueueTest {
    private val dir: File = Files.createTempDirectory("auto").toFile()
    private val file = File(dir, "calls-auto.json")

    @After fun cleanup() { dir.deleteRecursively() }

    /** A fake media index: synthetic recordings, a generation counter, and how often it was asked. */
    private class FakeIndex : RecordingSource {
        val recs = ArrayList<Recording>()
        var gen = 0L
        var queries = 0
        var denied = false
        fun add(name: String): Recording {
            gen++
            return Recording(1000 + gen, "content://media/external_primary/audio/media/${1000 + gen}", name, gen * 1000, gen).also { recs += it }
        }
        override fun generation() = gen
        override fun since(after: Long): List<Recording> {
            queries++
            if (denied) throw SecurityException("no READ_MEDIA_AUDIO")
            return recs.filter { it.generation > after }
        }
    }

    private val idx = FakeIndex()
    private fun queue() = AutoQueue(file, idx, now = { 42 })

    @Test fun offQueuesNothingAndDoesNotAsk() {
        idx.add("fake-1.m4a")
        val q = queue()
        assertFalse(q.isEnabled)
        assertEquals(0, q.scan())
        assertEquals(0, idx.queries)
    }

    @Test fun onlyRecordingsAfterTheOptInAreQueued() {
        repeat(5) { idx.add("old-$it.m4a") } // the backlog
        val q = queue()
        q.enable()
        assertEquals("enabling reads the generation only", 0, idx.queries)
        assertEquals(0, q.scan())
        val n = idx.add("new.m4a")
        assertEquals(1, q.scan())
        assertEquals(0, q.scan()) // a second change: nothing new
        val r = q.next("he")!!
        assertEquals(n.uri, r.uri); assertEquals(n.id, r.autoId); assertEquals("he", r.language); assertEquals(n.recordedAt, r.recordedAt)
        assertNull(q.next("he")) // one at a time: no other queued
    }

    @Test fun detectionIsOneQueryPerChangeNoPolling() {
        val q = queue(); q.enable()
        idx.add("a.m4a"); idx.add("b.m4a")
        q.scan()
        assertEquals(1, idx.queries)
        assertEquals(2, q.pending())
        // Taking jobs and finishing them never touches the index.
        val a = q.next("en")!!
        q.finished(a.autoId!!, CallJobState.Done("t1", a.name), null)
        assertEquals(1, idx.queries)
    }

    @Test fun doneIsNeverRedoneAndOldestGoesFirst() {
        val q = queue(); q.enable()
        val a = idx.add("a.m4a"); val b = idx.add("b.m4a")
        q.scan()
        val first = q.next("en")!!
        assertEquals(a.id, first.autoId)
        q.finished(a.id, CallJobState.Done("t1", "a.m4a"), null)
        q.scan()
        assertEquals(b.id, q.next("en")!!.autoId)
        q.finished(b.id, CallJobState.Done("t2", "b.m4a"), null)
        q.scan()
        assertNull(q.next("en"))
        assertEquals(0, q.pending())
    }

    @Test fun failuresAreListedRetriedAndDismissed() {
        val q = queue(); q.enable()
        val a = idx.add("a.m4a"); q.scan()
        q.next("en")
        q.finished(a.id, CallJobState.Failed("a.m4a", "no audio track"), null)
        assertEquals(listOf("no audio track"), q.failures().map { it.error })
        assertNull(q.next("en"))
        q.retry(a.id)
        assertTrue(q.failures().isEmpty())
        assertEquals(a.id, q.next("en")!!.autoId)
        q.finished(a.id, CallJobState.Cancelled("a.m4a"), CallStop.OWNER)
        assertEquals("Cancelled", q.failures().single().error)
        q.dismiss(a.id)
        assertTrue(q.failures().isEmpty())
        q.scan()
        assertNull("dismissed stays seen", q.next("en"))
    }

    @Test fun preemptedGoesBackWithoutCountingAndStopsGiveUp() {
        val q = queue(); q.enable()
        val a = idx.add("a.m4a"); q.scan()
        repeat(10) {
            q.next("en")
            q.finished(a.id, CallJobState.Cancelled("a.m4a"), CallStop.PREEMPTED)
        }
        assertEquals(1, q.pending())
        repeat(AutoQueue.MAX_ATTEMPTS) {
            assertTrue(q.next("en") != null)
            q.finished(a.id, CallJobState.Cancelled("a.m4a"), CallStop.SYSTEM)
        }
        assertNull(q.next("en"))
        assertTrue(q.failures().single().error!!.startsWith("Stopped"))
    }

    @Test fun aJobThatNeverEndedCountsAsAnAttempt() {
        val q = queue(); q.enable()
        idx.add("a.m4a"); q.scan()
        // Taken, then the process died: a new AutoQueue on the same file (app restart).
        repeat(AutoQueue.MAX_ATTEMPTS) { assertTrue(queue().next("en") != null) }
        assertNull(queue().next("en"))
    }

    @Test fun optInAndHistorySurviveARestart() {
        val q = queue(); q.enable()
        val a = idx.add("a.m4a"); q.scan(); q.next("en")
        q.finished(a.id, CallJobState.Done("t1", "a.m4a"), null)
        val again = queue()
        assertTrue(again.isEnabled)
        again.scan()
        assertNull(again.next("en"))
        idx.add("b.m4a")
        assertEquals(1, again.scan())
    }

    @Test fun noPermissionIsShownAndClearsWhenGranted() {
        val q = queue(); q.enable()
        idx.add("a.m4a")
        idx.denied = true
        assertEquals(0, q.scan())
        assertEquals(AutoQueue.NO_ACCESS, q.lastScanError)
        assertEquals(AutoQueue.NO_ACCESS, queue().lastScanError)
        idx.denied = false
        assertEquals(1, q.scan())
        assertNull(q.lastScanError)
    }

    @Test fun offDropsTheQueueAndOnAgainStartsFromNow() {
        val q = queue(); q.enable()
        idx.add("a.m4a"); q.scan()
        q.disable()
        assertFalse(queue().isEnabled)
        assertEquals(0, q.pending())
        q.enable()
        assertEquals(0, q.scan()) // a.m4a predates the new opt-in
        idx.add("b.m4a")
        assertEquals(1, q.scan())
    }

    @Test fun finishedHistoryIsBoundedButStillNeverRequeued() {
        val q = queue(); q.enable()
        repeat(AutoQueue.KEEP + 20) {
            idx.add("r$it.m4a"); q.scan()
            val r = q.next("en")!!
            q.finished(r.autoId!!, CallJobState.Done("t$it", r.name), null)
        }
        assertTrue(file.length() < 200_000)
        assertEquals(0, q.scan())
        assertNull(queue().next("en"))
    }
}
