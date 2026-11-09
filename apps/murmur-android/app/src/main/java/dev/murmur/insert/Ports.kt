package dev.flowclone.insert

/**
 * Injectable editor abstraction for the V3 insertion state machine.
 *
 * The Android implementation (`service/AndroidInsertionEnvironment`) adapts
 * `AccessibilityService.InputMethod` + `AccessibilityNodeInfo`; tests use fakes.
 * Nothing here touches android.* so the state machine runs on the plain JVM.
 */

/** Result of `InputConnection.getSurroundingText`; [text] null = content-null object. */
data class SurroundingText(val text: String?)

/** The IME channel: `AccessibilityService.InputMethod.currentInputConnection`. */
interface InputConnectionPort {
    /** `commitText(text, 1, null)`. May throw on a dead binder. */
    fun commitText(text: String): Boolean
    /** `getSurroundingText(before, after, 0)`; null = no object returned. */
    fun getSurroundingText(before: Int, after: Int): SurroundingText?
}

/** Plain-value read of an editable node, used by the placeholder rules. */
data class NodeSnapshot(
    val text: String?,
    val hint: String? = null,
    val contentDescription: String? = null,
    val showingHint: Boolean = false,
)

/** The accessibility channel: an editable `AccessibilityNodeInfo`. */
interface EditableNode {
    val packageName: String?
    val isEditable: Boolean
    val isPassword: Boolean
    val inputType: Int
    /** `ACTION_SET_TEXT` is in the node's action list. */
    val canSetText: Boolean
    fun snapshot(): NodeSnapshot
    /** `AccessibilityNodeInfo.refresh()`. */
    fun refresh(): Boolean
    /** `performAction(ACTION_SET_TEXT, ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE = text)`. */
    fun setText(text: String): Boolean
}

/** Everything the inserter reads from the a11y service / InputMethod. */
interface InsertionEnvironment {
    /** `onStartInput` seen and not yet finished. */
    fun isInputSessionActive(): Boolean
    fun hasSessionEverStarted(): Boolean
    fun currentInputConnection(): InputConnectionPort?
    /** Suspend until the next `onStartInput` or timeout; true if it arrived. */
    suspend fun awaitInputStart(timeoutMs: Long): Boolean
    /** `EditorInfo.packageName` of the editor the IME is bound to. */
    fun imeBoundPackage(): String?
    /** `findFocus(FOCUS_INPUT).packageName`. */
    fun focusedInputPackage(): String?
    /** `EditorInfo.inputType` of the bound editor; null = no editor bound. */
    fun editorInputType(): Int?
    /** `EditorInfo.hintText`. */
    fun editorHint(): String?
    /** Last `onStartInput` had `restarting = true`. */
    fun boundEditorRestarted(): Boolean
    fun isInNotificationReplyContext(): Boolean
    /** Package of the focused application window root. */
    fun currentPackage(): String?
    /** Package captured when dictation started. */
    fun dictationStartPackage(): String?
    /** Focus nudge attempt 1: `ACTION_FOCUS` on the focused editable node. */
    fun reacquireInputSession(): Boolean
    /** Focus nudge attempt 2: `ACTION_CLEAR_FOCUS` then `ACTION_FOCUS`. */
    fun reacquireStaleSession(): Boolean
    /** `FocusedNodeFinder.findFocusedNode(root)`. */
    fun findFocusedInputNode(): EditableNode?
    /** Editable node whose identity hash matches the dictation-start snapshot. */
    fun relocateDictationStartNode(): EditableNode?
}
