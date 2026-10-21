package dev.flowclone.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.InputMethod
import android.graphics.Rect
import android.os.Bundle
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import dev.flowclone.insert.EditableNode
import dev.flowclone.insert.InputConnectionPort
import dev.flowclone.insert.NodeSnapshot
import dev.flowclone.insert.SurroundingText
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull

/** Editor bound to the user's real keyboard, from `onStartInput` (07 §2.1). */
data class FocusedEditor(
    val packageName: String?,
    val inputType: Int,
    val imeOptions: Int,
    val hintText: String?,
    val restarted: Boolean,
) {
    companion object { val EMPTY = FocusedEditor(null, 0, 0, null, false) }
}

/** `AccessibilityService.InputMethod` (API 33): the insertion channel. */
class ShellInputMethod(service: AccessibilityService, private val onStarted: () -> Unit) : InputMethod(service) {
    private val starts = MutableStateFlow(0L)
    private val finishes = MutableStateFlow(0L)

    @Volatile var focusedEditor = FocusedEditor.EMPTY; private set

    /** Incremented on every onStartInput (keep-alive's per-session nudge epoch). */
    val epoch: Long get() = starts.value

    override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
        try {
            super.onStartInput(attribute, restarting)
            focusedEditor = FocusedEditor(
                attribute.packageName, attribute.inputType, attribute.imeOptions,
                runCatching { attribute.hintText?.toString() }.getOrNull(), restarting,
            )
            runCatching(onStarted)
            starts.value++
        } catch (e: NullPointerException) {
            // Framework NPE seen in Wispr's crash guard: don't count this start.
        }
    }

    override fun onFinishInput() {
        super.onFinishInput()
        focusedEditor = FocusedEditor.EMPTY
        finishes.value++
        runCatching(onStarted)
    }

    val isInputSessionActive get() = starts.value > finishes.value
    val hasSessionEverStarted get() = starts.value > 0

    suspend fun awaitNextInputStart(ms: Long): Boolean {
        val c0 = starts.value
        return withTimeoutOrNull(ms) { starts.first { it > c0 }; true } ?: false
    }

    fun connectionPort(): InputConnectionPort? {
        val ic = currentInputConnection ?: return null
        return object : InputConnectionPort {
            override fun commitText(text: String): Boolean { ic.commitText(text, 1, null); return true }
            override fun getSurroundingText(before: Int, after: Int): SurroundingText? =
                ic.getSurroundingText(before, after, 0)?.let { SurroundingText(it.text?.toString()) }
        }
    }

    /** Down + up of [keyCode] on the current input connection (Enter / Tab paste segments). */
    fun sendKey(keyCode: Int): Boolean {
        val ic = currentInputConnection ?: return false
        val t = android.os.SystemClock.uptimeMillis()
        ic.sendKeyEvent(android.view.KeyEvent(t, t, android.view.KeyEvent.ACTION_DOWN, keyCode, 0))
        ic.sendKeyEvent(android.view.KeyEvent(t, t, android.view.KeyEvent.ACTION_UP, keyCode, 0))
        return true
    }

    /** Raw surrounding text + selection for field context capture. */
    fun readSurrounding(before: Int, after: Int): Triple<String, Int, Int>? {
        val st = currentInputConnection?.getSurroundingText(before, after, 0) ?: return null
        val t = st.text?.toString() ?: return null
        return Triple(t, st.selectionStart, st.selectionEnd)
    }
}

/** [EditableNode] over an `AccessibilityNodeInfo`. */
class NodeAdapter(val node: AccessibilityNodeInfo) : EditableNode {
    override val packageName: String? get() = node.packageName?.toString()
    override val isEditable: Boolean get() = FocusedNodeFinder.isEditable(node)
    override val isPassword: Boolean get() = node.isPassword
    override val inputType: Int get() = node.inputType
    override val canSetText: Boolean get() = node.actionList.any { it.id == AccessibilityNodeInfo.ACTION_SET_TEXT }

    override fun snapshot() = NodeSnapshot(
        text = node.text?.toString(),
        hint = node.hintText?.toString(),
        contentDescription = node.contentDescription?.toString(),
        showingHint = node.isShowingHintText,
    )

    override fun refresh(): Boolean = runCatching { node.refresh() }.getOrDefault(false)

    override fun setText(text: String): Boolean = node.performAction(
        AccessibilityNodeInfo.ACTION_SET_TEXT,
        Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text) },
    )
}

/** `FocusedNodeFinder` (07 §3.2) + dictation-start node identity. */
object FocusedNodeFinder {
    private const val MAX_DEPTH = 30
    private const val WALK_BUDGET_MS = 2000L

    fun isEditable(n: AccessibilityNodeInfo): Boolean {
        if (n.isEditable) return true
        val c = n.className?.toString() ?: return false
        return "EditText" in c || "TextInputEditText" in c || "AutoCompleteTextView" in c
    }

    fun isExcludedField(n: AccessibilityNodeInfo) =
        n.isPassword || dev.flowclone.policy.EditorInputType.isContentExtractionExcluded(n.inputType)

    fun find(service: AccessibilityService, root: AccessibilityNodeInfo?, keyboardVisible: Boolean): AccessibilityNodeInfo? {
        service.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.let { return it }
        service.findFocus(AccessibilityNodeInfo.FOCUS_ACCESSIBILITY)?.takeIf(::isEditable)?.let { return it }
        root ?: return null
        val deadline = System.currentTimeMillis() + WALK_BUDGET_MS
        walk(root, 0, deadline) { it.isFocused && isEditable(it) }?.let { return it }
        if (keyboardVisible) walk(root, 0, deadline) { it.isVisibleToUser && isEditable(it) }?.let { return it }
        return null
    }

    fun walk(n: AccessibilityNodeInfo, depth: Int, deadline: Long, pred: (AccessibilityNodeInfo) -> Boolean): AccessibilityNodeInfo? {
        if (depth > MAX_DEPTH || System.currentTimeMillis() > deadline) return null
        if (pred(n)) return n
        for (i in 0 until n.childCount) {
            val c = n.getChild(i) ?: continue
            walk(c, depth + 1, deadline, pred)?.let { return it }
        }
        return null
    }

    /** FNV-1a 64 over pkg, class, viewId, contentDescription (+ bounds). */
    fun identityHash(n: AccessibilityNodeInfo, withBounds: Boolean): Long {
        var h = -0x340d631b7bdddcdbL // 0xcbf29ce484222325
        fun mix(s: String?) {
            for (ch in (s ?: "\u0000")) { h = h xor ch.code.toLong(); h *= 0x100000001b3L }
            h = h xor 0x1fL; h *= 0x100000001b3L
        }
        mix(n.packageName?.toString()); mix(n.className?.toString()); mix(n.viewIdResourceName); mix(n.contentDescription?.toString())
        if (withBounds) { val r = Rect(); n.getBoundsInScreen(r); mix(r.flattenToString()) }
        return h
    }
}
