package dev.flowclone.command

/**
 * Command Mode: the field a command acts on, read when the command starts (the a11y service
 * pins it like a dictation's target, S12).
 */
sealed interface CommandTarget {
    /**
     * An allowed editable field. [selection] = its text + selection: with a non-empty range the
     * answer replaces it; with none (caret only, or the app exposes no selection) the
     * instruction generates text at the cursor.
     */
    data class Field(val appId: String?, val selection: SelectionSnapshot?) : CommandTarget {
        /** The text the instruction rewrites, or null (speak-to-generate). */
        val selected: String? get() = selection?.takeIf { it.hasSelection }?.selected
    }

    /** Password / sensitive field, banking app, user denylist, or no editable field: nothing is read or sent. */
    data class Refused(val message: String) : CommandTarget

    companion object {
        const val PROTECTED_MESSAGE = "Command Mode is off in password, sensitive and banking fields"
        const val NO_FIELD_MESSAGE = "Command Mode needs a text field: tap into one first"
        /** The keyboard reports a selection the app doesn't let us read: never guess (it would write at the cursor instead). */
        const val UNREADABLE_SELECTION_MESSAGE = "Command Mode can't read the selected text in this app"
    }
}
