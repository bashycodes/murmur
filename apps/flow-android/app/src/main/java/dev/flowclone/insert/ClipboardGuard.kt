package dev.flowclone.insert

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Review S16: the transcript never stays on the clipboard. When insertion fails (or the user
 * taps Copy) it is put there as a sensitive clip (`ClipDescription.EXTRA_IS_SENSITIVE`, API 33+)
 * for [holdMs]; then, or as soon as a later insertion succeeds, the user's previous clip is
 * restored (or the clipboard cleared) — unless the user copied something else meanwhile.
 */
class ClipboardGuard(
    private val port: Port,
    private val scope: CoroutineScope,
    private val holdMs: Long = HOLD_MS,
) {
    /** What the clipboard holds now, as far as this app may read it. */
    data class Clip(
        /** Our marker when the clip is one we set; null for anybody else's. */
        val marker: String?,
        /** Opaque platform clip (ClipData) to put back later. */
        val raw: Any?,
    )

    interface Port {
        /** Null when the platform does not let us read the clipboard right now. */
        fun current(): Clip?
        /** Set a sensitive plain-text clip tagged with [marker]. */
        fun setSensitive(text: String, marker: String)
        fun restore(raw: Any)
        fun clear()
        /** True if someone else changed the primary clip at or after [since] (ms, [now]'s clock). */
        fun changedSince(since: Long): Boolean
        fun now(): Long
    }

    private var job: Job? = null
    private var marker: String? = null
    private var setAt = 0L
    private var previous: Any? = null
    private var counter = 0

    /** A transcript is on the clipboard and will be removed. */
    val holding: Boolean get() = marker != null

    @Synchronized
    fun copy(text: String) {
        if (marker == null) {
            // Keep the user's own clip (never one of ours) to put back afterwards.
            previous = port.current()?.takeIf { it.marker == null }?.raw
        }
        val m = "flow-clip-${++counter}-${port.now()}"
        port.setSensitive(text, m)
        marker = m
        setAt = port.now()
        job?.cancel()
        job = scope.launch { delay(holdMs); expire() }
    }

    /** The text went in after all (Paste retry): take it off the clipboard now. */
    fun onInserted() { if (holding) expire() }

    @Synchronized
    fun expire() {
        val m = marker ?: return
        job?.cancel(); job = null
        val c = port.current()
        val stillOurs = if (c != null) c.marker == m else !port.changedSince(setAt)
        if (stillOurs) {
            val p = previous
            if (p != null) port.restore(p) else port.clear()
        }
        marker = null
        previous = null
    }

    companion object {
        const val HOLD_MS = 60_000L
    }
}
