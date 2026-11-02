package dev.flowclone

import dev.flowclone.core.RouterErrors
import dev.flowclone.debug.DebugAudioStore
import dev.flowclone.debug.DebugLogFile
import dev.flowclone.debug.DebugLogger
import dev.flowclone.core.BlockingCalls
import dev.flowclone.insert.ClipboardGuard
import dev.flowclone.service.AndroidClipPort
import dev.flowclone.data.EngineMismatch
import dev.flowclone.data.HebrewModels
import dev.flowclone.data.KeyboardLanguage
import dev.flowclone.data.LanguageDefault
import dev.flowclone.data.Settings
import dev.flowclone.data.LastDictation
import dev.flowclone.data.CloudKeyStore
import dev.flowclone.data.CloudMigration
import dev.flowclone.data.OpenRouter
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
import dev.flowclone.notes.NotesStore
import dev.flowclone.data.SuggestionStore
import dev.flowclone.data.HistoryStore
import dev.flowclone.data.InsightsStore
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
    /** Learn-from-edits suggestions waiting for Add / Dismiss (Dictionary screen). */
    lateinit var suggestions: SuggestionStore; private set
    lateinit var history: HistoryStore; private set
    /**
     * Insights page counts (`files/insights.json`; words, recording length, app per local day,
     * no text). Separate from [history]: clearing History or retention "Never" keeps them.
     */
    val insights: InsightsStore by lazy {
        InsightsStore(File(filesDir, "insights.json")) { history.statsRows() }
    }
    lateinit var engine: EngineHolder; private set
    lateinit var dev: DevStore; private set
    /** D28: the OpenRouter API key (Keystore-sealed, id [OpenRouter.ID]). */
    lateinit var cloudKeys: CloudKeyStore; private set
    /** D21: opt-in device-local debug log (Developer settings; default off). */
    lateinit var debugFile: DebugLogFile; private set
    lateinit var debugLog: DebugLogger; private set
    /** Developer "Keep last dictation audio (debug)" (default off): WAVs in files/debug/audio/. */
    lateinit var debugAudio: DebugAudioStore; private set
    /** One low-priority writer for [debugAudio]; dictation never waits on it. */
    val debugAudioWriter: java.util.concurrent.ExecutorService by lazy {
        java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "flow-debug-audio").apply { isDaemon = true; priority = Thread.MIN_PRIORITY } }
    }
    val caps: BuildCaps = BuildCaps.current()

    /** S16: every transcript the app puts on the clipboard is removed again after a minute. */
    val clipboard: ClipboardGuard by lazy {
        ClipboardGuard(AndroidClipPort(this), CoroutineScope(SupervisorJob() + Dispatchers.Main))
    }

    /** Latest keyboard/field decision from the accessibility service (null = no allowed field). */
    val fieldFocus = MutableStateFlow<BubbleVisibilityPolicy.Decision?>(null)

    /** Command Mode: the allowed focused field has a non-empty selection (the bubble shows ✎). */
    val fieldSelection = MutableStateFlow(false)

    /** Last dictation's result + route (Developer page, engine badge). In-memory only (§5). */
    val lastRoute = MutableStateFlow<LastDictation?>(null)

    /** D35: History rows being retried now (their Retry shows busy). */
    val retrying = MutableStateFlow<Set<Long>>(emptySet())

    /** D35: bumped when a retry changed a History row, so open History screens reload. */
    val historyRevision = MutableStateFlow(0L)

    /**
     * The dictation screen of ours in front right now (any note editor sets itself on resume,
     * clears on pause), or null. While set, the accessibility service treats our own field like any
     * other app's ([dev.flowclone.policy.OwnAppGate]); otherwise our screens get no bubble.
     */
    @Volatile var dictationSurface: Any? = null

    /** Notes text (the Scratchpad / Quick note is one of them) + their index, app-private `files/notes/`. */
    val documents: NotesStore by lazy { NotesStore(File(filesDir, "notes")) }

    /**
     * Every [documents] read and write runs on this one thread, in order: an editor opened right
     * after another one paused reads the text that pause just saved.
     */
    val documentsIo: java.util.concurrent.ExecutorService by lazy {
        java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "flow-documents").apply { isDaemon = true } }
    }

    /** The live accessibility service, when connected. */
    @Volatile var accessibility: AccessibilityBridge? = null

    /** Health-monitor inputs (elapsedRealtime ms). */
    val processStartAt: Long = android.os.SystemClock.elapsedRealtime()
    @Volatile var a11yConnectedAt: Long? = null
    @Volatile var a11yHeartbeatAt: Long? = null

    private val healthMonitor = AccessibilityHealth.Monitor()

    /** Ghost-service check (Wispr AccessibilityHealthMonitor), see [AccessibilityHealth]; debounced, shared by all callers. */
    fun accessibilityHealth(): AccessibilityHealth.Status {
        val enabled = A11ySetting.isEnabled(
            android.provider.Settings.Secure.getString(contentResolver, android.provider.Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            packageName, FlowAccessibilityService::class.java.name,
        )
        val screenOn = getSystemService(android.os.PowerManager::class.java)?.isInteractive ?: true
        val now = android.os.SystemClock.elapsedRealtime()
        val a = accessibility
        // A live answer from the bound service also counts as a heartbeat.
        val probe = a?.let { svc -> { svc.probeAlive().also { if (it) a11yHeartbeatAt = now } } }
        val raw = AccessibilityHealth.evaluate(
            AccessibilityHealth.Input(
                enabledInSettings = enabled, connected = accessibility != null, processStartAt = processStartAt,
                connectedAt = a11yConnectedAt, lastHeartbeatAt = a11yHeartbeatAt, screenOn = screenOn,
                now = now, liveProbe = probe,
            ),
        )
        return healthMonitor.update(raw, now)
    }

    override fun onCreate() {
        super.onCreate()
        // Review C2: a blocking native call on Main is a bug (ANR on the server route); say so loudly.
        BlockingCalls.isMainThread = { android.os.Looper.myLooper() == android.os.Looper.getMainLooper() }
        BlockingCalls.onViolation = { Log.e("FlowEngine", "blocking native call on the main thread: $it", Throwable("C2")) }
        settings = SettingsStore(this)
        dictionary = DictionaryStore(this)
        suggestions = SuggestionStore(this)
        history = HistoryStore(this)
        dev = DevStore(this, caps)
        cloudKeys = CloudKeyStore(this)
        debugFile = DebugLogFile.inFilesDir(filesDir)
        debugLog = DebugLogger(
            enabled = { dev.debugLog },
            sink = debugFile,
            logcat = { Log.i(DebugLogger.TAG, it) },
            // Scrubbed from every line, whatever path a secret took (never the clipboard: nothing reads it).
            secrets = { debugSecrets() },
        )
        // D36: before anything reads the cleanup order (AI cleanup off before the upgrade stays off).
        runCatching { dev.migrateCleanupD36(phoneAiWasOn = settings.load().llmEnabled) }
            .onFailure { Log.w("FlowEngine", "cleanup order migration: ${it.message}") }
        migrateCloud()
        engine = EngineHolder(this)
        debugAudio = DebugAudioStore.inFilesDir(filesDir)
        // 72 h limit also when the option was turned off meanwhile.
        runCatching { debugAudioWriter.execute { runCatching { debugAudio.prune() } } }
    }

    /** The language the next dictation uses ([dev.flowclone.data.LanguageDefault]). */
    fun effectiveLanguage(s: Settings = settings.load()): String = resolveLanguage(s).language

    /** [effectiveLanguage] and where it came from (D37: keyboard / phone / fixed). */
    fun resolveLanguage(s: Settings = settings.load()): LanguageDefault.Resolved =
        LanguageDefault.resolve(
            s.language, java.util.Locale.getDefault().language, engine.loadedAsr(),
            if (engine.hebrewModel() != null) setOf(HebrewModels.LANGUAGE) else emptySet(),
            keyboard = if (s.language == Settings.LANGUAGE_KEYBOARD) keyboardLanguage() else null,
        )

    /**
     * D37: the active keyboard's language (whisper code), or null when there is no keyboard
     * subtype or its language is unknown. One binder call; never throws.
     */
    @Suppress("DEPRECATION") // InputMethodSubtype.locale: only when languageTag is empty
    fun keyboardLanguage(): String? = try {
        val st = getSystemService(android.view.inputmethod.InputMethodManager::class.java)?.currentInputMethodSubtype
        st?.let { KeyboardLanguage.of(it.languageTag, it.locale) }
    } catch (_: Exception) {
        null
    }

    /**
     * D28, once: stored D18 cloud settings (schema 1) become the one OpenRouter config. The rules
     * are the router's (`nativeMigrateCloud`, pure and fast); only runs (and only loads the native
     * library this early) while a schema-1 config is stored. A failure leaves it for the next start.
     */
    @Synchronized
    fun migrateCloud() {
        if (dev.legacyCloudJson() == null) return
        val r = runCatching {
            CloudMigration.run(dev, cloudKeys) { NativeFlowCore.migrateCloud(it) }
        }.onFailure { Log.w("FlowEngine", "cloud migration: ${it.message}") }.getOrNull() ?: return
        // Ids and names only; never a key.
        debugLog.event("cloud_migration", "migrated=${r.migrated} removed=${r.removedNames.size} key_moved=${r.keyFrom != null} cloud_disabled=${r.cloudDisabled}")
    }

    /** Server token + the stored OpenRouter key (runs on the debug-log thread, never throws). */
    private fun debugSecrets(): List<String> {
        val out = ArrayList<String>()
        runCatching { dev.load() }.getOrNull()?.token?.takeIf { it.isNotEmpty() }?.let(out::add)
        runCatching { cloudKeys.get(OpenRouter.ID) }.getOrNull()?.let(out::add)
        return out
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
    /** S12: may the transcript go into the field focused now (see [dev.flowclone.policy.InsertGuard])? */
    fun insertTarget(explicit: Boolean): dev.flowclone.session.InsertTarget = dev.flowclone.session.InsertTarget.OK
    /** Health probe: does the system still answer this service (window query)? False or throwing = dead binding. */
    fun probeAlive(): Boolean = true
    /** D21 debug log: status + step trail of the last [insertTextAndVerify]. */
    val lastInsertSteps: List<String> get() = emptyList()
    /** D21 debug log: the field captured by [captureFieldContext] is a password / sensitive field. */
    val startFieldProtected: Boolean get() = false
    /**
     * Learn from my edits: a reader of the field pinned by [captureFieldContext], or null when
     * that field must not be re-read (password / sensitive / banking / no field).
     */
    fun editReader(): dev.flowclone.learn.FieldReader? = null

    /**
     * Command Mode: the focused field's text + selection, or why it is refused (password /
     * sensitive / banking / denylisted / no field). Pins the field like [captureFieldContext].
     */
    fun captureCommand(): dev.flowclone.command.CommandTarget =
        dev.flowclone.command.CommandTarget.Refused(dev.flowclone.command.CommandTarget.NO_FIELD_MESSAGE)

    /** Command Mode: replace [target]'s selection with [text] (at the cursor without one). False = field unchanged. */
    suspend fun applyCommand(target: dev.flowclone.command.CommandTarget.Field, text: String): Boolean = false
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
    /** Test seam: the route the next dictation needs (null = [wantedRoute] from the settings). */
    private val routeProbe: (() -> String?)? = null,
    /** Test seam: builds the engine for a route instead of the native load (null = the real load). */
    private val builder: ((String?) -> State)? = null,
) {
    companion object {
        /** Longest a reload waits for the previous engine's running commit (20 s UI timeout + slack). */
        const val UNLOAD_WAIT_MS = 30_000L
        /** H5: the bubble / main screen while the Hebrew model loads (also after a keyboard switch, D37). */
        const val HEBREW_LOADING = "Loading the Hebrew speech model…"
        /** D37: switching back from the Hebrew model to the default one (keyboard / language change). */
        const val DEFAULT_LOADING = "Loading the default speech model…"
    }

    sealed interface State {
        data object NotLoaded : State
        data object Loading : State
        /** [asr]: the on-device ASR the router got (null: none, e.g. server / cloud only); [mock]: the mock engine. */
        data class Ready(
            val core: FlowCore, val description: String, val llm: EnginePlan.LlmStatus = EnginePlan.LlmStatus.Off,
            val asr: dev.flowclone.data.LoadedAsr? = null, val mock: Boolean = false,
            /** D24: why the phone can't take over when the server is unreachable (null = it can). */
            val clientNote: String? = null,
            /** D24: the on-device ASR the router loads on demand (keep warm off), for the debug log. */
            val onDemandAsr: dev.flowclone.data.LoadedAsr? = null,
        ) : State
        data class Failed(val message: String, val kind: String? = null) : State
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val _state = MutableStateFlow<State>(State.NotLoaded)
    val state: StateFlow<State> = _state
    /** Whether the loaded router got a local engine (decides update-in-place vs reopen on dev save). */
    @Volatile private var loadedWithLocal: Boolean? = null

    fun modelsDir() = File(app.filesDir, "models").apply { mkdirs() }
    fun parakeetDir() = File(app.filesDir, "parakeet").apply { mkdirs() }
    fun hebrewDir() = File(app.filesDir, "hebrew").apply { mkdirs() }

    /** H5: the Hebrew slot's model when it is installed and intact (a pinned one, or D41 a custom one from the server), else null. */
    fun hebrewModel(): Pair<String, HebrewModels.Installed>? {
        val path = app.settings.hebrewModelPath ?: return null
        return HebrewModels.installed(File(path))?.let { path to it }
    }

    /** The language whose own model the phone loads for the next dictation (`he`), or null = the default engine. */
    private fun wantedRoute(s: Settings = app.settings.load()): String? =
        if (routeProbe != null) routeProbe.invoke()
        else if (!app.caps.local || s.mockEngine) null
        else HebrewModels.route(app.effectiveLanguage(s), hebrewModel() != null)

    /** The route the current (or loading) engine was planned with. */
    @Volatile private var loadedRoute: String? = null

    /** "Loading the Hebrew model…" while a load for a language model runs (bubble / main screen). */
    @Volatile var loadingLabel: String? = null; private set

    /**
     * H5 (decision 4: one model in RAM): after a language change, reload when the next
     * dictation needs a different model (Hebrew ↔ default). No-op otherwise. D37: also at every
     * dictation start, since the keyboard language can change at any time; the label is set
     * before the reload so a dictation started right now says why it has to wait.
     */
    @Synchronized
    fun ensureLanguage() {
        when (_state.value) {
            State.NotLoaded -> return
            State.Loading -> {
                // A native load can't be interrupted: a load for a model no longer needed is
                // dropped when it ends and the needed one loads next ([ensureLoaded]). Meanwhile
                // a dictation says which model it is waiting for.
                val want = wantedRoute()
                loadingLabel = labelFor(want, switching = loadingLabel != null || want != loadedRoute)
                return
            }
            else -> Unit
        }
        val want = wantedRoute()
        if (want != loadedRoute) {
            loadingLabel = labelFor(want, switching = true)
            reload()
        }
    }

    /** What a dictation started while the model for [route] loads says; [switching] = after a language change. */
    private fun labelFor(route: String?, switching: Boolean): String? = when {
        route != null -> HEBREW_LOADING
        switching -> DEFAULT_LOADING
        else -> null
    }

    /** Native library status, for the UI. */
    fun libraryState(): NativeFlowCore.LibraryState = NativeFlowCore.loadLibrary()

    @Synchronized
    fun ensureLoaded() {
        when (_state.value) {
            is State.Ready, State.Loading -> return
            else -> Unit
        }
        _state.value = State.Loading
        scope.launch {
            var next = load()
            // D37: the keyboard language changed while this load ran (it can't be interrupted):
            // never make that model the active one; free it, then load the one needed now
            // (one model in RAM: [load] waits for the free). Stays Loading throughout.
            while (true) {
                val stale = synchronized(this@EngineHolder) {
                    if (wantedRoute() == loadedRoute) {
                        _state.value = next
                        loadingLabel = null
                        null
                    } else next
                } ?: break
                Log.i("FlowEngine", "language changed during the load: dropping the ${loadedRoute ?: "default"} model")
                drop(stale)
                next = load()
            }
            // The language resolution changed with the loaded engine: load the right model now.
            if (_state.value !is State.Failed) ensureLanguage()
        }
    }

    /** Lets go of an engine that was never the active one (freed like [unload]'s). */
    private fun drop(s: State) {
        val core = (s as? State.Ready)?.core ?: return
        core.unload()
        (core as? NativeFlowCore)?.let { unloading = it }
    }

    @Synchronized
    fun reload() {
        unload()
        ensureLoaded()
    }

    /** The engine [unload] let go of; [load] waits until its native memory is really freed. */
    @Volatile private var unloading: NativeFlowCore? = null

    /**
     * Never blocks (review C1): a push/commit/poll still running keeps the native engine alive,
     * and it is freed off Main when that call returns. The next [load] waits for the free, so
     * two models are never resident at once.
     */
    @Synchronized
    fun unload() {
        runCatching { persistCloudUsage() }
        val core = (_state.value as? State.Ready)?.core
        core?.unload()
        (core as? NativeFlowCore)?.let { unloading = it }
        _state.value = State.NotLoaded
        loadedWithLocal = null
    }

    /** For DictationController: the warm core, or why there isn't one. */
    fun current(): FlowCore = when (val s = _state.value) {
        is State.Ready -> s.core
        State.Loading -> throw CoreException(loadingLabel ?: "Speech model is still loading…", CoreException.KIND_LOADING)
        is State.Failed -> throw CoreException(s.message, s.kind)
        State.NotLoaded -> { ensureLoaded(); throw CoreException("Speech model is loading, try again in a moment.", CoreException.KIND_LOADING) }
    }

    /** D24: why the phone route can't serve a fallback right now, or null. */
    fun clientNote(): String? = (_state.value as? State.Ready)?.clientNote

    /** The on-device ASR that loaded, or null (not loaded, mock, server / cloud only). */
    fun loadedAsr(): dev.flowclone.data.LoadedAsr? = (_state.value as? State.Ready)?.asr

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
            try { r.updateRouter(dev.routerConfigFor(app.caps)); return } catch (e: CoreException) {
                Log.w("FlowEngine", "update_config: ${e.message}")
                app.debugLog.event("router_config", e.message, e)
            }
        }
        reload()
    }

    fun llmDir() = File(app.filesDir, "llm").apply { mkdirs() }

    /**
     * A freshly opened router gets the stored OpenRouter key (keys never travel in the config
     * JSON) and this month's persisted usage counters. A notice from a legacy config the router
     * migrated itself (should the shell's migration not have run) is kept for the banner.
     */
    private fun syncCloud(r: NativeFlowCore) {
        if (!app.caps.cloud) return
        app.cloudKeys.get(OpenRouter.ID)?.let { key ->
            runCatching { r.setCloudKey(OpenRouter.ID, key) }.onFailure { Log.w("FlowEngine", "set_cloud_key: ${it.message}") }
        }
        app.dev.cloudUsageJson?.let { r.setCloudUsage(it) }
        r.cloudNotice()?.let { app.dev.cloudNotice = it; r.ackCloudNotice() }
    }

    /** The OpenRouter key was saved / cleared: tell the live router (D28: id `openrouter`). */
    fun setCloudKey(key: String?) {
        runCatching { router()?.setCloudKey(OpenRouter.ID, key) }.onFailure { Log.w("FlowEngine", "set_cloud_key: ${it.message}") }
    }

    /** Persist the router's cloud counters (after a cloud dictation, and on Reset counters). */
    fun persistCloudUsage() {
        router()?.cloudUsageJson()?.let { app.dev.cloudUsageJson = it }
    }

    private fun load(): State {
        unloading?.let { if (!it.awaitFreed(UNLOAD_WAIT_MS)) Log.w("FlowEngine", "previous engine still busy after ${UNLOAD_WAIT_MS} ms; loading anyway") }
        unloading = null
        val threads = (Runtime.getRuntime().availableProcessors() / 2).coerceIn(1, 4)
        val s = app.settings.load()
        // Parakeet derives bpe.vocab for hotwords into TMPDIR (core-architecture "Android: Parakeet" 4).
        runCatching { android.system.Os.setenv("TMPDIR", app.cacheDir.path, true) }
        // D28: the catalogue cache (no network here; the router's STT / cleanup resolve models from it).
        if (app.caps.cloud) runCatching { NativeFlowCore.setCloudCacheDir(app.cacheDir.path) }
        app.migrateCloud()
        val route = wantedRoute(s)
        loadingLabel = labelFor(route, switching = loadingLabel != null || route != loadedRoute)
        loadedRoute = route
        builder?.let { return it(route) }
        val hebrew = hebrewModel()?.let { (path, m) -> HebrewModels.languageModel(path, m, app.dev.parakeetDecoding) }
        val local = if (!app.caps.local && !s.mockEngine) EnginePlan.Plan.Missing("This build has no on-device engine.")
        else EnginePlan.plan(
            s, app.settings.modelPath, app.settings.llmModelPath, { File(it).isFile }, threads,
            parakeetPath = app.settings.parakeetPath, isModelDir = { ParakeetFiles.isModelDir(File(it)) },
            parakeetDecoding = app.dev.parakeetDecoding,
            language = app.effectiveLanguage(s), hebrew = hebrew,
        )
        return when (val plan = EngineRoute.plan(local, app.dev.load(), app.caps)) {
            EngineRoute.Plan.Mock -> State.Ready(FakeFlowCore("This is a test of on-device dictation."), "mock engine", mock = true)
            is EngineRoute.Plan.Unavailable -> {
                app.debugLog.event("engine_load", "${plan.kind ?: "unavailable"}: ${plan.message}")
                State.Failed(plan.message, plan.kind)
            }
            is EngineRoute.Plan.Routed -> {
                val r = EngineRoute.assemble(plan, loader, opener)
                if (r.llm is EnginePlan.LlmStatus.Failed) {
                    Log.w("FlowEngine", "LLM disabled: ${(r.llm as EnginePlan.LlmStatus.Failed).message}")
                    app.debugLog.event("llm_load", (r.llm as EnginePlan.LlmStatus.Failed).message)
                }
                if (r.core == null) {
                    Log.w("FlowEngine", "load failed: ${r.error}")
                    app.debugLog.event("engine_load", "${r.errorKind ?: "error"}: ${r.error}")
                    State.Failed(RouterErrors.userMessage(r.errorKind, r.error ?: "Speech engine failed to load"), r.errorKind)
                } else {
                    EngineMismatch.message(EnginePlan.selectedEngine(s, plan.local), plan.local, r.asr, r.description)?.let {
                        Log.w("FlowEngine", "engine mismatch: $it")
                        app.debugLog.event("engine_mismatch", it)
                    }
                    loadedWithLocal = plan.local != null
                    (r.core as? NativeFlowCore)?.takeIf { it.isRouter }?.let(::syncCloud)
                    State.Ready(r.core, r.description, r.llm, asr = r.asr, clientNote = r.clientNote, onDemandAsr = r.onDemandAsr)
                }
            }
        }
    }
}
