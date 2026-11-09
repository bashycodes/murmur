package dev.flowclone.command

/**
 * Command Mode: the focused field's text and selection when the command started (node text +
 * `textSelectionStart/End`, UTF-16 indices like a Kotlin [String]). [start] ≤ [end] always.
 */
data class SelectionSnapshot(val text: String, val start: Int, val end: Int) {
    val selected: String get() = text.substring(start, end)
    val before: String get() = text.substring(0, start)
    val after: String get() = text.substring(end)
    val hasSelection: Boolean get() = end > start

    companion object {
        /**
         * Normalises what the accessibility node reports: a backwards selection (anchor after
         * focus) is swapped, indices past the text are clamped. Null without text or without
         * selection info (`-1`).
         */
        fun of(text: String?, selStart: Int, selEnd: Int): SelectionSnapshot? {
            if (text == null || selStart < 0 || selEnd < 0) return null
            val a = minOf(selStart, selEnd).coerceIn(0, text.length)
            val b = maxOf(selStart, selEnd).coerceIn(0, text.length)
            return SelectionSnapshot(text, a, b)
        }
    }
}

/**
 * Where a Command Mode answer goes: only the selected range is replaced, everything around it
 * is kept. Pure, so the splice is JVM-tested; the a11y service writes [Plan.SetText] with
 * `ACTION_SET_TEXT` and puts the cursor at [Plan.SetText.cursor].
 */
object SelectionSplice {
    sealed interface Plan {
        /** The field's whole new text, and the cursor right after the inserted answer. */
        data class SetText(val text: String, val cursor: Int) : Plan

        /** The selection is gone or can't be located for sure: touch nothing. */
        data object Stale : Plan
    }

    /**
     * [current] = the field's text now. Unchanged since [captured]: the captured range. Edited
     * elsewhere meanwhile: wherever the selected text now occurs, if it occurs exactly once.
     * Otherwise (or with nothing selected) [Plan.Stale].
     */
    fun plan(captured: SelectionSnapshot, current: String?, replacement: String): Plan {
        if (current == null || !captured.hasSelection) return Plan.Stale
        val at = when {
            current == captured.text -> captured.start
            else -> {
                val first = current.indexOf(captured.selected)
                if (first < 0 || current.indexOf(captured.selected, first + 1) >= 0) return Plan.Stale
                first
            }
        }
        val end = at + captured.selected.length
        return Plan.SetText(current.substring(0, at) + replacement + current.substring(end), at + replacement.length)
    }

    /** The field reads back as [plan] wrote it (editors may trim trailing whitespace). */
    fun applied(plan: Plan.SetText, readBack: String?): Boolean =
        readBack != null && readBack.trimEnd() == plan.text.trimEnd()
}
