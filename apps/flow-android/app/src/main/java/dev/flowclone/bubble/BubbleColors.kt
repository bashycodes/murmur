package dev.flowclone.bubble

import android.graphics.Color
import dev.flowclone.ui.Tokens

/**
 * Bubble colours from the app design (docs/design/app-design/AndroidBubble.dc.html, flow-theme.css).
 * The bubble is always dark (it floats over any app, either theme), so it takes fixed token
 * values from [Tokens] where the design shares them; `raised` (vast-875) has no token of its own.
 */
internal object BubbleColors {
    /** vast-950: the pill, the preview tooltip, the snooze target. */
    val surface = Tokens.LIGHT.ink
    /** vast-875: a resting round button on the pill (mic disc, hold-mode pulse). */
    val raised = Color.parseColor("#FF343432")
    /** sand-50: glyphs, waveform bars, the primary action's disc. */
    val onSurface = Tokens.DARK.button
    /** vast-600: the dot's ring / dimmed strokes. */
    val muted = Tokens.LIGHT.badgeNeutralFg
    /** destructive (dark theme): errors and the repair alert only, never plain cancel. */
    val error = Tokens.DARK.danger
    /** Pill shadow: 0 2px 6px rgba(26,26,26,.3). */
    val shadow = Color.argb(0x4D, 0x1A, 0x1A, 0x1A)
    const val SHADOW_RADIUS_DP = 3f
    const val SHADOW_DY_DP = 2f
    /** Transparent margin round the bubble view so its shadow is not clipped by the window. */
    const val SHADOW_PAD_DP = 6
}
