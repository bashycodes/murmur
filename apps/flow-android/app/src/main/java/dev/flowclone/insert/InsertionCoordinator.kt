package dev.flowclone.insert

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Terminal fallback: `ClipData` with `ClipDescription.EXTRA_IS_SENSITIVE = true` (07 §3.4). */
interface ClipboardPort {
    fun copySensitive(text: String)
}

/**
 * `insertTextAndVerify` (07 §3.1): one insertion at a time; the 3500 ms budget starts
 * BEFORE the mutex so queueing eats budget; failure → sensitive clipboard copy.
 */
class InsertionCoordinator(
    private val inserter: V3Inserter,
    private val clipboard: ClipboardPort,
    private val now: () -> Long,
) {
    private val mutex = Mutex()

    /** Keep-alive skips its ticks while > 0. */
    @Volatile var pendingInsertions = 0
        private set

    val isLocked get() = mutex.isLocked

    suspend fun insertTextAndVerify(text: String): InsertResult {
        val t0 = now()
        return mutex.withLock {
            pendingInsertions++
            val r = try {
                inserter.insert(text, t0)
            } finally {
                pendingInsertions--
            }
            if (!r.succeeded) clipboard.copySensitive(text)
            r
        }
    }
}
