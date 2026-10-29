package dev.flowclone.bubble

import android.graphics.Color

/**
 * Bubble colours from the app design (docs/design/app-design/AndroidBubble.dc.html, flow-theme.css).
 * Kept local to the bubble package until the shared ui tokens land; then this folds into them.
 */
internal object BubbleColors {
    /** vast-950: the pill, the preview tooltip, the snooze target. */
    val surface = Color.parseColor("#FF1A1A1A")
    /** vast-875: a resting round button on the pill (mic disc, hold-mode pulse). */
    val raised = Color.parseColor("#FF343432")
    /** sand-50: glyphs, waveform bars, the primary action's disc. */
    val onSurface = Color.parseColor("#FFFCFCFB")
    /** vast-600: the dot's ring / dimmed strokes. */
    val muted = Color.parseColor("#FF71716E")
    /** destructive (dark theme): errors and the repair alert only, never plain cancel. */
    val error = Color.parseColor("#FFFF5757")
    /** Pill shadow: 0 2px 6px rgba(26,26,26,.3). */
    val shadow = Color.argb(0x4D, 0x1A, 0x1A, 0x1A)
    const val SHADOW_RADIUS_DP = 3f
    const val SHADOW_DY_DP = 2f
    /** Transparent margin round the bubble view so its shadow is not clipped by the window. */
    const val SHADOW_PAD_DP = 6
}
