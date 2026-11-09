package dev.murmur.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread

/** Review C1: the native handle is never freed while a call is using it. */
class HandleLeaseTest {
    private val direct = Executor { it.run() }

    @Test fun closeWithoutUsersFreesOnce() {
        val freed = mutableListOf<Long>()
        val l = HandleLease(42, { freed += it }, direct)
        assertEquals(7, l.use({ -1 }) { it.toInt() - 35 })
        l.close(); l.close()
        assertEquals(listOf(42L), freed)
        assertFalse(l.isOpen)
        assertEquals("closed", l.use({ "closed" }) { "open" })
        assertTrue(l.awaitFreed(0))
    }

    @Test fun closeDuringACallDefersTheFreeToTheLastCaller() {
        val freed = AtomicInteger()
        val l = HandleLease(42, { freed.incrementAndGet() }, direct)
        val inside = CountDownLatch(1)
        val proceed = CountDownLatch(1)
        val t = thread { l.use({}) { inside.countDown(); proceed.await() } }
        inside.await()
        l.close()
        assertEquals("freed under a running call", 0, freed.get())
        assertEquals("no new caller after close", -1L, l.use({ -1L }) { it })
        assertFalse(l.awaitFreed(10))
        proceed.countDown(); t.join()
        assertEquals(1, freed.get())
        assertTrue(l.awaitFreed(0))
    }

    @Test fun releaseWaitsForCallersAndNeverFrees() {
        val freed = AtomicInteger()
        val l = HandleLease(42, { freed.incrementAndGet() }, direct)
        val inside = CountDownLatch(1)
        val t = thread { l.use({}) { inside.countDown(); Thread.sleep(50) } }
        inside.await()
        assertEquals(42L, l.release())
        t.join()
        assertEquals(0, freed.get())
        assertEquals(0L, l.release())
        l.close()
        assertEquals(0, freed.get())
    }

    @Test fun zeroHandleIsClosed() {
        val l = HandleLease(0, { error("never") }, direct)
        assertFalse(l.isOpen)
        assertTrue(l.awaitFreed(0))
    }

    /** Many threads hammer the handle while another closes it: free exactly once, never with a user inside. */
    @Test fun stress() {
        repeat(200) { round ->
            val inside = AtomicInteger()
            val violations = AtomicInteger()
            val freed = AtomicInteger()
            val l = HandleLease(round + 1L, {
                if (inside.get() != 0) violations.incrementAndGet()
                freed.incrementAndGet()
            }, direct)
            val stop = CountDownLatch(1)
            val workers = List(4) {
                thread {
                    while (stop.count > 0) l.use({ Thread.yield() }) {
                        inside.incrementAndGet(); Thread.yield(); inside.decrementAndGet()
                    }
                }
            }
            Thread.sleep(1)
            l.close()
            stop.countDown(); workers.forEach { it.join() }
            assertTrue(l.awaitFreed(TimeUnit.SECONDS.toMillis(5)))
            assertEquals(1, freed.get())
            assertEquals(0, violations.get())
        }
    }
}
