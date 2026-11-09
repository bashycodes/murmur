package dev.flowclone.bubble

import dev.flowclone.core.SessionRoute
import dev.flowclone.session.DictationHint
import dev.flowclone.session.DictationState

/** "via phone | server | provider" for the live route (2026-10-22 shell changelog), + why it is not the first route. */
object RouteLabel {
    /** D24: the server is unreachable and the phone model is loading on demand (audio is kept). */
    const val LOADING_PHONE_MODEL = "Loading phone model…"

    fun of(route: SessionRoute?, providerLabel: (String) -> String? = { null }): String? {
        val r = route ?: return null
        val base = when (r.engine) {
            "local" -> if (r.loading) r.loadingLabel ?: LOADING_PHONE_MODEL else "via phone"
            "remote" -> "via server"
            "cloud" -> {
                val name = r.provider?.let { providerLabel(it) ?: it } ?: "cloud"
                "via $name" + (r.model?.let { " ($it)" } ?: "")
            }
            else -> "via ${r.engine}"
        }
        return base + (r.fallback?.let { " · ${reason(it)}" } ?: "")
    }

    fun reason(fallback: String): String = when (fallback) {
        "connect_failed", "disconnected", "timeout" -> "server unreachable"
        "busy" -> "server busy"
        "auth_failed" -> "token or key rejected"
        "proto_mismatch" -> "server version differs"
        "circuit_open" -> "paused after failures"
        "server_not_configured" -> "no server set"
        "local_not_ready" -> "phone model not ready"
        "cloud_not_configured" -> "cloud not set up"
        "budget_exceeded" -> "cloud budget reached"
        "quota_exceeded" -> "cloud out of credits"
        "provider_error" -> "cloud error"
        else -> fallback.replace('_', ' ')
    }
}

/** The live-preview line next to the bubble: partial transcript, or a silence hint. */
object PreviewContent {
    const val MAX_CHARS = 140
    const val CANT_HEAR = "Can't hear you. Check the microphone."
    const val NO_SPEECH = "No speech heard. Nothing was inserted."
    const val ONLY_FILLER = "Only filler words heard. Nothing was inserted."
    /** Command Mode: the bubble is listening for an instruction, not for text to type. */
    const val COMMAND_LISTENING = "Command: say what to do with the selection"
    const val COMMAND_WORKING = "Command: rewriting…"

    data class Line(val text: String, val dim: Boolean)

    /**
     * [badge]: D17 `show_engine_badge` — the last dictation's engine (S / L / L•), prefixed while recording.
     * [route]: the live session's [RouteLabel] ("via server"); shown instead of the badge while
     * recording / processing, so the user sees where the audio goes before the commit.
     */
    fun line(state: DictationState, partial: String?, hint: DictationHint?, enabled: Boolean, badge: String? = null, route: String? = null): Line? {
        val live = state is DictationState.Recording || state is DictationState.Processing
        if (route != null && live) {
            val l = lineNoBadge(state, partial, hint, enabled)
            return if (l == null) Line(route, dim = true) else l.copy(text = "$route · ${l.text}")
        }
        return lineNoBadge(state, partial, hint, enabled)?.let { l -> if (badge == null) l else l.copy(text = "[$badge] ${l.text}") }
            ?: if (badge != null && state is DictationState.Recording) Line("[$badge]", dim = true) else null
    }

    private fun lineNoBadge(state: DictationState, partial: String?, hint: DictationHint?, enabled: Boolean): Line? {
        when (hint) {
            DictationHint.NoSpeechDetected -> return Line(NO_SPEECH, dim = false)
            DictationHint.OnlyFiller -> return Line(ONLY_FILLER, dim = false)
            DictationHint.NoSpeechYet -> if (state is DictationState.Recording) return Line(CANT_HEAR, dim = false)
            null -> Unit
        }
        val command = (state as? DictationState.Recording)?.command == true || (state as? DictationState.Processing)?.command == true
        if (command) {
            // Always shown (the only cue that this is a command); the instruction itself only with the live preview on.
            val said = partial?.takeIf { enabled && it.isNotBlank() }?.let { "Command: " + tail(it, MAX_CHARS - 9) }
            return if (state is DictationState.Recording) Line(said ?: COMMAND_LISTENING, dim = false) else Line(COMMAND_WORKING, dim = true)
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
