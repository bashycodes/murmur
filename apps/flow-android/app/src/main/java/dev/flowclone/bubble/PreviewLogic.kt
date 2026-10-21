package dev.flowclone.bubble

import dev.flowclone.session.DictationHint
import dev.flowclone.session.DictationState

/** The live-preview line next to the bubble: partial transcript, or a silence hint. */
object PreviewContent {
    const val MAX_CHARS = 140
    const val CANT_HEAR = "Can't hear you. Check the microphone."
    const val NO_SPEECH = "No speech heard. Nothing was inserted."
    const val ONLY_FILLER = "Only filler words heard. Nothing was inserted."

    data class Line(val text: String, val dim: Boolean)

    /** [badge]: D17 `show_engine_badge` — the last dictation's engine (S / L / L•), prefixed while recording. */
    fun line(state: DictationState, partial: String?, hint: DictationHint?, enabled: Boolean, badge: String? = null): Line? =
        lineNoBadge(state, partial, hint, enabled)?.let { l -> if (badge == null) l else l.copy(text = "[$badge] ${l.text}") }
            ?: if (badge != null && state is DictationState.Recording) Line("[$badge]", dim = true) else null

    private fun lineNoBadge(state: DictationState, partial: String?, hint: DictationHint?, enabled: Boolean): Line? {
        when (hint) {
            DictationHint.NoSpeechDetected -> return Line(NO_SPEECH, dim = false)
            DictationHint.OnlyFiller -> return Line(ONLY_FILLER, dim = false)
            DictationHint.NoSpeechYet -> if (state is DictationState.Recording) return Line(CANT_HEAR, dim = false)
            null -> Unit
        }
        if (!enabled || partial.isNullOrBlank()) return null
        return when (state) {
            is DictationState.Recording -> Line(tail(partial), dim = false)
            is DictationState.Processing -> Line(tail(partial), dim = true)
            else -> null
        }
    }

    /** Keep the newest words: cut at a word boundary and prefix an ellipsis. */
    fun tail(text: String, max: Int = MAX_CHARS): String {
        val t = text.trim().replace(Regex("\\s+"), " ")
        if (t.length <= max) return t
        val cut = t.substring(t.length - max + 1)
        val sp = cut.indexOf(' ')
        return "…" + if (sp in 0..20) cut.substring(sp + 1) else cut
    }
}

/**
 * Where the preview window goes: above the bubble (below it when there's no room), aligned
 * to the bubble's docked side so it grows away from the screen edge.
 */
object PreviewPlacement {
    data class Place(val alignRight: Boolean, val x: Int, val y: Int)

    fun place(bubbleX: Int, bubbleY: Int, bubbleW: Int, bubbleH: Int, previewH: Int, screenW: Int, gapPx: Int, topMarginPx: Int): Place {
        val right = bubbleX + bubbleW / 2 > screenW / 2
        val above = bubbleY - gapPx - previewH
        val y = if (above >= topMarginPx) above else bubbleY + bubbleH + gapPx
        val x = if (right) maxOf(0, screenW - (bubbleX + bubbleW)) else maxOf(0, bubbleX)
        return Place(right, x, y)
    }
}
