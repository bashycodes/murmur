package dev.flowclone.service

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.InputMethod
import android.content.Intent
import android.graphics.Rect
import android.os.Build
import android.os.SystemClock
import android.util.Log
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import android.widget.Toast
import dev.flowclone.AccessibilityBridge
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.command.CommandApply
import dev.flowclone.command.CommandField
import dev.flowclone.command.CommandGuard
import dev.flowclone.command.CommandTarget
import dev.flowclone.command.SelectionSnapshot
import dev.flowclone.core.FieldContext
import dev.flowclone.data.FieldContextBuilder
import dev.flowclone.health.AccessibilityHealth
import dev.flowclone.health.ServiceWatchdog
import dev.flowclone.insert.ClipboardPort
import dev.flowclone.insert.EditableNode
import dev.flowclone.insert.ImeKeepAlive
import dev.flowclone.insert.InputConnectionPort
import dev.flowclone.insert.InsertionCoordinator
import dev.flowclone.insert.InsertionEnvironment
import dev.flowclone.insert.V3Inserter
import dev.flowclone.policy.BubbleVisibilityPolicy
import dev.flowclone.policy.EditorInputType
import dev.flowclone.policy.InsertGuard
import dev.flowclone.policy.OwnAppGate
import dev.flowclone.policy.PolicyLists
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Keyboard-visibility watcher + insertion channel (07 §2, §3).
 *
 * Events only poke a CONFLATED channel; one loop evaluates state then sleeps 50 ms
 * (≈20 Hz), which keeps the main thread free under event storms (07 §2 notes).
 */
class FlowAccessibilityService : AccessibilityService(), AccessibilityBridge {
    private var scope = newScope()
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var inputMethod: ShellInputMethod? = null

    @Volatile private var currentPackageName: String? = null
    @Volatile private var keyboardVisible = false
    @Volatile private var replyContext = false
    @Volatile private var lastDecision: BubbleVisibilityPolicy.Decision? = null

    /** Dictation-start snapshot (07 §3.2 `dictationStartSnapshot`). */
    private data class StartSnapshot(val pkg: String?, val hash: Long?, val looseHash: Long?)
    @Volatile private var startSnapshot: StartSnapshot? = null

    private lateinit var coordinator: InsertionCoordinator
    private lateinit var keepAlive: ImeKeepAlive
    private val watchdog = ServiceWatchdog()

    private fun newScope() = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val app get() = FlowApp.of(this)

    override fun onCreateInputMethod(): InputMethod =
        ShellInputMethod(this) { wake.trySend(Unit) }.also { inputMethod = it }

    override fun onServiceConnected() {
        super.onServiceConnected()
        val env = Environment()
        coordinator = InsertionCoordinator(V3Inserter(env, SystemClock::elapsedRealtime), Clipboard(), SystemClock::elapsedRealtime)
        keepAlive = ImeKeepAlive(KeepAliveDeps(env), SystemClock::elapsedRealtime)
        connected()
        // Host the bubble. Starting the FGS from a bound a11y service is allowed.
        if (!app.settings.serviceStoppedByUser) startFlowService()
    }

    private fun connected() {
        app.accessibility = this
        app.a11yConnectedAt = SystemClock.elapsedRealtime()
        app.a11yHeartbeatAt = null
        startLoop()
        startHealthTick()
    }

    private fun startFlowService() {
        runCatching { startForegroundService(Intent(this, FlowService::class.java).setAction(FlowService.ACTION_SHOW)) }
            .onFailure { Log.w(TAG, "could not start FlowService", it) }
    }

    /**
     * Every 15 s: prove the binding is alive (a `getWindows()` probe that returns windows is
     * the heartbeat the ghost check reads), and respawn FlowService if it died unexpectedly
     * (`MainServiceWatchdog`, ≥ 5 s apart).
     */
    private fun startHealthTick() {
        scope.launch {
            while (true) {
                if (probeAlive()) app.a11yHeartbeatAt = SystemClock.elapsedRealtime()
                val wanted = android.provider.Settings.canDrawOverlays(this@FlowAccessibilityService) && !app.settings.serviceStoppedByUser
                if (watchdog.shouldRestart(wanted, FlowService.running, SystemClock.elapsedRealtime())) {
                    Log.w(TAG, "FlowService not running; restarting")
                    startFlowService()
                }
                delay(AccessibilityHealth.HEARTBEAT_INTERVAL_MS)
            }
        }
    }

    private fun startLoop() {
        scope.launch {
            while (true) {
                wake.receive()
                runCatching { evaluateCurrentState() }.onFailure { Log.w(TAG, "evaluate failed", it) }
                delay(50)
            }
        }
        wake.trySend(Unit)
    }

    override fun probeAlive(): Boolean =
        runCatching { windows.isNotEmpty() || rootInActiveWindow != null }.getOrDefault(false)

    override fun onAccessibilityEvent(event: AccessibilityEvent?) { wake.trySend(Unit) }

    override fun onInterrupt() = Unit

    override fun onUnbind(intent: Intent?): Boolean {
        teardown()
        return true // → onRebind
    }

    override fun onRebind(intent: Intent?) {
        super.onRebind(intent)
        scope = newScope()
        connected()
    }

    override fun onDestroy() {
        teardown()
        super.onDestroy()
    }

    private fun teardown() {
        if (::keepAlive.isInitialized) keepAlive.disarm()
        scope.cancel()
        if (app.accessibility === this) { app.accessibility = null; app.a11yConnectedAt = null }
        app.fieldFocus.value = null
        app.fieldSelection.value = false
    }

    // ---- 07 §2.2 evaluateCurrentState ----

    private fun evaluateCurrentState() {
        val ws = runCatching { windows }.getOrDefault(emptyList())
        val ime = ws.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }
        val imeVisible = ime != null && imeVisibleFraction(ime) >= 0.3f
        val appWindow = ws.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_APPLICATION && it.isFocused }
        val root = appWindow?.root
            ?: (if (ime != null) ws.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_SYSTEM && it.isFocused }?.root else null)
        val pkg = root?.packageName?.toString()
        currentPackageName = pkg
        keyboardVisible = imeVisible
        replyContext = BubbleVisibilityPolicy.isNotificationReplyContext(pkg, ime != null, appWindow != null)
        val editor = inputMethod?.focusedEditor ?: FocusedEditor.EMPTY
        val node = if (imeVisible) FocusedNodeFinder.find(this, root, true) else null
        val dataSensitive = Build.VERSION.SDK_INT >= 34 && node?.isAccessibilityDataSensitive == true
        val decision = BubbleVisibilityPolicy.evaluate(
            BubbleVisibilityPolicy.Input(
                inputType = editor.inputType, imeOptions = editor.imeOptions,
                packageName = if (pkg == packageName) null else pkg,
                nodePassword = node?.isPassword, imeVisible = imeVisible && !ownAppBlocked(pkg),
                notificationReplyContext = replyContext, accessibilityDataSensitive = dataSensitive,
            ),
            app.settings.load().userDenylist,
        )
        val published = decision.takeIf { it.show }
        if (published != lastDecision) {
            lastDecision = published
            app.fieldFocus.value = published
        }
        // Command Mode's ✎: the IME's selection, else the node's (apps that report one but not the other).
        val selected = published != null && (inputMethod?.hasSelection == true ||
            (node != null && node.textSelectionStart >= 0 && node.textSelectionEnd != node.textSelectionStart))
        if (app.fieldSelection.value != selected) app.fieldSelection.value = selected
    }

    /** Our own screens get no bubble and no insert, except the Scratchpad while it is in front ([OwnAppGate]). */
    private fun ownAppBlocked(pkg: String?) = OwnAppGate.blocks(pkg, packageName, app.dictationSurface != null)

    /** SoftKeyboardStateTracker: visible fraction of the IME window rect. */
    private fun imeVisibleFraction(w: AccessibilityWindowInfo): Float {
        val r = Rect(); w.getBoundsInScreen(r)
        if (r.height() <= 0) return 0f
        val screenH = resources.displayMetrics.heightPixels
        val visible = maxOf(0, screenH - r.top).coerceAtMost(r.height())
        return visible.toFloat() / r.height()
    }

    private fun rootForInsert(): AccessibilityNodeInfo? = rootInActiveWindow

    private fun focusedNode(): AccessibilityNodeInfo? = FocusedNodeFinder.find(this, rootForInsert(), keyboardVisible)

    // ---- AccessibilityBridge ----

    override suspend fun insertTextAndVerify(text: String): Boolean = withContext(Dispatchers.Main) {
        val r = coordinator.insertTextAndVerify(text)
        Log.i(TAG, "insert ${r.status} ${r.reason ?: ""} steps=${r.steps}")
        lastInsertSteps = listOfNotNull(r.status.name, r.reason?.name) + r.steps
        r.succeeded
    }

    /** D21: the last insertion's status + step trail, for the debug log. */
    @Volatile override var lastInsertSteps: List<String> = emptyList(); private set
    /** D21: the field pinned at dictation start is a password / sensitive field (debug log redacts it). */
    @Volatile override var startFieldProtected: Boolean = false; private set

    /** Pin [node] as the dictation / command target (S12 [insertTarget], learn-from-edits). */
    private fun pinStart(pkg: String?, node: AccessibilityNodeInfo?) {
        startSnapshot = StartSnapshot(
            pkg ?: node?.packageName?.toString(),
            node?.let { FocusedNodeFinder.identityHash(it, true) },
            node?.let { FocusedNodeFinder.identityHash(it, false) },
        )
    }

    override fun captureFieldContext(): FieldContext? {
        val pkg = currentPackageName
        val node = focusedNode()
        pinStart(pkg, node)
        val editor = inputMethod?.focusedEditor ?: FocusedEditor.EMPTY
        val type = editor.inputType.takeIf { editor.packageName != null } ?: node?.inputType ?: 0
        startFieldProtected = dev.flowclone.debug.DebugRedact.isProtectedField(
            type, node?.isPassword == true, Build.VERSION.SDK_INT >= 34 && node?.isAccessibilityDataSensitive == true,
        )
        if (node?.isPassword == true) return FieldContextBuilder.build(pkg, 0x81, null, 0, 0)
        val s = inputMethod?.readSurrounding(500, 500)
        if (s != null) return FieldContextBuilder.build(pkg, type, s.first, s.second, s.third)
        // IME read null → node text with selection (06 §1).
        val text = node?.let { NodeAdapter(it).snapshot() }?.let {
            dev.flowclone.insert.TextMatching.readNodeText(it, editor.hintText, emptySet())
        }
        return FieldContextBuilder.build(pkg, type, text, node?.textSelectionStart ?: -1, node?.textSelectionEnd ?: -1)
    }

    /**
     * Learn from my edits: re-read the field pinned at dictation start. Never for a password /
     * sensitive / content-excluded field or a banking app: those return no reader, and a field
     * that turns protected (or focus moving to another field or app) ends the watch before any
     * text is read.
     */
    override fun editReader(): dev.flowclone.learn.FieldReader? {
        val snap = startSnapshot ?: return null
        if (startFieldProtected || BubbleVisibilityPolicy.isBanking(snap.pkg)) return null
        return dev.flowclone.learn.FieldReader { readEditedField(snap) }
    }

    private fun readEditedField(snap: StartSnapshot): dev.flowclone.learn.FieldRead {
        val node = focusedNode() ?: return dev.flowclone.learn.FieldRead.Unavailable
        if (node.packageName?.toString() != snap.pkg) return dev.flowclone.learn.FieldRead.FocusChanged
        if (snap.looseHash != null && FocusedNodeFinder.identityHash(node, false) != snap.looseHash) return dev.flowclone.learn.FieldRead.FocusChanged
        val editor = inputMethod?.focusedEditor ?: FocusedEditor.EMPTY
        val type = editor.inputType.takeIf { editor.packageName != null } ?: node.inputType
        if (dev.flowclone.debug.DebugRedact.isProtectedField(type, node.isPassword, Build.VERSION.SDK_INT >= 34 && node.isAccessibilityDataSensitive) ||
            FocusedNodeFinder.isExcludedField(node) || EditorInputType.isContentExtractionExcluded(type)
        ) return dev.flowclone.learn.FieldRead.FocusChanged
        val text = dev.flowclone.insert.TextMatching.readNodeText(NodeAdapter(node).snapshot(), editor.hintText, emptySet())
        return text?.let { dev.flowclone.learn.FieldRead.Text(it) } ?: dev.flowclone.learn.FieldRead.Unavailable
    }

    // ---- Command Mode ----

    override fun captureCommand(): CommandTarget {
        runCatching { evaluateCurrentState() }
        val pkg = currentPackageName
        val node = focusedNode()
        pinStart(pkg, node)
        val editor = inputMethod?.focusedEditor ?: FocusedEditor.EMPTY
        val type = editor.inputType.takeIf { editor.packageName != null } ?: node?.inputType ?: 0
        val sensitive = Build.VERSION.SDK_INT >= 34 && node?.isAccessibilityDataSensitive == true
        startFieldProtected = dev.flowclone.debug.DebugRedact.isProtectedField(type, node?.isPassword == true, sensitive)
        val appPkg = pkg ?: node?.packageName?.toString()
        CommandGuard.refusal(
            hasField = node != null || inputMethod?.isInputSessionActive == true, inputType = type,
            nodePassword = node?.let(FocusedNodeFinder::isExcludedField) == true, dataSensitive = sensitive,
            pkg = appPkg, userDenylist = app.settings.load().userDenylist, ownAppBlocked = ownAppBlocked(pkg),
        )?.let {
            Log.i(TAG, "command refused")
            return CommandTarget.Refused(it)
        }
        // The node: the whole text + selection (what ACTION_SET_TEXT rewrites).
        val nodeText = node?.let { dev.flowclone.insert.TextMatching.readNodeText(NodeAdapter(it).snapshot(), editor.hintText, emptySet()) }
        val fromNode = SelectionSnapshot.of(nodeText, node?.textSelectionStart ?: -1, node?.textSelectionEnd ?: -1)
        if (fromNode?.hasSelection == true) return CommandTarget.Field(appPkg, fromNode)
        // The IME: a window around its selection (apps whose node hides the selection).
        val fromIme = inputMethod?.readSurrounding(COMMAND_WINDOW, COMMAND_WINDOW)?.let { SelectionSnapshot.of(it.first, it.second, it.third) }
        if (fromIme?.hasSelection == true) return CommandTarget.Field(appPkg, fromIme)
        // Text is selected but unreadable: refuse rather than turn "make it shorter" into new text at the cursor.
        if (inputMethod?.hasSelection == true) return CommandTarget.Refused(CommandTarget.UNREADABLE_SELECTION_MESSAGE)
        return CommandTarget.Field(appPkg, fromNode ?: fromIme)
    }

    override suspend fun applyCommand(target: CommandTarget.Field, text: String): Boolean = withContext(Dispatchers.Main) {
        val sel = target.selection?.takeIf { it.hasSelection }
            // Nothing selected: the answer goes in at the cursor like a dictation.
            ?: return@withContext insertTextAndVerify(text)
        val node = focusedNode()
        val hint = inputMethod?.focusedEditor?.hintText
        val field = object : CommandField {
            override fun readText(): String? = node?.let {
                runCatching { it.refresh() }
                dev.flowclone.insert.TextMatching.readNodeText(NodeAdapter(it).snapshot(), hint, emptySet())
            }
            override val canSetText: Boolean get() = node != null && NodeAdapter(node).canSetText
            override fun setText(text: String): Boolean = node != null && NodeAdapter(node).setText(text)
            override fun setCursor(at: Int): Boolean = node?.performAction(
                AccessibilityNodeInfo.ACTION_SET_SELECTION,
                android.os.Bundle().apply {
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, at)
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, at)
                },
            ) == true
            override fun imeSelectedText(): String? = inputMethod?.selectedText()
            override fun imeCommit(text: String): Boolean = inputMethod?.commitText(text) == true
        }
        val ok = runCatching { CommandApply.replaceSelection(sel, text, field) }.getOrDefault(false)
        lastInsertSteps = listOf(if (ok) "COMMAND_REPLACED" else "COMMAND_NOT_REPLACED")
        Log.i(TAG, "command replace ok=$ok")
        ok
    }

    override suspend fun sendKey(key: dev.flowclone.core.PasteKey): Boolean = withContext(Dispatchers.Main) {
        val code = when (key) {
            dev.flowclone.core.PasteKey.ENTER -> android.view.KeyEvent.KEYCODE_ENTER
            dev.flowclone.core.PasteKey.TAB -> android.view.KeyEvent.KEYCODE_TAB
            dev.flowclone.core.PasteKey.BREAK -> return@withContext true
        }
        val ok = inputMethod?.sendKey(code) == true
        Log.i(TAG, "sendKey $key ok=$ok")
        ok
    }

    override fun insertTarget(explicit: Boolean): dev.flowclone.session.InsertTarget {
        runCatching { evaluateCurrentState() }
        val pkg = currentPackageName
        val node = focusedNode()
        val editor = inputMethod?.focusedEditor ?: FocusedEditor.EMPTY
        // The keyboard may have closed during processing (the node tier still inserts): judge the field, not the IME.
        val allowed = BubbleVisibilityPolicy.evaluate(
            BubbleVisibilityPolicy.Input(
                inputType = editor.inputType.takeIf { editor.packageName != null } ?: node?.inputType ?: 0,
                imeOptions = editor.imeOptions,
                packageName = if (pkg == packageName) null else pkg,
                nodePassword = node?.isPassword, imeVisible = true,
                notificationReplyContext = replyContext,
                accessibilityDataSensitive = Build.VERSION.SDK_INT >= 34 && node?.isAccessibilityDataSensitive == true,
            ),
            app.settings.load().userDenylist,
        ).show && !ownAppBlocked(pkg)
        val start = startSnapshot?.let { InsertGuard.Field(it.pkg, it.looseHash) }
        val now = InsertGuard.Field(pkg ?: node?.packageName?.toString(), node?.let { FocusedNodeFinder.identityHash(it, false) })
        val r = InsertGuard.check(start, now, allowed, explicit)
        if (r != dev.flowclone.session.InsertTarget.OK) Log.i(TAG, "insert target $r (explicit=$explicit)")
        return r
    }

    override fun onDictationActive(active: Boolean) {
        scope.launch {
            if (active) keepAlive.arm(scope) else { keepAlive.disarm(); }
        }
    }

    // ---- InsertionEnvironment over the live service ----

    private inner class Environment : InsertionEnvironment {
        private fun im() = inputMethod
        override fun isInputSessionActive() = im()?.isInputSessionActive == true
        override fun hasSessionEverStarted() = im()?.hasSessionEverStarted == true
        override fun currentInputConnection(): InputConnectionPort? = im()?.connectionPort()
        override suspend fun awaitInputStart(timeoutMs: Long) = im()?.awaitNextInputStart(timeoutMs) ?: run { delay(timeoutMs); false }
        override fun imeBoundPackage() = im()?.focusedEditor?.packageName
        override fun focusedInputPackage() = findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.packageName?.toString()
        override fun editorInputType() = im()?.focusedEditor?.takeIf { it.packageName != null }?.inputType
        override fun editorHint() = im()?.focusedEditor?.hintText
        override fun boundEditorRestarted() = im()?.focusedEditor?.restarted == true
        override fun isInNotificationReplyContext() = replyContext
        override fun currentPackage() = currentPackageName
        override fun dictationStartPackage() = startSnapshot?.pkg

        private fun nudgeTarget(): AccessibilityNodeInfo? = if (replyContext) null else focusedNode()

        override fun reacquireInputSession(): Boolean =
            nudgeTarget()?.performAction(AccessibilityNodeInfo.ACTION_FOCUS) == true

        override fun reacquireStaleSession(): Boolean {
            val n = nudgeTarget() ?: return false
            val cleared = n.performAction(AccessibilityNodeInfo.ACTION_CLEAR_FOCUS)
            var focused = n.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            if (cleared && !focused) focused = n.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            return focused
        }

        override fun findFocusedInputNode(): EditableNode? = focusedNode()?.let(::NodeAdapter)

        override fun relocateDictationStartNode(): EditableNode? {
            val snap = startSnapshot ?: return null
            val root = rootForInsert() ?: return null
            val deadline = System.currentTimeMillis() + 2000
            snap.hash?.let { h ->
                FocusedNodeFinder.walk(root, 0, deadline) { FocusedNodeFinder.isEditable(it) && FocusedNodeFinder.identityHash(it, true) == h }
                    ?.let { return NodeAdapter(it) }
            }
            val loose = snap.looseHash ?: return null
            val matches = mutableListOf<AccessibilityNodeInfo>()
            FocusedNodeFinder.walk(root, 0, deadline) {
                if (FocusedNodeFinder.isEditable(it) && FocusedNodeFinder.identityHash(it, false) == loose) matches += it
                false
            }
            return matches.singleOrNull()?.let(::NodeAdapter) // unique match only
        }
    }

    private inner class KeepAliveDeps(private val env: Environment) : ImeKeepAlive.Deps {
        private fun anchorPkg() = startSnapshot?.pkg ?: currentPackageName
        private fun livenessExcluded(): Boolean {
            val t = env.editorInputType() ?: return true
            return EditorInputType.isContentExtractionExcluded(t) || BubbleVisibilityPolicy.isBanking(anchorPkg())
        }
        private fun staleProbeSurface(p: String?) = p in PolicyLists.RESTART_PRONE_READABLE
        override fun shouldSkip(): Boolean {
            val p = anchorPkg()
            return coordinator.pendingInsertions > 0 || coordinator.isLocked || !env.isInputSessionActive() ||
                replyContext || livenessExcluded() ||
                (p in PolicyLists.KNOWN_NULL_SURROUNDING_TEXT && !staleProbeSurface(p)) ||
                env.imeBoundPackage() != p
        }
        override fun connection() = env.currentInputConnection()
        override fun inputSessionEpoch() = inputMethod?.epoch ?: 0L
        override fun onStaleTick() = staleProbeSurface(anchorPkg()) && env.reacquireStaleSession()
    }

    /** 07 §3.4: ClipData with EXTRA_IS_SENSITIVE so the system clip preview stays masked; S16: removed after 60 s. */
    private inner class Clipboard : ClipboardPort {
        override fun copySensitive(text: String) {
            app.clipboard.copy(text)
            Toast.makeText(this@FlowAccessibilityService, R.string.toast_copied_for_a_minute, Toast.LENGTH_SHORT).show()
        }
    }

    companion object {
        private const val TAG = "FlowA11y"
        /** Command Mode via the IME: characters read on each side of the selection. */
        private const val COMMAND_WINDOW = 2000
    }
}
