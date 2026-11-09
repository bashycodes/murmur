package dev.flowclone.bubble

import kotlin.math.roundToInt

/**
 * Bubble gesture state machine, independent of MotionEvent so it is JVM-testable.
 * The view calls [down]/[move]/[up] and schedules [longPressTimeout] after
 * `ViewConfiguration.getLongPressTimeout()`.
 *
 * - tap → [Action.Tap] (toggle, POPO)
 * - long-press → [Action.HoldStart]; release → [Action.HoldEnd] (PTT), even after dragging
 * - move past touch slop before the timeout → drag only
 * - in gesture-blocked states (Recording / Processing / Error / Repair) the bubble only
 *   moves; a release without drag is reported as [Action.BlockedTap] for button hit-testing.
 */
class BubbleGesture(private val touchSlopPx: Float, private val blocked: () -> Boolean) {
    sealed interface Action {
        data object Tap : Action
        data object HoldStart : Action
        data object HoldEnd : Action
        data object DragStart : Action
        data class DragBy(val dx: Float, val dy: Float) : Action
        data object DragEnd : Action
        data class BlockedTap(val x: Float, val y: Float) : Action
    }

    private enum class Phase { NONE, PRESSED, BLOCKED_PRESSED, HOLD }

    private var phase = Phase.NONE
    private var dragging = false
    private var downX = 0f; private var downY = 0f
    private var lastX = 0f; private var lastY = 0f

    fun down(x: Float, y: Float) {
        phase = if (blocked()) Phase.BLOCKED_PRESSED else Phase.PRESSED
        dragging = false
        downX = x; downY = y; lastX = x; lastY = y
    }

    fun move(x: Float, y: Float): List<Action> {
        if (phase == Phase.NONE) return emptyList()
        val out = mutableListOf<Action>()
        if (!dragging) {
            val dx = x - downX; val dy = y - downY
            if (dx * dx + dy * dy <= touchSlopPx * touchSlopPx) return emptyList()
            dragging = true
            out += Action.DragStart
        }
        out += Action.DragBy(x - lastX, y - lastY)
        lastX = x; lastY = y
        return out
    }

    fun longPressTimeout(): List<Action> {
        if (phase != Phase.PRESSED || dragging) return emptyList()
        phase = Phase.HOLD
        return listOf(Action.HoldStart)
    }

    fun up(x: Float, y: Float): List<Action> {
        val p = phase
        phase = Phase.NONE
        return when (p) {
            Phase.NONE -> emptyList()
            Phase.HOLD -> if (dragging) listOf(Action.HoldEnd, Action.DragEnd) else listOf(Action.HoldEnd)
            Phase.PRESSED -> if (dragging) listOf(Action.DragEnd) else listOf(Action.Tap)
            Phase.BLOCKED_PRESSED -> if (dragging) listOf(Action.DragEnd) else listOf(Action.BlockedTap(x, y))
        }
    }

    fun cancel() { phase = Phase.NONE; dragging = false }
}

/** What a tap on an error bubble (its ! or the pill around it) does. */
object ErrorTap {
    enum class Action {
        /** Check the engine again and start dictating when it is ready. */
        RECHECK,
        /** A real engine error: open the app, where it is fixed. */
        OPEN_APP,
        /** Just dismiss the error. */
        DISMISS,
    }

    fun of(kind: dev.flowclone.session.DictationState.ErrorKind): Action = when (kind) {
        is dev.flowclone.session.DictationState.ErrorKind.EngineLoading -> Action.RECHECK
        is dev.flowclone.session.DictationState.ErrorKind.EngineUnavailable -> Action.OPEN_APP
        else -> Action.DISMISS
    }
}

/**
 * Bubble anchors: X snaps to Left (20dp) or Right (screenW − w − 20dp); Y snaps
 * to one of 100 evenly spaced anchors between 56dp margins, default index 35.
 * The snapped anchor is persisted via [Store]. The margins scale with the
 * "Bubble size" [scale] like the bubble itself; the saved anchor (side + index) is
 * size-independent, so it stays valid when the size changes.
 */
class BubblePositioner(
    val screenW: Int, val screenH: Int, val density: Float, private val store: Store, val scale: Float = 1f,
) {
    enum class Side { LEFT, RIGHT }
    data class Saved(val side: Side, val yIndex: Int)
    data class Pos(val x: Int, val y: Int)

    interface Store {
        fun load(): Saved?
        fun save(s: Saved)
    }

    companion object {
        const val Y_ANCHORS = 100
        const val DEFAULT_Y_INDEX = 35
        const val EDGE_DP = 20
        const val Y_MARGIN_DP = 56
    }

    private fun dp(v: Int) = (v * density * scale).toInt()
    private var saved: Saved = store.load() ?: Saved(Side.RIGHT, DEFAULT_Y_INDEX)

    /** Never off-screen: a bubble wider than the screen minus its margin is pulled back in. */
    fun xFor(side: Side, bubbleW: Int) = when (side) {
        Side.LEFT -> dp(EDGE_DP)
        Side.RIGHT -> screenW - bubbleW - dp(EDGE_DP)
    }.coerceIn(0, maxOf(0, screenW - bubbleW))

    fun yForIndex(i: Int, bubbleH: Int): Int {
        val top = dp(Y_MARGIN_DP)
        val bottom = maxOf(top, screenH - dp(Y_MARGIN_DP) - bubbleH)
        return top + ((bottom - top) * i.coerceIn(0, Y_ANCHORS - 1) / (Y_ANCHORS - 1).toFloat()).roundToInt()
    }

    /** Current resting position; a right-docked bubble grows leftwards when it widens. */
    fun position(bubbleW: Int, bubbleH: Int) = Pos(xFor(saved.side, bubbleW), yForIndex(saved.yIndex, bubbleH))

    fun snap(x: Int, y: Int, bubbleW: Int, bubbleH: Int): Pos {
        val side = if (x + bubbleW / 2 < screenW / 2) Side.LEFT else Side.RIGHT
        val top = dp(Y_MARGIN_DP)
        val bottom = maxOf(top + 1, screenH - dp(Y_MARGIN_DP) - bubbleH)
        val idx = (((y - top).toFloat() / (bottom - top)) * (Y_ANCHORS - 1)).roundToInt().coerceIn(0, Y_ANCHORS - 1)
        saved = Saved(side, idx)
        store.save(saved)
        return position(bubbleW, bubbleH)
    }

    fun clampDrag(x: Int, y: Int, bubbleW: Int, bubbleH: Int) =
        Pos(x.coerceIn(0, maxOf(0, screenW - bubbleW)), y.coerceIn(0, maxOf(0, screenH - bubbleH)))
}

/** Bubble waveform maths. Heights in dp. */
object Waveform {
    val SCALE = floatArrayOf(.25f, .47f, .69f, .875f, 1f, 1f, .875f, .69f, .47f, .25f)
    const val BAR_WIDTH_DP = 3f
    const val BAR_GAP_DP = 4f
    const val MIN_HEIGHT_DP = 3f
    const val BASE_HEIGHT_DP = 32f
    private val KF_T = floatArrayOf(0f, .2f, .4f, .8f, .9f, 1f)
    private val KF_V = floatArrayOf(.57f, .74f, 1f, .66f, .83f, .57f)

    fun keyframe(p: Float): Float {
        val x = p.coerceIn(0f, 1f)
        for (i in 1 until KF_T.size) {
            if (x <= KF_T[i]) {
                val f = (x - KF_T[i - 1]) / (KF_T[i] - KF_T[i - 1])
                return KF_V[i - 1] + f * (KF_V[i] - KF_V[i - 1])
            }
        }
        return KF_V.last()
    }

    /** `h_i = max(3, max(3, kf((t + 0.1·i) mod 1 s)·scale_i·32)·s)` with s = smoothed level. */
    fun barHeightsDp(tMs: Long, level: Float): FloatArray = FloatArray(SCALE.size) { i ->
        val phase = ((tMs / 1000f) + 0.1f * i) % 1f
        maxOf(MIN_HEIGHT_DP, maxOf(MIN_HEIGHT_DP, keyframe(phase) * SCALE[i] * BASE_HEIGHT_DP) * level)
    }

    /** `s += (level − s)·0.15` per frame. */
    class Smoother {
        var value = 0f; private set
        fun step(level: Float): Float { value += (level - value) * 0.15f; return value }
    }
}
