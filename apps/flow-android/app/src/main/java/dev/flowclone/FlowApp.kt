package dev.flowclone

import dev.flowclone.core.RouterErrors
import dev.flowclone.data.LastDictation
import dev.flowclone.data.CloudKeyStore
import dev.flowclone.data.RouterOpener
import dev.flowclone.data.ParakeetFiles
import dev.flowclone.data.EngineRoute
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevStore
import dev.flowclone.data.BuildCaps
import android.app.Application
import android.util.Log
import dev.flowclone.core.CoreException
import dev.flowclone.service.FlowAccessibilityService
import dev.flowclone.core.FieldContext
import dev.flowclone.core.PasteKey
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
    lateinit var dev: DevStore; private set
    /** D18: cloud STT API keys (Keystore-sealed, one vault per provider). */
    lateinit var cloudKeys: CloudKeyStore; private set
    val caps: BuildCaps = BuildCaps.current()

    /** Latest keyboard/field decision from the accessibility service (null = no allowed field). */
    val fieldFocus = MutableStateFlow<BubbleVisibilityPolicy.Decision?>(null)

    /** Last dictation's result + route (Developer page, engine badge). In-memory only (§5). */
    val lastRoute = MutableStateFlow<LastDictation?>(null)

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
            packageName, FlowAccessibilityService::class.java.name,
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
        dev = DevStore(this, caps)
        cloudKeys = CloudKeyStore(this)
        engine = EngineHolder(this)
    }

    companion object {
        fun of(ctx: android.content.Context) = ctx.applicationContext as FlowApp
    }
}

/** What FlowService needs from FlowAccessibilityService (same process). */
interface AccessibilityBridge {
    suspend fun insertTextAndVerify(text: String): Boolean
    /** Enter / Tab through the a11y InputMethod's connection (paste segments). */
    suspend fun sendKey(key: PasteKey): Boolean
    fun captureFieldContext(): FieldContext?
    fun onDictationActive(active: Boolean)
}

/**
 * Keeps the speech engine warm for the FGS lifetime (§9 `load_models` / `unload`).
 * Selection: mock engine setting → [FakeFlowCore]; else (D17) the local engine when this
 * flavour has one and the client route needs it, wrapped by the native router
 * (`nativeOpenRouter`), which also owns the server route. All sessions go to the router.
 */
class EngineHolder(
    private val app: FlowApp,
    private val loader: FlowCoreLoader = NativeFlowCore,
    private val opener: RouterOpener = NativeFlowCore.routerOpener,
) {
    sealed interface State {
        data object NotLoaded : State
        data object Loading : State
        data class Ready(val core: FlowCore, val description: String, val llm: EnginePlan.LlmStatus = EnginePlan.LlmStatus.Off) : State
        data class Failed(val message: String, val kind: String? = null) : State
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val _state = MutableStateFlow<State>(State.NotLoaded)
    val state: StateFlow<State> = _state
    /** Whether the loaded router got a local engine (decides update-in-place vs reopen on dev save). */
    @Volatile private var loadedWithLocal: Boolean? = null

    fun modelsDir() = File(app.filesDir, "models").apply { mkdirs() }
    fun parakeetDir() = File(app.filesDir, "parakeet").apply { mkdirs() }

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
        runCatching { persistCloudUsage() }
        (_state.value as? State.Ready)?.core?.unload()
        _state.value = State.NotLoaded
        loadedWithLocal = null
    }

    /** For DictationController: the warm core, or why there isn't one. */
    fun current(): FlowCore = when (val s = _state.value) {
        is State.Ready -> s.core
        State.Loading -> throw CoreException("Speech model is still loading…")
        is State.Failed -> throw CoreException(s.message, s.kind)
        State.NotLoaded -> { ensureLoaded(); throw CoreException("Speech model is loading, try again in a moment.") }
    }

    /** The router handle, when the native router is the active engine. */
    fun router(): NativeFlowCore? = ((_state.value as? State.Ready)?.core as? NativeFlowCore)?.takeIf { it.isRouter }

    /**
     * Developer settings were saved (§5 "Settings → router"): `update_config` in place, applied
     * from the next session, unless the local engine must now be loaded or dropped (reopen).
     */
    @Synchronized
    fun applyDevSettings(dev: DevSettings) {
        val r = router()
        val wantLocal = EngineRoute.needsLocal(dev.effective(app.caps), app.caps)
        if (r != null && loadedWithLocal == wantLocal && !dev.effective(app.caps).bothOff) {
            try { r.updateRouter(dev.routerConfigFor(app.caps)); return } catch (e: CoreException) { Log.w("FlowEngine", "update_config: ${e.message}") }
        }
        reload()
    }

    fun llmDir() = File(app.filesDir, "llm").apply { mkdirs() }

    /**
     * D18: a freshly opened router gets every stored provider key (keys never travel in the
     * config JSON) and this month's persisted usage counters.
     */
    private fun syncCloud(r: NativeFlowCore) {
        if (!app.caps.cloud) return
        for (p in app.dev.load().cloud.providers) {
            val key = app.cloudKeys.get(p.id) ?: continue
            runCatching { r.setCloudKey(p.id, key) }.onFailure { Log.w("FlowEngine", "set_cloud_key ${p.id}: ${it.message}") }
        }
        app.dev.cloudUsageJson?.let { r.setCloudUsage(it) }
    }

    /** A provider's key was saved / cleared on the Developer page: tell the live router. */
    fun setCloudKey(providerId: String, key: String?) {
        runCatching { router()?.setCloudKey(providerId, key) }.onFailure { Log.w("FlowEngine", "set_cloud_key $providerId: ${it.message}") }
    }

    /** Persist the router's cloud counters (after a cloud dictation, and on Reset counters). */
    fun persistCloudUsage() {
        router()?.cloudUsageJson()?.let { app.dev.cloudUsageJson = it }
    }

    private fun load(): State {
        val threads = (Runtime.getRuntime().availableProcessors() / 2).coerceIn(1, 4)
        val s = app.settings.load()
        // Parakeet derives bpe.vocab for hotwords into TMPDIR (core-architecture "Android: Parakeet" 4).
        runCatching { android.system.Os.setenv("TMPDIR", app.cacheDir.path, true) }
        val local = if (!app.caps.local && !s.mockEngine) EnginePlan.Plan.Missing("This build has no on-device engine.")
        else EnginePlan.plan(
            s, app.settings.modelPath, app.settings.llmModelPath, { File(it).isFile }, threads,
            parakeetPath = app.settings.parakeetPath, isModelDir = { ParakeetFiles.isModelDir(File(it)) },
        )
        return when (val plan = EngineRoute.plan(local, app.dev.load(), app.caps)) {
            EngineRoute.Plan.Mock -> State.Ready(FakeFlowCore("This is a test of on-device dictation."), "mock engine")
            is EngineRoute.Plan.Unavailable -> State.Failed(plan.message, plan.kind)
            is EngineRoute.Plan.Routed -> {
                val r = EngineRoute.assemble(plan, loader, opener)
                if (r.llm is EnginePlan.LlmStatus.Failed) Log.w("FlowEngine", "LLM disabled: ${(r.llm as EnginePlan.LlmStatus.Failed).message}")
                if (r.core == null) {
                    Log.w("FlowEngine", "load failed: ${r.error}")
                    State.Failed(RouterErrors.userMessage(r.errorKind, r.error ?: "Speech engine failed to load"), r.errorKind)
                } else {
                    loadedWithLocal = plan.local != null
                    (r.core as? NativeFlowCore)?.takeIf { it.isRouter }?.let(::syncCloud)
                    State.Ready(r.core, r.description, r.llm)
                }
            }
        }
    }
}
