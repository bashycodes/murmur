package dev.flowclone.calls

import dev.flowclone.core.CoreException
import dev.flowclone.core.MeetingException
import dev.flowclone.core.MeetingJob
import dev.flowclone.core.MeetingProgress
import dev.flowclone.core.MeetingSegment
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.locks.ReentrantLock

/** D46 job runner: decode → own engine → transcript, cancel, failures, and dictation first. */
class CallJobsTest {
    private val dir: File = Files.createTempDirectory("calls").toFile()
    private val store = CallsStore(dir)

    @After fun cleanup() { dir.deleteRecursively() }

    /** [seconds] of 16 kHz mono in 1000-frame blocks. */
    private class Tone(val seconds: Int, val rate: Int = 16_000, val failAt: Int = -1) : AudioSource {
        var closed = false
        private var sent = 0
        override val durationMs: Long? = seconds * 1000L
        override fun next(): AudioBlock? {
            if (sent == failAt) throw DecodeException("bad frame")
            if (sent * 1000 >= seconds * rate) return null
            sent++
            return AudioBlock(FloatArray(1000) { 0.1f }, 1, rate)
        }
        override fun close() { closed = true }
    }

    /**
     * A meeting job whose every 8th second of audio ends a chunk that takes [chunkMs] of "CPU"
     * ([cpu], shared with the dictation in the priority test).
     */
    private class FakeJob(val chunkMs: Long = 0, val cpu: ReentrantLock = ReentrantLock(), val failOnPush: Int = -1) : MeetingJob {
        var samples = 0L; var total = 0L; var closed = false
        val pushes = AtomicInteger()
        @Volatile var cancelled = false
        val inChunk = AtomicBoolean(false)
        private var chunkStart = 0L
        override fun setTotalSamples(samples: Long) { total = samples }
        override fun push(pcm: ShortArray, length: Int): List<MeetingSegment> {
            if (cancelled) throw MeetingException("cancelled", MeetingException.CANCELLED)
            if (pushes.incrementAndGet() == failOnPush) throw MeetingException("3 chunks failed", "failed")
            samples += length
            if (samples - chunkStart >= 8 * 16_000) return chunk()
            return emptyList()
        }
        private fun chunk(): List<MeetingSegment> {
            cpu.lock(); inChunk.set(true)
            try { if (chunkMs > 0) Thread.sleep(chunkMs) } finally { inChunk.set(false); cpu.unlock() }
            val s = MeetingSegment(chunkStart / 16, samples / 16, "words ${chunkStart / 16_000}")
            chunkStart = samples
            return listOf(s)
        }
        override fun finish(): List<MeetingSegment> = if (samples > chunkStart) chunk() else emptyList()
        override fun progress() = MeetingProgress(doneMs = chunkStart / 16, receivedMs = samples / 16, totalMs = total / 16)
        override fun cancel() { cancelled = true }
        override fun close() { closed = true }
    }

    private fun jobs(src: () -> AudioSource, engine: (String) -> MeetingJob, busy: () -> Boolean = { false }, thread: Boolean = false) = CallJobs(
        store, { src() }, engine, now = { 1_700_000_000_000 }, yieldTo = busy, pause = { Thread.sleep(it.coerceAtMost(5)) },
        runner = if (thread) { r -> Thread(r).start() } else { r -> r.run() },
    )

    private val req = CallRequest("content://x/1", "call.m4a", 1_600_000_000_000, "he")

    private fun await(j: CallJobs) { repeat(1000) { if (!j.state.value.busy) return; Thread.sleep(10) }; error("still running") }

    @Test fun transcribesAndSavesTheTranscript() {
        val src = Tone(20); val job = FakeJob()
        val j = jobs({ src }, { lang -> assertEquals("he", lang); job })
        assertTrue(j.start(req))
        val done = j.state.value as CallJobState.Done
        val t = store.load(done.id)!!
        assertEquals(listOf("words 0", "words 8", "words 16"), t.segments.map { it.text })
        assertEquals(20_000L, t.durationMs)
        assertEquals(1_600_000_000_000, t.recordedAt)
        assertEquals("call.m4a", t.sourceName); assertEquals("he", t.language)
        assertEquals(320_000L, job.total)
        assertTrue(src.closed && job.closed)
    }

    @Test fun cancelEndsTheJobAndSavesNothing() {
        val job = FakeJob(chunkMs = 20)
        val j = jobs({ Tone(6000) }, { job }, thread = true)
        assertTrue(j.start(req))
        var n = 0
        while (job.pushes.get() <= 3 && n++ < 500) Thread.sleep(10)
        assertTrue(j.state.value.busy)
        j.cancel()
        await(j)
        assertEquals(CallJobState.Cancelled("call.m4a"), j.state.value)
        assertTrue(store.list().isEmpty())
        assertTrue(job.closed)
    }

    @Test fun failuresSaySoAndFreeEverything() {
        val src = Tone(5, failAt = 2)
        val j = jobs({ src }, { FakeJob() })
        j.start(req)
        assertEquals("bad frame", (j.state.value as CallJobState.Failed).message)
        assertTrue(src.closed)

        val k = jobs({ Tone(5) }, { throw CoreException("no model") })
        k.start(req)
        assertEquals("no model", (k.state.value as CallJobState.Failed).message)

        val job = FakeJob(failOnPush = 3)
        val m = jobs({ Tone(30) }, { job })
        m.start(req)
        assertTrue(m.state.value is CallJobState.Failed)
        assertTrue(job.closed)
        assertTrue(store.list().isEmpty())
        m.acknowledge()
        assertEquals(CallJobState.Idle, m.state.value)
    }

    @Test fun oneJobAtATime() {
        val hold = CountDownLatch(1)
        val j = jobs({ hold.await(); Tone(1) }, { FakeJob() }, thread = true)
        assertTrue(j.start(req))
        assertFalse(j.start(req.copy(name = "other.m4a")))
        hold.countDown()
        await(j)
        assertTrue(j.state.value is CallJobState.Done)
        assertTrue(j.start(req)) // free again
        await(j)
    }

    /**
     * Owner requirement (D46): a dictation started mid-job is never blocked. The job's own engine
     * is separate (here: a separate fake), and while the dictation runs the job starts no new
     * chunk: the dictation waits for at most the one chunk already in flight.
     */
    @Test fun dictationMidJobWaitsForAtMostTheChunkInFlight() {
        val cpu = ReentrantLock() // the phone's CPU, shared by both engines
        val chunkMs = 150L
        val job = FakeJob(chunkMs, cpu)
        val dictating = AtomicBoolean(false)
        val j = jobs({ Tone(80) }, { job }, busy = { dictating.get() }, thread = true)
        assertTrue(j.start(req))

        // Start the dictation while a chunk is being transcribed (the worst case).
        var n = 0
        while (!job.inChunk.get() && n++ < 5000) Thread.sleep(1)
        assertTrue("a chunk is in flight", job.inChunk.get())
        val t0 = System.nanoTime()
        dictating.set(true)
        // The dictation's engine needs the CPU: it gets it once the in-flight chunk ends.
        cpu.lock()
        val waitedMs = (System.nanoTime() - t0) / 1_000_000
        try {
            assertTrue("dictation waited $waitedMs ms (one chunk is $chunkMs ms)", waitedMs <= chunkMs + 100)
            // While it records and is processed, the job pushes no more audio (so no new chunk).
            val pushesAtStart = job.pushes.get()
            Thread.sleep(400)
            assertTrue("job kept pushing during dictation", job.pushes.get() - pushesAtStart <= 1)
            assertFalse(job.inChunk.get())
        } finally {
            cpu.unlock()
        }
        // The dictation's own work never touches the job's engine: it "succeeds" independently.
        dictating.set(false)
        await(j)
        val done = j.state.value as CallJobState.Done
        assertEquals(10, store.load(done.id)!!.segments.size)
    }
}
