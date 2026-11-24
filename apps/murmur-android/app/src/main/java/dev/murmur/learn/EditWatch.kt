package dev.murmur.learn

import dev.murmur.core.FieldContext

/**
 * "Learn from my edits" (Windows `apps/murmur-windows/src/learn.rs`),
 * platform-free.
 *
 * After an insert the accessibility service keeps re-reading the field the dictation went into
 * for up to [WatchOpts.windowMs]. The dictated region is found between two anchors taken from
 * the field context at dictation start (the text just before and just after the caret), so
 * edits elsewhere in the field don't count. When the watch ends, the last region is
 * word-aligned against what was inserted (`MurmurCore.learnEdit`) and the surviving
 * substitutions become **suggestions** ([Suggestions]): nothing reaches the dictionary
 * without the user's Add.
 *
 * End reasons mirror the Windows shell's; only [EndReason.counts]
 * ones are learned from. Android addition: [EndReason.STABLE] ends the watch early once an
 * edit has stopped changing for [WatchOpts.stableMs], so the field isn't read longer than needed.
 *
 * Timing (D110): besides the [WatchOpts.pollMs] timer, the field is read
 * [WatchOpts.settleMs] after each text-changed accessibility event ([FieldChanges]), never
 * closer than [WatchOpts.minReadGapMs] to the previous read, so a fix that stays in the field
 * for 300 ms is seen before a send or clear. A cleared field ([EndReason.CLEARED]) learns from
 * the last good observation, never from the empty field.
 */
sealed interface FieldRead {
    /** The whole text of the watched field (still focused). */
    data class Text(val text: String) : FieldRead
    /** Focus moved to another field or app, or the field became protected: stop watching. */
    data object FocusChanged : FieldRead
    /** Transient failure (no node right now); try again on the next poll. */
    data object Unavailable : FieldRead
}

/** Reads the field the dictation went into. Never reads a password / sensitive field. */
fun interface FieldReader {
    fun read(): FieldRead
}

/** Text-changed signal for the watched field (no text: only "something changed"). */
interface FieldChanges : AutoCloseable {
    /** True as soon as a change arrived (or one is pending), false after [timeoutMs] without. */
    suspend fun await(timeoutMs: Long): Boolean

    /** Stop listening for changes. */
    override fun close()
}

data class WatchOpts(
    /** Total observation window (Windows `learn_window_ms`). */
    val windowMs: Long = 45_000,
    /** The inserted text must show up in the field within this time. */
    val baselineMs: Long = 3_000,
    /** Re-read interval without a text-changed event (Windows `learn_poll_ms`). */
    val pollMs: Long = 1_000,
    /** Cost bound: at most one read per this many ms, events or not. */
    val minReadGapMs: Long = 250,
    /** After a text-changed event, wait this long so a burst of keystrokes is one read. */
    val settleMs: Long = 100,
    /** An edit unchanged this long ends the watch early ([EndReason.STABLE]). */
    val stableMs: Long = 8_000,
)

enum class EndReason {
    TIMEOUT, FOCUS_CHANGED, STABLE,
    /** The whole field became empty (sent or cleared): the last good observation counts. */
    CLEARED,
    /** The text around the insert changed so the region can't be found. */
    ANCHOR_MISMATCH,
    /** The user deleted the dictated text (or sent it: chat boxes clear). */
    REGION_REMOVED,
    /** The inserted text never appeared between the anchors. */
    PASTE_NOT_OBSERVED,
    /** Learn from my edits was turned off (or the watch was dropped) meanwhile. */
    CANCELLED,
    ;

    /** Whether the last observation may be learned from. */
    val counts: Boolean get() = this == TIMEOUT || this == FOCUS_CHANGED || this == STABLE || this == CLEARED
}

/** [edited]: the corrected region, when the reason counts and it differs from the insert. */
data class Outcome(val reason: EndReason, val edited: String? = null)

/** Text before / after the caret at dictation start. */
data class Anchors(val before: String = "", val after: String = "") {
    companion object {
        /** Long enough to be unique in practice, short enough to survive edits far from the insert. */
        const val CHARS = 48

        fun of(ctx: FieldContext) = Anchors(ctx.before.takeLast(CHARS), ctx.after.take(CHARS))
    }
}

object EditText {
    /** Collapse whitespace so trailing spaces / line breaks from the insert don't count as edits. */
    fun normalize(s: String): String = s.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")

    private fun starts(hay: String, needle: String): List<Int> {
        if (needle.isEmpty()) return listOf(0)
        val out = ArrayList<Int>()
        var i = hay.indexOf(needle)
        while (i >= 0) { out += i; i = hay.indexOf(needle, i + 1) }
        return out
    }

    /**
     * The dictated region of [field]: between the `before` anchor and the next `after` anchor,
     * choosing the pair whose length is closest to [expectLen] (the inserted length) when
     * anchors repeat. Null when the anchors are gone.
     */
    fun region(field: String, a: Anchors, expectLen: Int): String? {
        var best: Triple<Int, Int, Int>? = null
        for (p in starts(field, a.before)) {
            val start = p + a.before.length
            val end = if (a.after.isEmpty()) field.length else field.indexOf(a.after, start).takeIf { it >= 0 } ?: continue
            val score = kotlin.math.abs((end - start) - expectLen)
            if (best == null || score < best.third) best = Triple(start, end, score)
        }
        return best?.let { field.substring(it.first, it.second) }
    }

    /**
     * Cut words typed before or after the dictated text out of the observed region, so going on
     * typing isn't read as a correction of the last inserted word. Semi-global word alignment:
     * leading and trailing words of [edited] are free; the window with the lowest edit distance
     * to [pasted] wins (ties: the longer window, so a word inserted inside the dictation stays).
     */
    fun boundEdit(pasted: String, edited: String): String {
        val p = pasted.split(Regex("\\s+")).filter { it.isNotEmpty() }
        val e = edited.split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (p.isEmpty() || e.isEmpty()) return ""
        fun key(w: String) = w.trim { !it.isLetterOrDigit() }
        val pk = p.map(::key)
        val ek = e.map(::key)
        val n = p.size
        val m = e.size
        val dp = Array(n + 1) { i -> IntArray(m + 1).also { it[0] = i } }
        for (i in 1..n) for (j in 1..m) {
            val sub = dp[i - 1][j - 1] + if (pk[i - 1] != ek[j - 1]) 1 else 0
            dp[i][j] = minOf(sub, dp[i - 1][j] + 1, dp[i][j - 1] + 1)
        }
        val best = (0..m).minOf { dp[n][it] }
        val end = (m downTo 0).first { dp[n][it] == best }
        var i = n
        var j = end
        while (i > 0 && j > 0) {
            val sub = dp[i - 1][j - 1] + if (pk[i - 1] != ek[j - 1]) 1 else 0
            when {
                dp[i][j] == sub && pk[i - 1] == ek[j - 1] -> { i--; j-- }
                dp[i][j] == dp[i][j - 1] + 1 -> j-- // word inserted inside the window
                dp[i][j] == sub -> { i--; j-- }
                else -> i--
            }
        }
        return e.subList(j, end).joinToString(" ")
    }
}

/** Observation state machine for one insert. Time is injected. */
class EditWatch(pasted: String, private val anchors: Anchors, private val startedMs: Long, val opts: WatchOpts = WatchOpts()) {
    private val pastedNorm = EditText.normalize(pasted)
    private val pastedLen = pasted.length
    private var baselineSeen = false
    private var lastRegion: String? = null
    /**
     * The last region that wasn't just a cut of the one before (backspacing from the end): what
     * a clear learns from, so holding Backspace down to an empty field isn't read as a fix.
     */
    private var lastGood: String? = null
    private var lastChangeMs = startedMs

    private fun remember(r: String) {
        val good = lastGood
        if (good == null || !(r.length < good.length && good.startsWith(r))) lastGood = r
        lastRegion = r
    }

    /** Null = keep watching. */
    fun observe(nowMs: Long, read: FieldRead): Outcome? {
        val elapsed = (nowMs - startedMs).coerceAtLeast(0)
        val baselineOver = elapsed >= minOf(opts.baselineMs, opts.windowMs)
        when (read) {
            FieldRead.FocusChanged -> return finish(EndReason.FOCUS_CHANGED)
            FieldRead.Unavailable -> Unit
            is FieldRead.Text -> {
                if (baselineSeen && EditText.normalize(read.text).isEmpty()) return finish(EndReason.CLEARED)
                val raw = EditText.region(read.text, anchors, pastedLen)
                when {
                    raw == null && !baselineSeen -> if (baselineOver) return finish(EndReason.PASTE_NOT_OBSERVED)
                    raw == null -> return finish(EndReason.ANCHOR_MISMATCH)
                    else -> {
                        val r = EditText.normalize(raw)
                        if (!baselineSeen) {
                            if (r == pastedNorm) { baselineSeen = true; remember(r); lastChangeMs = nowMs }
                            else if (baselineOver) return finish(EndReason.PASTE_NOT_OBSERVED)
                        } else if (r.isEmpty()) {
                            return finish(EndReason.REGION_REMOVED)
                        } else {
                            if (r != lastRegion) lastChangeMs = nowMs
                            remember(r)
                            if (r != pastedNorm && nowMs - lastChangeMs >= opts.stableMs) return finish(EndReason.STABLE)
                        }
                    }
                }
            }
        }
        if (elapsed >= opts.windowMs) return finish(if (baselineSeen) EndReason.TIMEOUT else EndReason.PASTE_NOT_OBSERVED)
        return null
    }

    fun finish(reason: EndReason): Outcome {
        val last = if (reason == EndReason.CLEARED) lastGood else lastRegion
        val edited = if (reason.counts && baselineSeen) last?.takeIf { it != pastedNorm && it.isNotEmpty() } else null
        return Outcome(reason, edited)
    }
}

/**
 * Drive [reader] until the watch ends: read, observe, then wait for a text-changed event from
 * [changes] or [WatchOpts.pollMs] (from the start of the read), whichever comes first. After an
 * event the next read waits [WatchOpts.settleMs]; reads never start closer than
 * [WatchOpts.minReadGapMs]. Without [changes] it is a plain timer. [stillWanted] is checked
 * before every read, so turning the setting off stops the re-reads at once.
 */
suspend fun runWatch(
    reader: FieldReader, watch: EditWatch, now: () -> Long, sleep: suspend (Long) -> Unit,
    stillWanted: () -> Boolean = { true },
    changes: FieldChanges? = null,
): Outcome {
    val gap = watch.opts.minReadGapMs.coerceAtLeast(1)
    val poll = watch.opts.pollMs.coerceAtLeast(gap)
    while (true) {
        if (!stillWanted()) return watch.finish(EndReason.CANCELLED)
        val started = now()
        watch.observe(started, reader.read())?.let { return it }
        val untilPoll = (started + poll - now()).coerceAtLeast(1)
        val changed = if (changes != null) changes.await(untilPoll) else { sleep(untilPoll); false }
        val wait = maxOf(if (changed) watch.opts.settleMs else 0, started + gap - now())
        if (wait > 0) sleep(wait)
    }
}
