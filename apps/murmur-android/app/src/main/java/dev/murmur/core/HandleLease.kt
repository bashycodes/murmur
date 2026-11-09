package dev.murmur.core

import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * Lifecycle of one native handle (review C1). Every native call runs inside [use], which counts
 * the caller in; [close] marks the handle dead and frees it only when the last caller has left,
 * so `nativeUnload` can never drop the box while `nativePushAudio` / `nativeCommit` /
 * `nativePoll` are still running on another thread (use-after-free).
 *
 * The free itself (`nativeUnload` joins the engine worker and may wait for a running commit)
 * runs on [freeExecutor], never on the thread that called [close] (which is often Main) or on
 * the AudioRecord thread that happened to leave last.
 */
class HandleLease(
    handle: Long,
    private val free: (Long) -> Unit,
    private val freeExecutor: Executor = FREE_EXECUTOR,
) {
    private val lock = Object()
    private var handle: Long = handle
    private var users = 0
    private var closed = handle == 0L
    private var releasing = false
    private val freed = CountDownLatch(if (handle == 0L) 0 else 1)

    /** False once [close] or [release] was called. */
    val isOpen: Boolean get() = synchronized(lock) { !closed }

    /** Callers currently inside [use] (tests / diagnostics). */
    val activeUsers: Int get() = synchronized(lock) { users }

    /** Run [f] with the live handle; [onClosed] when the handle is gone. Never frees under [f]. */
    inline fun <T> use(onClosed: () -> T, f: (Long) -> T): T {
        val h = enter()
        if (h == 0L) return onClosed()
        try {
            return f(h)
        } finally {
            leave()
        }
    }

    @PublishedApi internal fun enter(): Long = synchronized(lock) {
        if (closed) 0L else { users++; handle }
    }

    @PublishedApi internal fun leave() {
        val toFree = synchronized(lock) {
            users--
            when {
                releasing -> { lock.notifyAll(); 0L }
                closed && users == 0 -> takeHandle()
                else -> 0L
            }
        }
        if (toFree != 0L) scheduleFree(toFree)
    }

    /**
     * Unload: no new [use] gets the handle; it is freed now (no users) or when the last user
     * leaves. Idempotent. Returns immediately; [awaitFreed] waits for the free.
     */
    fun close() {
        val toFree = synchronized(lock) {
            if (closed) return
            closed = true
            if (users == 0) takeHandle() else 0L
        }
        if (toFree != 0L) scheduleFree(toFree)
    }

    /**
     * Ownership transfer (`nativeOpenRouter` takes the local handle): waits for running callers,
     * then hands the raw handle over without freeing it. 0 when already closed.
     */
    fun release(): Long {
        val x = synchronized(lock) {
            if (closed) return 0L
            closed = true
            releasing = true
            while (users > 0) lock.wait()
            takeHandle()
        }
        freed.countDown()
        return x
    }

    /** Wait until the native handle was freed (or given away); true when it was. */
    fun awaitFreed(timeoutMs: Long): Boolean = freed.await(timeoutMs, TimeUnit.MILLISECONDS)

    private fun takeHandle(): Long {
        val x = handle
        handle = 0
        lock.notifyAll()
        return x
    }

    private fun scheduleFree(h: Long) {
        freeExecutor.execute {
            try {
                free(h)
            } finally {
                freed.countDown()
            }
        }
    }

    companion object {
        /** One daemon thread for every `nativeUnload` (they are rare and may block). */
        val FREE_EXECUTOR: Executor = Executors.newSingleThreadExecutor { r ->
            Thread(r, "murmur-native-free").apply { isDaemon = true }
        }
    }
}
