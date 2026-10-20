package dev.flowclone

import android.app.Application
import android.util.Log
import dev.flowclone.core.CoreException
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FlowCore
import dev.flowclone.core.FlowCoreLoader
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.data.DictionaryStore
import dev.flowclone.data.HistoryStore
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.SettingsStore
import dev.flowclone.health.A11ySetting
import dev.flowclone.health.AccessibilityHealth
import dev.flowclone.policy.BubbleVisibilityPolicy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import java.io.File

/** Process-wide singletons. Single process: the a11y service and FlowService share these. */
class FlowApp : Application() {
    lateinit var settings: SettingsStore; private set
    lateinit var dictionary: DictionaryStore; private set
    lateinit var history: HistoryStore; private set
    lateinit var engine: EngineHolder; private set

    /** Latest keyboard/field decision from the accessibility service (null = no allowed field). */
    val fieldFocus = MutableStateFlow<BubbleVisibilityPolicy.Decision?>(null)

    /** The live accessibility service, when connected. */
    @Volatile var accessibility: AccessibilityBridge? = null

    /** Health-monitor inputs (elapsedRealtime ms). */
    val processStartAt: Long = android.os.SystemClock.elapsedRealtime()
    @Volatile var a11yConnectedAt: Long? = null
    @Volatile var a11yHeartbeatAt: Long? = null

    /** Ghost-service check (Wispr AccessibilityHealthMonitor), see [AccessibilityHealth]. */
    fun accessibilityHealth(): AccessibilityHealth.Status {
        val enabled = A11ySetting.isEnabled(
            android.provider.Settings.Secure.getString(contentResolver, android.provider.Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            packageName, dev.flowclone.service.FlowAccessibilityService::class.java.name,
        )
        val screenOn = getSystemService(android.os.PowerManager::class.java)?.isInteractive ?: true
        return AccessibilityHealth.evaluate(
            AccessibilityHealth.Input(
                enabledInSettings = enabled, connected = accessibility != null, processStartAt = processStartAt,
                connectedAt = a11yConnectedAt, lastHeartbeatAt = a11yHeartbeatAt, screenOn = screenOn,
                now = android.os.SystemClock.elapsedRealtime(),
            ),
        )
    }

    override fun onCreate() {
        super.onCreate()
        settings = SettingsStore(this)
        dictionary = DictionaryStore(this)
        history = HistoryStore(this)
        engine = EngineHolder(this)
    }

    companion object {
        fun of(ctx: android.content.Context) = ctx.applicationContext as FlowApp
    }
}

/** What FlowService needs from FlowAccessibilityService (same process). */
interface AccessibilityBridge {
    suspend fun insertTextAndVerify(text: String): Boolean
    fun captureFieldContext(): dev.flowclone.core.FieldContext?
    fun onDictationActive(active: Boolean)
}

/**
 * Keeps the speech engine warm for the FGS lifetime (§9 `load_models` / `unload`).
 * Selection: mock engine setting → [FakeFlowCore]; else [NativeFlowCore] with the imported model.
 */
class EngineHolder(private val app: FlowApp, private val loader: FlowCoreLoader = NativeFlowCore) {
    sealed interface State {
        data object NotLoaded : State
        data object Loading : State
        data class Ready(val core: FlowCore, val description: String, val llm: EnginePlan.LlmStatus = EnginePlan.LlmStatus.Off) : State
        data class Failed(val message: String) : State
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val _state = MutableStateFlow<State>(State.NotLoaded)
    val state: StateFlow<State> = _state

    fun modelsDir() = File(app.filesDir, "models").apply { mkdirs() }

    /** Native library status, for the UI. */
    fun libraryState(): NativeFlowCore.LibraryState = NativeFlowCore.loadLibrary()

    @Synchronized
    fun ensureLoaded() {
        when (_state.value) {
            is State.Ready, State.Loading -> return
            else -> Unit
        }
        _state.value = State.Loading
        scope.launch { _state.value = load() }
    }

    @Synchronized
    fun reload() {
        unload()
        ensureLoaded()
    }

    @Synchronized
    fun unload() {
        (_state.value as? State.Ready)?.core?.unload()
        _state.value = State.NotLoaded
    }

    /** For DictationController: the warm core, or why there isn't one. */
    fun current(): FlowCore = when (val s = _state.value) {
        is State.Ready -> s.core
        State.Loading -> throw CoreException("Speech model is still loading…")
        is State.Failed -> throw CoreException(s.message)
        State.NotLoaded -> { ensureLoaded(); throw CoreException("Speech model is loading, try again in a moment.") }
    }

    fun llmDir() = File(app.filesDir, "llm").apply { mkdirs() }

    private fun load(): State {
        val threads = (Runtime.getRuntime().availableProcessors() / 2).coerceIn(1, 4)
        val plan = EnginePlan.plan(app.settings.load(), app.settings.modelPath, app.settings.llmModelPath, { File(it).isFile }, threads)
        return when (plan) {
            EnginePlan.Plan.Mock -> State.Ready(FakeFlowCore("This is a test of on-device dictation."), "mock engine")
            is EnginePlan.Plan.Missing -> State.Failed(plan.message)
            is EnginePlan.Plan.Native -> {
                val r = EnginePlan.load(plan, loader)
                if (r.llm is EnginePlan.LlmStatus.Failed) Log.w("FlowEngine", "LLM disabled: ${r.llm.message}")
                if (r.core == null) { Log.w("FlowEngine", "load failed: ${r.error}"); State.Failed(r.error ?: "Speech engine failed to load") }
                else State.Ready(r.core, r.description, r.llm)
            }
        }
    }
}
