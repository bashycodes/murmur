package dev.flowclone.bubble

import dev.flowclone.session.DictationState
import dev.flowclone.session.DictationState.ErrorKind

/** What the bubble shows (07 §5.4). */
data class BubbleUi(
    val state: DictationState = DictationState.Idle,
    val showCopy: Boolean = false,
    val level: Float = 0f,
    /** Shrunk to a 36dp dot after 5 s in Ready (07 §5.3). */
    val minimized: Boolean = false,
    /** Accessibility service off or dead: alert icon, tap opens the repair screen (07 §5.4 Repair). */
    val repair: Boolean = false,
    /** Window shown at all (presence policy: snooze, shake reveal, repair). */
    val visible: Boolean = state != DictationState.Idle,
    /** "Bubble size" setting: multiplies every bubble dimension ([dev.flowclone.data.Settings.bubbleScale]). */
    val scale: Float = 1f,
) {
    /** Resting (no dictation in flight), so the repair / minimised looks apply. */
    val resting: Boolean get() = state == DictationState.Idle || state == DictationState.Ready
}

enum class BubbleButton { MAIN, CANCEL, DONE, COPY, PASTE, DISMISS }

/**
 * Bubble sizes and button layout (07 §5.4), pure so sizes and hit-testing are JVM-testable.
 * Every length is the spec's dp value × [BubbleUi.scale]; at scale 1 it is the spec exactly.
 */
object BubbleGeometry {
    /** One round button: the h×h square starting at [left] px (the bubble's full height). */
    data class Slot(val left: Float, val right: Float, val button: BubbleButton)

    /** Widths from 07 §5.4 (56 circle, 120 with copy, 230 tap row, 160 hold row; 176 paste/copy/dismiss), unscaled. */
    fun baseWidthDp(u: BubbleUi): Float = if (u.resting && u.repair) 56f else if (u.resting && u.minimized && !u.showCopy) 36f else when (val s = u.state) {
        DictationState.Idle -> 56f
        DictationState.Ready -> if (u.showCopy) 120f else 56f
        is DictationState.Recording -> if (s.hold) 160f else 230f
        is DictationState.Processing -> if (s.hold) 160f else 230f
        is DictationState.Error -> if (s.kind is ErrorKind.FailedToPaste) 176f else 56f
    }

    fun baseHeightDp(u: BubbleUi): Float = if (u.resting && u.minimized && !u.repair && !u.showCopy) 36f else 56f

    fun widthDp(u: BubbleUi): Float = baseWidthDp(u) * u.scale
    fun heightDp(u: BubbleUi): Float = baseHeightDp(u) * u.scale

    /**
     * The round buttons drawn for [u] in a [widthPx]-wide bubble, left to right; [px] turns
     * unscaled dp into px (density × scale). [processingCancel]: a held Processing bubble shows
     * its ✕ only after 3 s. Anything outside a slot is [BubbleButton.MAIN] ([hit]).
     */
    fun slots(u: BubbleUi, widthPx: Float, heightPx: Float, px: (Float) -> Float, processingCancel: Boolean = true): List<Slot> {
        fun at(left: Float, b: BubbleButton) = Slot(left, left + heightPx, b)
        return when (val s = u.state) {
            DictationState.Idle, DictationState.Ready -> when {
                u.repair || u.minimized && !u.showCopy -> listOf(at(0f, BubbleButton.MAIN))
                u.showCopy -> listOf(at(0f, BubbleButton.MAIN), at(px(64f), BubbleButton.COPY))
                else -> listOf(at(0f, BubbleButton.MAIN))
            }
            is DictationState.Recording ->
                if (s.hold) listOf(at(0f, BubbleButton.MAIN))
                else listOf(at(0f, BubbleButton.CANCEL), at(widthPx - px(56f), BubbleButton.DONE))
            is DictationState.Processing ->
                if (!s.hold || processingCancel) listOf(at(0f, BubbleButton.CANCEL)) else emptyList()
            is DictationState.Error -> when (s.kind) {
                is ErrorKind.FailedToPaste -> listOf(at(0f, BubbleButton.PASTE), at(px(60f), BubbleButton.COPY), at(px(120f), BubbleButton.DISMISS))
                else -> listOf(at(0f, BubbleButton.DISMISS))
            }
        }
    }

    /** The button under ([x], [y]) px in a bubble [heightPx] tall; the pill around the buttons is [BubbleButton.MAIN]. */
    fun hit(slots: List<Slot>, heightPx: Float, x: Float, y: Float): BubbleButton =
        slots.firstOrNull { x >= it.left && x < it.right && y >= 0f && y < heightPx }?.button ?: BubbleButton.MAIN
}
