package dev.flowclone.insert

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * IME connection keep-alive: while recording, probe the bound InputConnection
 * so it doesn't go stale; on a null read nudge (CLEAR_FOCUS+FOCUS) at most once per
 * IME session epoch. Always armed while recording.
 */
class ImeKeepAlive(private val deps: Deps, private val now: () -> Long) {
    interface Deps {
        /** insertion pending / mutex locked / no session / reply context / liveness excluded / editor ≠ anchor … */
        fun shouldSkip(): Boolean
        fun connection(): InputConnectionPort?
        /** Incremented on every `onStartInput`. */
        fun inputSessionEpoch(): Long
        /** hintAware && staleProbeSurface(anchor) && reacquireStaleSession() succeeded. */
        fun onStaleTick(): Boolean
    }

    companion object {
        const val FIRST_TICK_MS = 1_000L
        const val INTERVAL_MS = 10_000L
        const val SLOW_TICK_MS = 500L
        const val MAX_INTERVAL_MS = 40_000L
        const val NUDGE_RETRY_TICK_MS = 1_000L
        const val ARMED_TTL_MS = 900_000L
    }

    private var job: Job? = null
    private var armedAt = 0L
    private var interval = INTERVAL_MS
    private var nudgedEpoch = -1L
    private var awaitingRecovery = false
    private var retryPending = false

    var recovered = false; private set
    var skipped = 0; private set
    var nudges = 0; private set
    var lastOkAt = 0L; private set
    val isArmed get() = job?.isActive == true

    fun arm(scope: CoroutineScope) {
        if (isArmed) return
        armedAt = now(); interval = INTERVAL_MS
        nudgedEpoch = -1; awaitingRecovery = false; retryPending = false
        recovered = false; skipped = 0; nudges = 0
        job = scope.launch {
            delay(FIRST_TICK_MS)
            while (isActive) {
                if (now() - armedAt > ARMED_TTL_MS) break
                tick()
                delay(interval)
            }
            job = null
        }
    }

    fun disarm() {
        job?.cancel()
        job = null
    }

    private fun tick() {
        if (deps.shouldSkip()) {
            skipped++; interval = INTERVAL_MS
            return
        }
        val t0 = now()
        val ic = deps.connection()
        val ok = ic != null && runCatching { ic.getSurroundingText(0, 0) }.getOrNull() != null
        val epoch = deps.inputSessionEpoch()
        if (awaitingRecovery && epoch != nudgedEpoch) { recovered = true; awaitingRecovery = false }
        if (ok) {
            lastOkAt = now()
        } else if (ic != null && nudgedEpoch != epoch && deps.onStaleTick()) {
            nudgedEpoch = epoch; nudges++; awaitingRecovery = true; retryPending = true
        }
        val slow = now() - t0 > SLOW_TICK_MS
        interval = if (slow) minOf(interval * 2, MAX_INTERVAL_MS) else INTERVAL_MS
        if (retryPending) {
            retryPending = false
            if (!slow) interval = NUDGE_RETRY_TICK_MS
        }
    }
}
