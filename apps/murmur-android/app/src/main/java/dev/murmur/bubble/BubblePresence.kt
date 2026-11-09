package dev.flowclone.bubble

import dev.flowclone.session.DictationState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlin.math.sqrt

/**
 * Shake: an impulse fires when |a|/g − 1 > 1.0 (100 ms cooldown); two impulses,
 * each within 1200 ms of the previous, are a shake. Pure: feed it accelerometer samples.
 */
class ShakeDetector {
    companion object {
        const val THRESHOLD = 1.0f
        const val COOLDOWN_MS = 100L
        const val WINDOW_MS = 1200L
        const val GRAVITY = 9.80665f
    }

    private var lastImpulseAt = Long.MIN_VALUE / 2
    private var count = 0

    /** Returns true when this sample completes a shake. */
    fun onSample(x: Float, y: Float, z: Float, nowMs: Long): Boolean {
        val g = sqrt(x * x + y * y + z * z) / GRAVITY - 1f
        if (g <= THRESHOLD) return false
        val since = nowMs - lastImpulseAt
        if (since < COOLDOWN_MS) return false
        count = if (since <= WINDOW_MS) count + 1 else 1
        lastImpulseAt = nowMs
        if (count >= 2) { count = 0; lastImpulseAt = Long.MIN_VALUE / 2; return true }
        return false
    }
}

/**
 * Bubble presence on top of the dictation state (minimise, snooze, shake):
 * - Ready for 5 s → minimised (36dp) when enabled; a touch restores it first.
 * - Snooze (drop on the bottom target, Ready only, refused in Repair) hides it for 10 min.
 * - Shake: snoozed → un-snooze; Idle → reveal for 10 s. Ignored on the lock screen.
 */
class BubblePresence(
    private val scope: CoroutineScope,
    private val minimiseEnabled: () -> Boolean,
) {
    companion object {
        const val MINIMISE_AFTER_MS = 5_000L
        const val SNOOZE_MS = 600_000L
        const val REVEAL_MS = 10_000L
    }

    data class Presence(val minimized: Boolean = false, val snoozed: Boolean = false, val revealed: Boolean = false)

    enum class ShakeOutcome { UNSNOOZED, REVEALED, IGNORED }

    private val _presence = MutableStateFlow(Presence())
    val presence: StateFlow<Presence> = _presence

    /** The a11y service is enabled but dead (ghost) or off: the bubble shows a repair icon. */
    @Volatile var repair = false

    private var state: DictationState = DictationState.Idle
    private var minimiseJob: Job? = null
    private var snoozeJob: Job? = null
    private var revealJob: Job? = null

    fun onState(s: DictationState) {
        val was = state
        state = s
        if (s == DictationState.Ready) {
            if (was != DictationState.Ready) armMinimise()
        } else {
            minimiseJob?.cancel(); minimiseJob = null
            if (_presence.value.minimized) update { it.copy(minimized = false) }
        }
    }

    /** A tap or long-press on the bubble. Returns true if it was minimised (now restored). */
    fun touch(): Boolean {
        val wasMin = _presence.value.minimized
        if (wasMin) update { it.copy(minimized = false) }
        if (state == DictationState.Ready) armMinimise()
        return wasMin
    }

    fun snooze(): Boolean {
        if (state != DictationState.Ready || repair) return false
        update { it.copy(snoozed = true, minimized = false) }
        snoozeJob?.cancel()
        snoozeJob = scope.launch { delay(SNOOZE_MS); update { it.copy(snoozed = false) } }
        return true
    }

    fun unsnooze() {
        snoozeJob?.cancel(); snoozeJob = null
        update { it.copy(snoozed = false) }
    }

    fun onShake(locked: Boolean): ShakeOutcome {
        if (locked) return ShakeOutcome.IGNORED
        if (_presence.value.snoozed) { unsnooze(); return ShakeOutcome.UNSNOOZED }
        if (state == DictationState.Idle) {
            update { it.copy(revealed = true) }
            revealJob?.cancel()
            revealJob = scope.launch { delay(REVEAL_MS); update { it.copy(revealed = false) } }
            return ShakeOutcome.REVEALED
        }
        return ShakeOutcome.IGNORED
    }

    /** Should the bubble window be shown for dictation state [s]? */
    fun visible(s: DictationState): Boolean {
        val p = _presence.value
        return when (s) {
            is DictationState.Recording, is DictationState.Processing, is DictationState.Error -> true
            DictationState.Ready -> !p.snoozed
            DictationState.Idle -> !p.snoozed && (p.revealed || repair)
        }
    }

    private fun armMinimise() {
        minimiseJob?.cancel()
        minimiseJob = if (!minimiseEnabled()) null else scope.launch {
            delay(MINIMISE_AFTER_MS)
            if (state == DictationState.Ready) update { it.copy(minimized = true) }
        }
    }

    private inline fun update(f: (Presence) -> Presence) { _presence.value = f(_presence.value) }
}

/**
 * Bottom snooze target: window = screen width × (73dp + 7% of screen height),
 * gravity BOTTOM; dropping the bubble centre within 100dp of the target centre snoozes.
 */
class SnoozeTarget(val screenW: Int, val screenH: Int, val density: Float) {
    companion object {
        const val SHOW_AFTER_DRAG_MS = 250L
        const val NEAR_DP = 100f
        const val CIRCLE_DP = 60f
        const val CIRCLE_NEAR_DP = 86f
    }

    fun windowHeightPx(): Int = (73 * density).toInt() + (screenH * 0.07f).toInt()

    /** Target centre in screen coordinates. */
    fun center(): Pair<Float, Float> = screenW / 2f to screenH - windowHeightPx() / 2f

    fun isNear(x: Float, y: Float): Boolean {
        val (cx, cy) = center()
        val dx = x - cx; val dy = y - cy
        val r = NEAR_DP * density
        return dx * dx + dy * dy <= r * r
    }
}
