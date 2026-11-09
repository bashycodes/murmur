package dev.murmur.command

import dev.murmur.debug.DebugRedact
import dev.murmur.policy.EditorInputType

/**
 * Command Mode's field policy: which fields a command may read (and send to the AI). Stricter
 * than dictation: any password / sensitive / content-excluded field (numbers, phone, URI),
 * banking app or user-denylisted app is refused before anything is read.
 */
object CommandGuard {
    /** Null = allowed, else the bubble's message. */
    fun refusal(
        hasField: Boolean, inputType: Int, nodePassword: Boolean, dataSensitive: Boolean,
        pkg: String?, userDenylist: Set<String>, ownAppBlocked: Boolean = false,
    ): String? = when {
        !hasField || ownAppBlocked -> CommandTarget.NO_FIELD_MESSAGE
        DebugRedact.isProtectedField(inputType, nodePassword, dataSensitive) ||
            EditorInputType.isContentExtractionExcluded(inputType) ||
            DebugRedact.isProtectedApp(pkg, userDenylist) -> CommandTarget.PROTECTED_MESSAGE
        else -> null
    }
}

/** The focused field as Command Mode writes it: the a11y node, and the a11y InputMethod's connection. */
interface CommandField {
    /** The node's whole text now (re-read), null = unreadable. */
    fun readText(): String?
    /** The node offers `ACTION_SET_TEXT`. */
    val canSetText: Boolean
    fun setText(text: String): Boolean
    /** `ACTION_SET_SELECTION` to a caret at [at]. */
    fun setCursor(at: Int): Boolean
    /** The input connection's selected text now; null = no connection (or nothing it reports). */
    fun imeSelectedText(): String?
    /** `commitText` on the input connection: replaces its current selection. */
    fun imeCommit(text: String): Boolean
}

/**
 * Replace only the captured selection: `ACTION_SET_TEXT` with the text around it kept
 * ([SelectionSplice]), verified by reading the field back; else the input connection's
 * `commitText`, but only while exactly the captured text is still selected there. A selection
 * that moved is never guessed at: false, and the field is left alone.
 */
object CommandApply {
    fun replaceSelection(captured: SelectionSnapshot, text: String, f: CommandField): Boolean {
        if (f.canSetText) {
            when (val plan = SelectionSplice.plan(captured, f.readText(), text)) {
                SelectionSplice.Plan.Stale -> return false
                is SelectionSplice.Plan.SetText -> if (f.setText(plan.text)) {
                    f.setCursor(plan.cursor)
                    return SelectionSplice.applied(plan, f.readText())
                }
            }
        }
        val selectedNow = f.imeSelectedText() ?: return false
        return selectedNow == captured.selected && f.imeCommit(text)
    }
}
