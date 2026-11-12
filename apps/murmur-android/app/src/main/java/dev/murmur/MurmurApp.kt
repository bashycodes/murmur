package dev.murmur

import dev.murmur.core.RouterErrors
import dev.murmur.bubble.DictationScreen
import dev.murmur.bubble.OwnScreens
import dev.murmur.debug.DebugAudioStore
import dev.murmur.debug.DebugLogFile
import dev.murmur.debug.DebugLogger
import dev.murmur.core.BlockingCalls
import dev.murmur.insert.ClipboardGuard
import dev.murmur.service.AndroidClipPort
import dev.murmur.data.EngineMismatch
import dev.murmur.data.HebrewModels
import dev.murmur.core.AsrEngine
import dev.murmur.core.LanguageModel
import dev.murmur.core.ParakeetDecoding
import dev.murmur.data.KeyboardLanguage
import dev.murmur.data.LanguageDefault
import dev.murmur.data.Settings
import dev.murmur.data.LastDictation
import dev.murmur.data.CloudKeyStore
import dev.murmur.data.CloudMigration
import dev.murmur.data.OpenRouter
import dev.murmur.data.CustomServer
import dev.murmur.data.CustomModelsKey
import dev.murmur.data.RouterOpener
import dev.murmur.data.LoadSet
import dev.murmur.data.LoadTimingsLog
import dev.murmur.data.ParakeetFiles
import dev.murmur.data.EngineRoute
import dev.murmur.data.DevSettings
import dev.murmur.data.DevStore
import dev.murmur.data.ServerHealth
import dev.murmur.data.BuildCaps
import android.app.Application
import android.util.Log
import dev.murmur.core.CoreException
import dev.murmur.service.MurmurAccessibilityService
import dev.murmur.core.FieldContext
import dev.murmur.core.PasteKey
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.MurmurCore
import dev.murmur.core.MurmurCoreLoader
import dev.murmur.session.LoadWait
import dev.murmur.session.LoadedEngine
import kotlinx.coroutines.flow.first
import dev.murmur.core.NativeMurmurCore
import dev.murmur.core.CustomModels
import dev.murmur.data.DictionaryStore
import dev.murmur.notes.NotesStore
import dev.murmur.data.SuggestionStore
import dev.murmur.data.HistoryStore
import dev.murmur.data.InsightsStore
import dev.murmur.data.EnginePlan
import dev.murmur.data.SettingsStore
import dev.murmur.data.SpeechEngines
import dev.murmur.health.A11ySetting
import dev.murmur.health.AccessibilityHealth
import dev.murmur.policy.BubbleVisibilityPolicy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import dev.murmur.calls.CallEngine
import dev.murmur.calls.AutoCalls
import dev.murmur.calls.AutoQueue
import dev.murmur.calls.CallJobService
import dev.murmur.calls.CallJobs
import dev.murmur.calls.MediaStoreRecordings
import dev.murmur.calls.CallsStore
import dev.murmur.calls.Decoders
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import java.io.File

/** Process-wide singletons. Single process: the a11y service and MurmurService share these. */
class MurmurApp : Application() {
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
        java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "murmur-debug-audio").apply { isDaemon = true; priority = Thread.MIN_PRIORITY } }
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

    /** Last server health probe (Test connection, Settings pages); in-memory only. */
    val serverHealth = ServerHealth()

    /** D35: History rows being retried now (their Retry shows busy). */
    val retrying = MutableStateFlow<Set<Long>>(emptySet())

    /** D35: bumped when a retry changed a History row, so open History screens reload. */
    val historyRevision = MutableStateFlow(0L)

    /** Which of our screens are in front (activity lifecycle): no bubble over them, except a note editor. */
    val ownScreens = OwnScreens()

    /** Notes text (the Scratchpad / Quick note is one of them) + their index, app-private `files/notes/`. */
    val documents: NotesStore by lazy { NotesStore(File(filesDir, "notes")) }

    /**
     * Every [documents] read and write runs on this one thread, in order: an editor opened right
     * after another one paused reads the text that pause just saved.
     */
    val documentsIo: java.util.concurrent.ExecutorService by lazy {
        java.util.concurrent.Executors.newSingleThreadExecutor { r -> Thread(r, "murmur-documents").apply { isDaemon = true } }
    }

    /** D46 Calls: call transcripts, app-private `files/calls/`. */
    val callsStore: CallsStore by lazy { CallsStore(File(filesDir, "calls")) }

    /**
     * D46 Calls: the one call transcription that may run (its own on-device engine, never the
     * dictation router). CallJobService keeps the app in the foreground while it runs. Tests swap it.
     */
    lateinit var calls: CallJobs

    /** D49: the auto-transcribe opt-in and queue of new call recordings (`files/calls-auto.json`). Tests swap it. */
    var autoCalls: AutoQueue
        get() = autoCallsRef ?: synchronized(this) {
            autoCallsRef ?: AutoQueue(File(filesDir, "calls-auto.json"), MediaStoreRecordings(this)).also { autoCallsRef = it }
        }
        set(v) { autoCallsRef = v }
    @Volatile private var autoCallsRef: AutoQueue? = null

    /**
     * D46: a dictation is recording or being processed (MurmurService sets it). A call transcription
     * waits between chunks while this is true, so dictation always comes first.
     */
    val dictationBusy = MutableStateFlow(false)

    /** The live accessibility service, when connected. */
    @Volatile var accessibility: AccessibilityBridge? = null

    /** Health-monitor inputs (elapsedRealtime ms). */
    val processStartAt: Long = android.os.SystemClock.elapsedRealtime()
    @Volatile var a11yConnectedAt: Long? = null
    @Volatile var a11yHeartbeatAt: Long? = null

    private val healthMonitor = AccessibilityHealth.Monitor()

    /** Ghost-service check, see [AccessibilityHealth]; debounced, shared by all callers. */
    fun accessibilityHealth(): AccessibilityHealth.Status {
        val enabled = A11ySetting.isEnabled(
            android.provider.Settings.Secure.getString(contentResolver, android.provider.Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES),
            packageName, MurmurAccessibilityService::class.java.name,
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
        registerActivityLifecycleCallbacks(OwnScreenCallbacks())
        // Review C2: a blocking native call on Main is a bug (ANR on the server route); say so loudly.
        BlockingCalls.isMainThread = { android.os.Looper.myLooper() == android.os.Looper.getMainLooper() }
        BlockingCalls.onViolation = { Log.e("MurmurEngine", "blocking native call on the main thread: $it", Throwable("C2")) }
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
            .onFailure { Log.w("MurmurEngine", "cleanup order migration: ${it.message}") }
        migrateCloud()
        engine = EngineHolder(this)
        // D68: the first dictionary word switches Parakeet to beam search, the last one back to greedy.
        dictionary.onChange = { engine.dictionaryChanged() }
        calls = CallJobs(
            callsStore,
            decoder = { r -> Decoders.open(this, android.net.Uri.parse(r.uri)) },
            engine = { lang -> CallEngine.open(this, lang) },
            yieldTo = { dictationBusy.value },
            nextAuto = { autoCalls.next(AutoCalls.language(this)) },
            onEnd = { req, end, stop ->
                req.autoId?.let { autoCalls.finished(it, end, stop) }
                CallJobService.notifyFinished(this, req, end)
            },
        )
        // D49: watch for new call recordings again (when the owner opted in); off the main thread.
        Thread({ AutoCalls.arm(this) }, "murmur-calls-arm").apply { isDaemon = true }.start()
        debugAudio = DebugAudioStore.inFilesDir(filesDir)
        // 72 h limit also when the option was turned off meanwhile.
        runCatching { debugAudioWriter.execute { runCatching { debugAudio.prune() } } }
    }

    /** The language the next dictation uses ([dev.murmur.data.LanguageDefault]). */
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
            CloudMigration.run(dev, cloudKeys) { NativeMurmurCore.migrateCloud(it) }
        }.onFailure { Log.w("MurmurEngine", "cloud migration: ${it.message}") }.getOrNull() ?: return
        // Ids and names only; never a key.
        debugLog.event("cloud_migration", "migrated=${r.migrated} removed=${r.removedNames.size} key_moved=${r.keyFrom != null} cloud_disabled=${r.cloudDisabled}")
    }

    /** Server token + the stored OpenRouter and custom-server keys (runs on the debug-log thread, never throws). */
    private fun debugSecrets(): List<String> {
        val out = ArrayList<String>()
        runCatching { dev.load() }.getOrNull()?.token?.takeIf { it.isNotEmpty() }?.let(out::add)
        runCatching { cloudKeys.get(OpenRouter.ID) }.getOrNull()?.let(out::add)
        runCatching { cloudKeys.get(CustomServer.ID) }.getOrNull()?.let(out::add)
        return out
    }

    /** Feeds [ownScreens]: every activity of ours, started to stopped, re-read on resume, pause and config changes. */
    private inner class OwnScreenCallbacks : ActivityLifecycleCallbacks {
        private val main = android.os.Handler(android.os.Looper.getMainLooper())
        private val configWatches = java.util.WeakHashMap<android.app.Activity, android.content.ComponentCallbacks>()

        private fun screen(a: android.app.Activity) = OwnScreens.Screen(dictation = a is DictationScreen, split = a.isInMultiWindowMode)

        override fun onActivityCreated(a: android.app.Activity, state: android.os.Bundle?) {
            // Split screen / free-form entered or left in place: the activity's own config changes (API 31+).
            val watch = object : android.content.ComponentCallbacks {
                override fun onConfigurationChanged(c: android.content.res.Configuration) {
                    main.post { ownScreens.refresh(a, screen(a)) }
                }
                @Deprecated("Deprecated in Java") override fun onLowMemory() = Unit
            }
            a.registerComponentCallbacks(watch)
            configWatches[a] = watch
        }
        override fun onActivityStarted(a: android.app.Activity) = ownScreens.started(a, screen(a))
        override fun onActivityResumed(a: android.app.Activity) = ownScreens.refresh(a, screen(a))
        override fun onActivityPaused(a: android.app.Activity) = ownScreens.refresh(a, screen(a))
        override fun onActivityStopped(a: android.app.Activity) = ownScreens.stopped(a, recreating = a.isChangingConfigurations)
        override fun onActivitySaveInstanceState(a: android.app.Activity, out: android.os.Bundle) = Unit
        override fun onActivityDestroyed(a: android.app.Activity) {
            configWatches.remove(a)?.let { a.unregisterComponentCallbacks(it) }
        }
    }

    companion object {
        fun of(ctx: android.content.Context) = ctx.applicationContext as MurmurApp
    }
}

/** What MurmurService needs from MurmurAccessibilityService (same process). */
interface AccessibilityBridge {
    suspend fun insertTextAndVerify(text: String): Boolean
    /** Enter / Tab through the a11y InputMethod's connection (paste segments). */
    suspend fun sendKey(key: PasteKey): Boolean
    fun captureFieldContext(): FieldContext?
    fun onDictationActive(active: Boolean)
    /** S12: may the transcript go into the field focused now (see [dev.murmur.policy.InsertGuard])? */
    fun insertTarget(explicit: Boolean): dev.murmur.session.InsertTarget = dev.murmur.session.InsertTarget.OK
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
    fun editReader(): dev.murmur.learn.FieldReader? = null

    /**
     * Command Mode: the focused field's text + selection, or why it is refused (password /
     * sensitive / banking / denylisted / no field). Pins the field like [captureFieldContext].
     */
    fun captureCommand(): dev.murmur.command.CommandTarget =
        dev.murmur.command.CommandTarget.Refused(dev.murmur.command.CommandTarget.NO_FIELD_MESSAGE)

    /** Command Mode: replace [target]'s selection with [text] (at the cursor without one). False = field unchanged. */
    suspend fun applyCommand(target: dev.murmur.command.CommandTarget.Field, text: String): Boolean = false
}

/**
 * Keeps the speech engine warm for the FGS lifetime (core `load_models` / `unload`).
 * Selection: mock engine setting → [FakeMurmurCore]; else (D17) the local engine when this
 * flavour has one and the client route needs it, wrapped by the native router
 * (`nativeOpenRouter`), which also owns the server route. All sessions go to the router.
 */
class EngineHolder(
    private val app: MurmurApp,
    private val loader: MurmurCoreLoader = NativeMurmurCore,
    private val opener: RouterOpener = NativeMurmurCore.routerOpener,
    /** Test seam: the load set the next dictation needs (null = [wantedSet] from the settings). */
    private val setProbe: (() -> LoadSet)? = null,
    /** Test seam: builds the engine for a load set instead of the native load (null = the real load). */
    private val builder: ((LoadSet) -> State)? = null,
    /** Test seam: the Parakeet search the next load needs (null = [wantedDecoding] from the settings). */
    private val decodingProbe: (() -> ParakeetDecoding?)? = null,
) {
    companion object {
        /** Longest a reload waits for the previous engine's running commit (20 s UI timeout + slack). */
        const val UNLOAD_WAIT_MS = 30_000L
        /** H5: the bubble / main screen while the Hebrew model loads (also after a keyboard switch, D37). */
        const val HEBREW_LOADING = "Loading the Hebrew speech model…"
        /** D37: switching back from the Hebrew model to the default one (keyboard / language change). */
        const val DEFAULT_LOADING = "Loading the default speech model…"
        /** D54: "Keep both languages loaded" loads the default and the Hebrew model together. */
        const val BOTH_LOADING = "Loading the English and Hebrew speech models…"
        /** D55: the bubble while a dictation waits for a load that names no model (cold start). */
        const val SPEECH_LOADING = "Loading the speech model…"
        /** D55: a dictation's wait gives up after this many unloads restarted the load (low memory). */
        const val MAX_LOAD_RESTARTS = 3
    }

    sealed interface State {
        data object NotLoaded : State
        data object Loading : State
        /** [asr]: the on-device ASR the router got (null: none, e.g. server / cloud only); [mock]: the mock engine. */
        data class Ready(
            val core: MurmurCore, val description: String, val llm: EnginePlan.LlmStatus = EnginePlan.LlmStatus.Off,
            val asr: dev.murmur.data.LoadedAsr? = null, val mock: Boolean = false,
            /** D24: why the phone can't take over when the server is unreachable (null = it can). */
            val clientNote: String? = null,
            /** D24: the on-device ASR the router loads on demand (keep warm off), for the debug log. */
            val onDemandAsr: dev.murmur.data.LoadedAsr? = null,
            /** D55: the plan loaded a phone speech model before the router opened ([EngineRoute.needsLocal]). */
            val localPlanned: Boolean = false,
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

    /** The models the phone loads for the next dictation ([LoadSet.of]). */
    private fun wantedSet(s: Settings = app.settings.load()): LoadSet =
        if (setProbe != null) setProbe.invoke()
        else if (!app.caps.local || s.mockEngine) LoadSet.Default
        else LoadSet.of(app.effectiveLanguage(s), hebrewModel() != null, s.keepBothLanguages) { defaultModelInstalled(s) }

    /**
     * The phone can transcribe the next dictation: what [wantedSet] loads for the current language
     * is installed ([SpeechEngines.phoneReady]; a Hebrew model alone counts when the language is Hebrew).
     * The one input every speech summary (drawer, Settings, Developer) takes for the phone.
     */
    fun phoneSpeechReady(s: Settings = app.settings.load()): Boolean =
        SpeechEngines.phoneReady(s, wantedSet(s)) { defaultModelInstalled(s) }

    /** D54: a default (whisper / Parakeet) model is installed, so "Keep both languages loaded" has two models to keep. */
    fun defaultModelInstalled(s: Settings = app.settings.load()): Boolean = EnginePlan.defaultInstalled(
        s, app.settings.modelPath, { File(it).isFile }, app.settings.parakeetPath, { ParakeetFiles.isModelDir(File(it)) },
    )

    /** The load set the current (or loading) engine was planned with. */
    @Volatile private var loadedSet: LoadSet = LoadSet.Default

    /**
     * D68: the English Parakeet search the next load plans ([EnginePlan.parakeetDecoding]: greedy
     * with an empty dictionary, beam once it has words), or null when no English Parakeet loads
     * (whisper, a Hebrew-only set, no on-device engine), so a dictionary edit then reloads nothing.
     */
    private fun wantedDecoding(s: Settings = app.settings.load(), set: LoadSet = wantedSet(s)): ParakeetDecoding? =
        if (decodingProbe != null) decodingProbe.invoke()
        else if (!app.caps.local || s.mockEngine || s.asrEngine != AsrEngine.PARAKEET || set is LoadSet.Language) null
        else EnginePlan.parakeetDecoding(app.dev.parakeetDecodingOverride, app.dictionary.all())

    /** The Parakeet search the current (or loading) engine was planned with ([wantedDecoding]). */
    @Volatile private var loadedDecoding: ParakeetDecoding? = null

    /** H5: the installed Hebrew model as a `ModelConfig.languages` entry (greedy unless overridden, D68), for dictation and calls. */
    fun hebrewLanguageModel(): LanguageModel? = hebrewModel()?.let { (path, m) ->
        HebrewModels.languageModel(path, m, EnginePlan.fixedDecoding(app.dev.parakeetDecodingOverride))
    }

    /**
     * D68: the dictionary was saved, so the wanted Parakeet search may have changed. The switch
     * frees the loaded core, so it never runs on the caller's (UI) thread nor while a dictation
     * records or processes: it waits until [MurmurApp.dictationBusy] is false, then [ensureLanguage]
     * (a no-op when the search did not change). A dictation started first applies it itself at
     * its start (MurmurService.beforeStart → [ensureLanguage]), before it records.
     */
    fun dictionaryChanged() {
        scope.launch {
            app.dictationBusy.first { !it }
            ensureLanguage()
        }
    }

    /** The next dictation needs a different engine than the loaded (or loading) one. */
    private fun stale(want: LoadSet = wantedSet()): Boolean = want != loadedSet || wantedDecoding(set = want) != loadedDecoding

    /** "Loading the Hebrew model…" while a load for a language model runs (bubble / main screen). */
    @Volatile var loadingLabel: String? = null; private set

    /**
     * H5 (decision 4: one model in RAM): after a language change, reload when the next
     * dictation needs a different model (Hebrew ↔ default). No-op otherwise. D37: also at every
     * dictation start, since the keyboard language can change at any time; the label is set
     * before the reload so a dictation started right now says why it has to wait. D54: with
     * both models loaded the wanted set stays [LoadSet.Both] across a switch, so nothing
     * reloads; toggling the setting changes it, so the next dictation reloads once. D68: the
     * same when the Parakeet search changes (the dictionary got its first word or lost its
     * last; a save goes through [dictionaryChanged]). The on-device LLM is kept.
     */
    @Synchronized
    fun ensureLanguage() {
        when (_state.value) {
            State.NotLoaded -> return
            State.Loading -> {
                // A native load can't be interrupted: a load for a model no longer needed is
                // dropped when it ends and the needed one loads next ([ensureLoaded]). Meanwhile
                // a dictation says which model it is waiting for.
                val want = wantedSet()
                loadingLabel = labelFor(want, switching = loadingLabel != null || want != loadedSet)
                return
            }
            else -> Unit
        }
        val want = wantedSet()
        if (stale(want)) {
            loadingLabel = labelFor(want, switching = true)
            keepLlmForSwitch((_state.value as? State.Ready)?.core)
            freeEngine()
            ensureLoaded()
        }
    }

    /** Load 3: [keepLlmForSwitch] kept an LLM; the next [load] reuses it, or frees it at its end. Guarded by this. */
    private var llmKept = false

    /**
     * Load 3: a language switch replaces only the speech model, so the on-device LLM of [core],
     * the engine about to be freed, is kept for the next load (~38 s for the 1.5B on the phone).
     * Called under this lock while that engine is still alive.
     */
    private fun keepLlmForSwitch(core: MurmurCore?) {
        if (core != null && runCatching { core.keepLlm() }.getOrDefault(false)) llmKept = true
    }

    /** What a dictation started while [set] loads says; [switching] = after a language change. */
    private fun labelFor(set: LoadSet, switching: Boolean): String? = when (set) {
        LoadSet.Both -> BOTH_LOADING
        is LoadSet.Language -> HEBREW_LOADING
        LoadSet.Default -> DEFAULT_LOADING.takeIf { switching }
    }

    /** Native library status, for the UI. */
    fun libraryState(): NativeMurmurCore.LibraryState = NativeMurmurCore.loadLibrary()

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
                    if (!stale()) {
                        _state.value = next
                        loadingLabel = null
                        null
                    } else next
                } ?: break
                Log.i("MurmurEngine", "language or dictionary changed during the load: dropping the $loadedSet model")
                synchronized(this@EngineHolder) { keepLlmForSwitch((stale as? State.Ready)?.core) }
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
        (core as? NativeMurmurCore)?.let { unloading = it }
    }

    @Synchronized
    fun reload() {
        unload()
        ensureLoaded()
    }

    /** The engine [unload] let go of; [load] waits until its native memory is really freed. */
    @Volatile private var unloading: NativeMurmurCore? = null

    /**
     * Never blocks (review C1): a push/commit/poll still running keeps the native engine alive,
     * and it is freed off Main when that call returns. The next [load] waits for the free, so
     * two models are never resident at once.
     */
    @Synchronized
    fun unload() {
        freeEngine()
        // Low-memory unload or a settings reload: no load is coming for a kept LLM. Freed off this
        // thread (it may be Main); a load that starts first takes it out of the native slot anyway.
        if (llmKept) {
            llmKept = false
            scope.launch { runCatching { loader.releaseKeptLlm() } }
        }
    }

    /** [unload] for a language switch: the LLM [keepLlmForSwitch] kept stays for the next load. */
    @Synchronized
    private fun freeEngine() {
        runCatching { persistCloudUsage() }
        val core = (_state.value as? State.Ready)?.core
        core?.unload()
        (core as? NativeMurmurCore)?.let { unloading = it }
        _state.value = State.NotLoaded
        loadedWithLocal = null
    }

    /** For DictationController: the warm core, or why there isn't one. */
    fun current(): MurmurCore = when (val s = _state.value) {
        is State.Ready -> s.core
        State.Loading -> throw CoreException(loadingLabel ?: "Speech model is still loading…", CoreException.KIND_LOADING)
        is State.Failed -> throw CoreException(s.message, s.kind)
        State.NotLoaded -> { ensureLoaded(); throw CoreException("Speech model is loading, try again in a moment.", CoreException.KIND_LOADING) }
    }

    /**
     * D55: a dictation started while [current] says "loading" records at once and starts once
     * the load ends. Null (the start is refused as before) only in the remote flavour, which
     * loads no phone model, and for a failed load. Any other state gets a wait too, so a load
     * that ended between [current] and this call is simply picked up.
     */
    fun loadWait(): LoadWait? {
        if (!app.caps.local || _state.value is State.Failed) return null
        return LoadWait(loadingLabel ?: SPEECH_LOADING, ::awaitEngine)
    }

    /**
     * D55: suspends until the load ends (a language switch meanwhile keeps it loading; an unload
     * restarts it, at most [MAX_LOAD_RESTARTS] times) and returns the engine. `phoneOnly` = the
     * load loaded the phone's speech model, so the held audio is transcribed there and never
     * sent to the server; otherwise (the D24 on-demand route, a server-only plan) the dictation
     * starts on its normal route. Throws the load's failure, or why the phone model the plan
     * wanted is missing (the router opened with the server only).
     */
    private suspend fun awaitEngine(): LoadedEngine {
        var restarts = 0
        while (true) {
            when (val s = state.first { it !is State.Loading }) {
                State.NotLoaded -> {
                    if (++restarts > MAX_LOAD_RESTARTS) throw CoreException("The speech model was unloaded while loading. Try again.", null)
                    ensureLoaded()
                }
                is State.Failed -> throw CoreException(s.message, s.kind)
                is State.Ready -> {
                    if (s.localPlanned && !s.mock && s.asr == null) {
                        throw CoreException("The phone speech model did not load" + (s.clientNote?.let { ": $it" } ?: ""), null)
                    }
                    return LoadedEngine(s.core, phoneOnly = s.mock || s.asr != null)
                }
                State.Loading -> Unit
            }
        }
    }

    /** D24: why the phone route can't serve a fallback right now, or null. */
    fun clientNote(): String? = (_state.value as? State.Ready)?.clientNote

    /** The on-device ASR that loaded, or null (not loaded, mock, server / cloud only). */
    fun loadedAsr(): dev.murmur.data.LoadedAsr? = (_state.value as? State.Ready)?.asr

    /** The router handle, when the native router is the active engine. */
    fun router(): NativeMurmurCore? = ((_state.value as? State.Ready)?.core as? NativeMurmurCore)?.takeIf { it.isRouter }

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
                Log.w("MurmurEngine", "update_config: ${e.message}")
                app.debugLog.event("router_config", e.message, e)
            }
        }
        reload()
    }

    fun llmDir() = File(app.filesDir, "llm").apply { mkdirs() }

    /**
     * A freshly opened router gets the stored OpenRouter and custom-server keys (keys never travel in the config
     * JSON) and this month's persisted usage counters. A notice from a legacy config the router
     * migrated itself (should the shell's migration not have run) is kept for the banner.
     */
    private fun syncCloud(r: NativeMurmurCore) {
        if (!app.caps.cloud) return
        for (id in listOf(OpenRouter.ID, CustomServer.ID)) app.cloudKeys.get(id)?.let { key ->
            runCatching { r.setCloudKey(id, key) }.onFailure { Log.w("MurmurEngine", "set_cloud_key: ${it.message}") }
        }
        app.dev.cloudUsageJson?.let { r.setCloudUsage(it) }
        r.cloudNotice()?.let { app.dev.cloudNotice = it; r.ackCloudNotice() }
    }

    /** A key was saved / cleared: tell the live router (D28 id `openrouter`, D52 `custom`). */
    fun setCloudKey(key: String?, id: String = OpenRouter.ID) {
        runCatching { router()?.setCloudKey(id, key) }.onFailure { Log.w("MurmurEngine", "set_cloud_key: ${it.message}") }
    }

    /**
     * D52 blocking (≤ 8 s, off the main thread): the custom server's model list with the typed [key].
     * Without one, the router lends the stored key only when [baseUrl]'s host is the consented one;
     * the shell never sends the stored key itself, so a URL typed into the page cannot receive it.
     */
    fun customCleanupModels(baseUrl: String, key: String?): CustomModels {
        val typed = CustomModelsKey.typedOnly(key)
        return router()?.customCleanupModels(baseUrl, typed) ?: NativeMurmurCore.customCleanupModels(baseUrl, typed)
    }

    /** Persist the router's cloud counters (after a cloud dictation, and on Reset counters). */
    fun persistCloudUsage() {
        router()?.cloudUsageJson()?.let { app.dev.cloudUsageJson = it }
    }

    /** Load 3: one `engine_load_timings` debug-log event (also logcat tag MurmurDebug) per native load, debug log on only. */
    private fun logLoadTimings(timings: String, set: LoadSet, threads: Int) {
        if (!app.debugLog.isEnabled()) return
        val cpus = runCatching {
            LoadTimingsLog.cpusAllowed(File("/proc/self/task/${android.os.Process.myTid()}/status").readText())
        }.getOrNull()
        app.debugLog.event("engine_load_timings", LoadTimingsLog.line(timings, set, threads, cpus))
    }

    /**
     * [loadEngine], then frees an LLM [keepLlmForSwitch] kept that this load did not take (none,
     * mock, server only). A load that failed put it back natively, so it stays kept for the retry
     * ([unload] frees it if none comes). The free runs outside the lock.
     */
    private fun load(): State {
        var s: State? = null
        try {
            s = loadEngine()
            return s
        } finally {
            val release = synchronized(this) { (llmKept && s !is State.Failed).also { if (it) llmKept = false } }
            if (release) runCatching { loader.releaseKeptLlm() }
        }
    }

    private fun loadEngine(): State {
        unloading?.let { if (!it.awaitFreed(UNLOAD_WAIT_MS)) Log.w("MurmurEngine", "previous engine still busy after ${UNLOAD_WAIT_MS} ms; loading anyway") }
        unloading = null
        val threads = (Runtime.getRuntime().availableProcessors() / 2).coerceIn(1, 4)
        val s = app.settings.load()
        // Parakeet derives bpe.vocab for hotwords into TMPDIR (core-architecture "Android: Parakeet" 4).
        runCatching { android.system.Os.setenv("TMPDIR", app.cacheDir.path, true) }
        // D28: the catalogue cache (no network here; the router's STT / cleanup resolve models from it).
        if (app.caps.cloud) runCatching { NativeMurmurCore.setCloudCacheDir(app.cacheDir.path) }
        app.migrateCloud()
        val set = wantedSet(s)
        loadingLabel = labelFor(set, switching = loadingLabel != null || set != loadedSet)
        loadedSet = set
        val decoding = wantedDecoding(s, set)
        loadedDecoding = decoding
        builder?.let { return it(set) }
        val hebrew = hebrewLanguageModel()
        val local = if (!app.caps.local && !s.mockEngine) EnginePlan.Plan.Missing("This build has no on-device engine.")
        else EnginePlan.plan(
            s, app.settings.modelPath, app.settings.llmModelPath, { File(it).isFile }, threads,
            parakeetPath = app.settings.parakeetPath, isModelDir = { ParakeetFiles.isModelDir(File(it)) },
            parakeetDecoding = decoding ?: ParakeetDecoding.GREEDY,
            language = app.effectiveLanguage(s), hebrew = hebrew, keepBoth = s.keepBothLanguages,
        )
        return when (val plan = EngineRoute.plan(local, app.dev.load(), app.caps)) {
            EngineRoute.Plan.Mock -> State.Ready(FakeMurmurCore("This is a test of on-device dictation."), "mock engine", mock = true)
            is EngineRoute.Plan.Unavailable -> {
                app.debugLog.event("engine_load", "${plan.kind ?: "unavailable"}: ${plan.message}")
                State.Failed(plan.message, plan.kind)
            }
            is EngineRoute.Plan.Routed -> {
                loader.takeLoadTimings() // a stale one from an earlier load is not this load's
                val r = EngineRoute.assemble(plan, loader, opener)
                loader.takeLoadTimings()?.let { logLoadTimings(it, set, threads) }
                if (r.llm is EnginePlan.LlmStatus.Failed) {
                    Log.w("MurmurEngine", "LLM disabled: ${(r.llm as EnginePlan.LlmStatus.Failed).message}")
                    app.debugLog.event("llm_load", (r.llm as EnginePlan.LlmStatus.Failed).message)
                }
                if (r.core == null) {
                    Log.w("MurmurEngine", "load failed: ${r.error}")
                    app.debugLog.event("engine_load", "${r.errorKind ?: "error"}: ${r.error}")
                    State.Failed(RouterErrors.userMessage(r.errorKind, r.error ?: "Speech engine failed to load"), r.errorKind)
                } else {
                    EngineMismatch.message(EnginePlan.selectedEngine(s, plan.local), plan.local, r.asr, r.description)?.let {
                        Log.w("MurmurEngine", "engine mismatch: $it")
                        app.debugLog.event("engine_mismatch", it)
                    }
                    loadedWithLocal = plan.local != null
                    (r.core as? NativeMurmurCore)?.takeIf { it.isRouter }?.let(::syncCloud)
                    State.Ready(
                        r.core, r.description, r.llm, asr = r.asr, clientNote = r.clientNote, onDemandAsr = r.onDemandAsr,
                        localPlanned = plan.local != null,
                    )
                }
            }
        }
    }
}
