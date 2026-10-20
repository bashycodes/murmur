package dev.flowclone.insert

import kotlinx.coroutines.delay

/**
 * One fake text field reachable through both insertion channels: the IME
 * InputConnection (commitText / getSurroundingText) and the accessibility node
 * (ACTION_SET_TEXT). Knobs model the failure modes 05/07 describe.
 */
class FakeField(var text: String = "", val pkg: String = "com.example.notes") : InputConnectionPort, EditableNode {
    var commits = mutableListOf<String>()
    var setTexts = mutableListOf<String>()
    /** commitText returns but the text never lands (stale session). */
    var commitSilentlyDropped = false
    var commitThrows = false
    /** getSurroundingText returns null (e.g. WhatsApp / OEM IMEs). */
    var surroundingNull = false
    /** Node text unreadable (null). */
    var nodeTextNull = false
    var hint: String? = null
    var showingHint = false
    var contentDesc: String? = null
    override var isPassword = false
    override var inputType = 0x1
    override var canSetText = true
    override var isEditable = true
    var setTextLands = true
    var refreshOk = true

    override fun commitText(text: String): Boolean {
        commits += text
        if (commitThrows) throw IllegalStateException("dead connection")
        if (!commitSilentlyDropped) this.text += text
        return true
    }

    override fun getSurroundingText(before: Int, after: Int): SurroundingText? =
        if (surroundingNull) null else SurroundingText(text)

    override val packageName get() = pkg
    override fun snapshot() = NodeSnapshot(
        text = if (nodeTextNull) null else (if (showingHint && text.isEmpty()) hint else text),
        hint = hint,
        contentDescription = contentDesc,
        showingHint = showingHint && text.isEmpty(),
    )
    override fun refresh() = refreshOk
    override fun setText(text: String): Boolean {
        setTexts += text
        if (setTextLands) { this.text = text; showingHint = false }
        return true
    }
}

class FakeEnv(var field: FakeField = FakeField()) : InsertionEnvironment {
    var sessionActive = true
    var everStarted = true
    var connection: InputConnectionPort? = field
    var boundPkg: String? = field.pkg
    var focusedPkg: String? = field.pkg
    var editorType: Int? = 0x1
    var editorHintText: String? = null
    var restarted = false
    var replyContext = false
    var currentPkg: String? = field.pkg
    var startPkg: String? = field.pkg
    var focusedNode: EditableNode? = field
    var relocatedNode: EditableNode? = null

    val focusActions = mutableListOf<String>()

    /** What an `onStartInput` triggered by a focus nudge does to the world, and after how long. */
    var onNudge: (FakeEnv.() -> Unit)? = null
    var nudgeStartDelayMs = 50L
    private var pendingStart = false

    override fun isInputSessionActive() = sessionActive
    override fun hasSessionEverStarted() = everStarted
    override fun currentInputConnection() = connection
    override suspend fun awaitInputStart(timeoutMs: Long): Boolean {
        if (pendingStart && nudgeStartDelayMs <= timeoutMs) {
            delay(nudgeStartDelayMs)
            pendingStart = false
            onNudge?.invoke(this)
            return true
        }
        delay(timeoutMs)
        return false
    }
    override fun imeBoundPackage() = boundPkg
    override fun focusedInputPackage() = focusedPkg
    override fun editorInputType() = editorType
    override fun editorHint() = editorHintText
    override fun boundEditorRestarted() = restarted
    override fun isInNotificationReplyContext() = replyContext
    override fun currentPackage() = currentPkg
    override fun dictationStartPackage() = startPkg
    override fun reacquireInputSession(): Boolean { focusActions += "FOCUS"; pendingStart = onNudge != null; return true }
    override fun reacquireStaleSession(): Boolean { focusActions += "CLEAR_FOCUS+FOCUS"; pendingStart = onNudge != null; return true }
    override fun findFocusedInputNode() = focusedNode
    override fun relocateDictationStartNode() = relocatedNode

    /** Simulate a healthy session rebinding to [field]. */
    fun bindTo(f: FakeField) {
        field = f; sessionActive = true; everStarted = true; connection = f
        boundPkg = f.pkg; focusedPkg = f.pkg; focusedNode = f
    }
}
