package dev.flowclone.session

import dev.flowclone.command.CommandRecord
import dev.flowclone.command.CommandTarget
import dev.flowclone.core.CommandOutcome
import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.PasteKey
import dev.flowclone.core.PasteSegment
import dev.flowclone.core.RouteDiag
import dev.flowclone.core.RouterErrors
import dev.flowclone.core.SessionConfig
import dev.flowclone.core.SessionRoute
import dev.flowclone.debug.DebugLogger
import dev.flowclone.debug.DictationTrace
import dev.flowclone.debug.InsertOutcome
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** Microphone capture. [start] delivers frames on its own thread until [stop] (which drains the tail). */
interface AudioSource {
    fun start(onFrame: (ShortArray, Int) -> Unit): Boolean

    /** What the frames come from: "mic", or "wav" (debug "WAV as microphone"); debug log `source`. Read after [start]. */
    val source: String get() = "mic"

    /** Like [start]; [onError] runs once (capture thread) when the capture dies mid-session (review C15). */
    fun start(onFrame: (ShortArray, Int) -> Unit, onError: () -> Unit): Boolean = start(onFrame)

    fun stop()
}

/** S12: may the transcript go into the field focused now? */
enum class InsertTarget {
    /** Same field as at dictation start (auto insert), or an allowed field (explicit paste). */
    OK,
    /** Focus moved to another app / field since the dictation started. */
    CHANGED,
    /** The focused field is protected (banking app, password / sensitive field, user denylist). */
    PROTECTED,
}

enum class FeedbackEvent {
    StartSound, StopSound, CancelHaptic, CapWarning, CapReached, NeedMicPermission, PasteRetryFailed, NoSpeech,
    /** S12: focus moved during the dictation; nothing was inserted (Paste puts it in the field focused now). */
    TargetChanged,
    /** S12: the focused field is protected; nothing was inserted there. */
    TargetProtected,
    /** C15: the microphone stopped delivering audio; what was heard is transcribed. */
    MicLost,
    /** Command Mode failed (refused field, no AI, timeout, …): error tone; the field was not touched. */
    CommandFailed,
}

/**
 * D35: a dictation's recording, kept for History's "Retry with current settings". [appCategory]
 * = the style bucket the dictation was sent with, so a retry styles it the same way.
 */
class Recording(val pcm: ShortArray, val appCategory: String?)

/**
 * D55: the speech engine is loading. A dictation records at once, holds its audio on the phone,
 * and [await] (suspending, cancellable) returns the engine once it is loaded, or throws
 * [dev.flowclone.core.CoreException] when the load failed. [label] names the model for the bubble.
 */
class LoadWait(val label: String, val await: suspend () -> LoadedEngine)

/**
 * D55: the engine a [LoadWait] got. [phoneOnly]: the load was for the phone's speech model, so
 * the held audio is transcribed there and never sent to the server (owner decision 2026-11-05);
 * otherwise (D24 on-demand route, server-only plan) the dictation starts on its normal route.
 */
class LoadedEngine(val core: FlowCore, val phoneOnly: Boolean)

/** D35: what History's "Retry with current settings" produced. */
sealed interface RetryOutcome {
    data class Done(val result: CommitResult) : RetryOutcome
    data class Failed(val message: String) : RetryOutcome
}

/** Transient hint shown next to the bubble (live preview line). */
enum class DictationHint {
    /** Recording, but nothing above the speech threshold for 3 s: "Can't hear you". */
    NoSpeechYet,
    /** The core returned EMPTY (silence gate / blank ASR): nothing was inserted. */
    NoSpeechDetected,
    /** EMPTY with `empty_reason = only_filler`: only "um"/"uh" was heard. */
    OnlyFiller,
}

/** 10-clone-spec-android §3: Idle → Ready → Recording(hold|tap) → Processing → Ready | Error. */
sealed interface DictationState {
    /** No allowed field focused: bubble hidden. */
    data object Idle : DictationState
    data object Ready : DictationState
    /** [command] = Command Mode: the speech is an instruction for the selection, not text to type. */
    data class Recording(val hold: Boolean, val startedAt: Long, val command: Boolean = false) : DictationState
    data class Processing(val hold: Boolean, val since: Long, val command: Boolean = false) : DictationState
    data class Error(val kind: ErrorKind) : DictationState

    sealed interface ErrorKind {
        data class FailedToPaste(val text: String) : ErrorKind
        data object FailedToTranscribe : ErrorKind
        data object MicUnavailable : ErrorKind
        data class EngineUnavailable(val message: String) : ErrorKind
        /**
         * The speech model is still loading ([message] names it). Not a failure: it clears once the
         * engine settles ([DictationController.engineSettled]); a tap checks again.
         */
        data class EngineLoading(val message: String) : ErrorKind
        /** Command Mode produced nothing to write ([message] says why); the field is unchanged. */
        data class CommandFailed(val message: String) : ErrorKind
    }
}

/**
 * Platform-free dictation orchestrator: owns the state machine, the 5-min cap timer,
 * the processing timeout, the 500 ms toggle debounce and the hand-off to insertion.
 * Android specifics (FGS promotion, keep-alive arming, sounds) hang off [Host].
 *
 * Threading (review C2): every blocking core call (`startSessionInfo`, `commit`) runs on
 * [startDispatcher] / [commitDispatcher], never on the caller's (Main) thread. The mic starts
 * at once and the [StartGate] holds its frames until the session is up.
 */
class DictationController(
    private val scope: CoroutineScope,
    private val audio: AudioSource,
    private val host: Host,
    /** Loaded core; throws [CoreException] when the engine is unavailable. */
    private val core: () -> FlowCore,
    /** Where the blocking `commit()` runs (Dispatchers.Default in the app). */
    private val commitDispatcher: CoroutineDispatcher,
    /** Read at each recording start, so a settings change applies to the next session. */
    var cap: SessionCap = SessionCap(),
    private val now: () -> Long,
    /**
     * Where the blocking `start_session` runs (Dispatchers.IO in the app: the server route
     * connects there). Unconfined = inline, for tests that need no thread hop.
     */
    private val startDispatcher: CoroutineDispatcher = Dispatchers.Unconfined,
    /** Epoch ms for the D21 debug trace's `ts` (the state machine itself runs on [now]). */
    private val wallClock: () -> Long = System::currentTimeMillis,
    /**
     * D55: asked when [core] says the speech model is loading. A [LoadWait] = record now
     * and transcribe on the phone once it is loaded; null = refuse the start as before.
     */
    private val loadWait: () -> LoadWait? = { null },
) {
    constructor(
        scope: CoroutineScope, audio: AudioSource, host: Host, core: () -> FlowCore,
        commitDispatcher: CoroutineDispatcher, now: () -> Long,
    ) : this(scope, audio, host, core, commitDispatcher, SessionCap(), now)

    interface Host {
        fun feedback(e: FeedbackEvent)
        /** insertTextAndVerify (incl. clipboard fallback on failure). */
        suspend fun insert(text: String): Boolean
        /** Send Enter / Tab into the focused field after a text run (paste `segments`). */
        suspend fun sendKey(key: PasteKey): Boolean = false
        /** [outcome] = the [InsertOutcome] wire value this dictation ended with (History shows why). */
        fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?)

        /**
         * D35: [recordHistory] with the dictation's [recording] for Retry (null = not kept: off,
         * Command Mode, or a protected target). Must return at once.
         */
        fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?, recording: Recording?) =
            recordHistory(result, inserted, appId, outcome)

        /** D35: keep this dictation's recording for Retry (read at record start). */
        fun keepRetryAudio(): Boolean = false
        fun hasMicPermission(): Boolean
        /** captureFieldSnapshot at dictation start (also pins the target field, S12); null when extraction isn't allowed. */
        fun captureContext(): FieldContext?
        fun sessionConfig(): SessionConfig
        /** true at record start (promote FGS to microphone, arm keep-alive), false when audio stops. */
        fun onRecordingActive(active: Boolean)
        /** D19: trailing silence that ends a hands-free (tap) dictation, 300-10000 ms; null = never. */
        fun endpointSilenceMs(): Int? = null
        /**
         * S12: re-check the target right before inserting. [explicit] = the user pressed Paste
         * (any allowed field is fine); otherwise the field must still be the one pinned by
         * [captureContext], and it must still pass the banking / password / denylist guard.
         */
        fun insertTarget(explicit: Boolean): InsertTarget = InsertTarget.OK
        /**
         * D21: one trace per dictation outcome. Called on the controller's thread; must return at
         * once (the app enqueues it). Whatever it does or throws never changes the dictation.
         */
        fun onDictationTrace(trace: DictationTrace) = Unit

        /** Developer "Keep last dictation audio (debug)": copy this dictation's PCM (read at record start). */
        fun keepDictationAudio(): Boolean = false

        /**
         * Called with the dictation's 16 kHz mono PCM just before [onDictationTrace] when
         * [keepDictationAudio] was on (never for a cancelled dictation). Returns the saved file's
         * name for the trace's `audio` field, or null (not kept). Must return at once (enqueue).
         */
        fun onDictationAudio(trace: DictationTrace, pcm: ShortArray): String? = null

        /**
         * D24: why the phone can't take over when the server is unreachable (no model imported,
         * …), or null when it can. Appended to a "Server unreachable" error so it says exactly why.
         */
        fun clientFallbackNote(): String? = null

        /**
         * A transcript that did not go into the target ([outcome] = target_changed |
         * target_protected | paste_failed). D24 spirit (never lose a dictation): the app keeps it
         * recoverable beyond the bubble (tap-to-copy notification), because dismissing the bubble
         * or starting the next dictation drops the bubble's Paste. Must return at once.
         */
        fun onNotInserted(text: String, outcome: String) = Unit

        /**
         * Learn from my edits: every text run of [pasted] went into the field captured at start
         * ([context], null = none). [sentKey] = the plan pressed Enter (the field is likely
         * cleared). Must return at once (the app starts its watch in the background).
         */
        fun onInserted(pasted: String, context: FieldContext?, sentKey: Boolean) = Unit

        /**
         * Command Mode: read the focused field's text + selection, refusing password / sensitive
         * fields, banking apps and the denylist. Also pins the field for [insertTarget] (S12).
         */
        fun captureCommand(): CommandTarget = CommandTarget.Refused(CommandTarget.NO_FIELD_MESSAGE)

        /**
         * Command Mode: put [text] in place of [target]'s selection, keeping the text around it
         * (or at the cursor without a selection). False = nothing was changed.
         */
        suspend fun applyCommand(target: CommandTarget.Field, text: String): Boolean = false

        /** Command Mode history row (instruction, original, result). Must return at once. */
        fun recordCommand(record: CommandRecord) = Unit

        /** A start found the speech model still loading ([message] names it); every time, so each tap says so. */
        fun engineLoading(message: String) = Unit

        /** D58: the dictation language follows the keyboard (read at record start: keep the recording for [stopLanguages]). */
        fun followsKeyboard(): Boolean = false

        /**
         * D58: the dictation languages (the [SessionConfig.languages] shape) the keyboard gives
         * now, at the stop; null = not read (the start ones stand). Must return at once.
         */
        fun stopLanguages(): List<String>? = null

        /** D58: the stop language differs from the start one: make the engine serve it (loads its model if needed). */
        fun prepareStopLanguage() = Unit
    }

    /** What the debug trace needs from the dictation's start. */
    private data class TraceStart(
        val wallMs: Long, val hold: Boolean, val appId: String?, val language: String? = null,
        /** D27: the style sent for the target's bucket (debug log `style`). */
        val style: String? = null,
        /** [AudioSource.source] of this dictation (debug log `source`: mic | wav). */
        val source: String? = null,
        /** Command Mode (debug log `mode` = command). */
        val command: Boolean = false,
        /** D58: the language read at the stop ("auto" = none forced); null = not read. */
        val stopLanguage: String? = null,
    )

    companion object {
        const val TOGGLE_DEBOUNCE_MS = 500L
        const val PROCESSING_TIMEOUT_MS = 20_000L
        /** How often the preview tick polls the core (partial, endpoint, live route). */
        const val PARTIAL_POLL_MS = 250L
        const val NO_SPEECH_HINT_AFTER_MS = 3_000L
        const val HINT_MS = 2_500L
        /** `Break` segment: let the app react (e.g. open an @-mention popup) before the next run. */
        const val BREAK_PAUSE_MS = 150L
        /**
         * Command Mode: the AI's budget after the instruction is transcribed (the core's own LLM
         * timeout is 25 s; an on-demand phone model load comes on top).
         */
        const val COMMAND_TIMEOUT_MS = 35_000L
        /** D24: at most one server pre-warm per this interval (field focus can flap). */
        const val PREWARM_MIN_INTERVAL_MS = 20_000L
        /** D35: a retry transcribes a whole recording again (up to the cap) after the user asked: more room than a dictation. */
        const val RETRY_TIMEOUT_MS = 90_000L
        /** D35: samples per push when a recording is fed back (100 ms, like the microphone). */
        const val RETRY_CHUNK = 1600

        /**
         * D55: the longest a dictation waits for the speech model (an unload wait of up to 30 s,
         * then a cold Hebrew + LLM load of about a minute on the phone). A recording still
         * waiting then stops and goes to History's Retry (D35).
         */
        const val LOAD_WAIT_MS = 150_000L
        /** D55: audio held while waiting: [LOAD_WAIT_MS] plus 5 s for the start (~5 MB of PCM). */
        const val LOAD_HOLD_SAMPLES = ((LOAD_WAIT_MS + 5_000L) / 1000L * SAMPLE_RATE).toInt()
        const val LOAD_TIMEOUT = "The speech model took too long to load. Try again."

        /** D24: "Server unreachable (…)" + why the phone could not take over. */
        fun withClientNote(msg: String, kind: String?, note: String?): String =
            if (note.isNullOrBlank() || kind != RouterErrors.SERVER_UNAVAILABLE || msg.contains("phone model")) msg
            else "$msg — phone fallback unavailable: $note"

        /** A commit ERROR caused by the router (both engines off, server-only and unreachable). */
        fun routerFailure(r: CommitResult): String? {
            if (RouterErrors.plaintextRefused(r.error)) return RouterErrors.NEEDS_TLS
            val kind = CoreException.kindOf(r.error) ?: return null
            if (kind !in RouterErrors.KINDS) return null
            return RouterErrors.userMessage(kind, r.error ?: kind)
        }

        /** C4: ERROR `cancelled` = the dictation was dropped on purpose; nothing to insert or report. */
        fun isCancelled(r: CommitResult) = r.status == CommitStatus.ERROR && r.error?.trim() == "cancelled"
    }

    private val _state = MutableStateFlow<DictationState>(DictationState.Idle)
    val state: StateFlow<DictationState> = _state
    private val _level = MutableStateFlow(0f)
    val level: StateFlow<Float> = _level
    private val _partial = MutableStateFlow<String?>(null)
    /** Latest live-preview text while recording / processing (only with `partialIntervalMs`). */
    val partial: StateFlow<String?> = _partial
    private val _route = MutableStateFlow<SessionRoute?>(null)
    /** Route of the live session ("via phone | server | provider"), updated on failover; null when idle. */
    val route: StateFlow<SessionRoute?> = _route
    private val _hint = MutableStateFlow<DictationHint?>(null)
    val hint: StateFlow<DictationHint?> = _hint
    private var pollJob: Job? = null
    private var hintJob: Job? = null
    private val silence = SilenceTracker()

    private var fieldFocused = false
    private var lastToggleAt = Long.MIN_VALUE / 2
    private var capJob: Job? = null
    private var processingJob: Job? = null
    private var processingCore: FlowCore? = null
    private var session: FlowCore? = null
    /** Completes with the engine once the session started (null: start failed or the dictation was dropped). */
    private var started: Deferred<FlowCore?>? = null
    /** D55: the dictation's wait for the speech model; cancelled with the dictation. */
    private var loadJob: Job? = null
    /** D55: the dictation recording (or stopped) while its engine loads; null once it started. */
    private var waiting: Waiting? = null
    /** D55: how long the dictation waited for the model (debug log `load_wait_ms`). */
    private var loadWaitMs: Long? = null

    /**
     * D55: a dictation waiting for [wait]. [endpointing]: still recording with no engine to
     * endpoint it, so its own silence endpoint applies. [recording]: set once it stopped, so a
     * failed load keeps it for History's Retry (D35); [dictation] = not a command.
     */
    private class Waiting(val wait: LoadWait, val dictation: Boolean) {
        @Volatile var endpointing = true
        var recording: Recording? = null
    }
    private var sessionAppId: String? = null
    private var sessionContext: FieldContext? = null
    /** Bumped by every start / cancel: late frames, start results and mic errors of an older dictation are ignored. */
    @Volatile private var generation = 0L
    /** Generation whose mic frames are accepted; 0 once its audio stopped (C15: no stale tail). */
    @Volatile private var liveFrames = 0L
    private val meter = LevelMeter()
    private var traceStart: TraceStart? = null
    /** "Keep last dictation audio (debug)" or D35 Retry: this dictation's PCM; null when neither keeps it. */
    private var capture: PcmCapture? = null
    /** The [capture] is for the debug log (else only for Retry). */
    private var captureDebug = false
    /** The [capture] is for Retry. */
    private var captureRetry = false
    /** D58: the language follows the keyboard: the [capture] is for a re-run in the stop language. */
    private var captureRelang = false
    /** D58: the config the dictation started with (the re-run takes it with the stop languages). */
    private var startCfg: SessionConfig? = null
    /** D35: a History retry holds the core; no dictation starts meanwhile. */
    @Volatile var retrying = false
        private set

    /** D21: build + hand off the trace; never throws, never suspends. */
    private fun trace(
        start: TraceStart?, outcome: String, result: CommitResult? = null, stoppedAt: Long? = null,
        endpointWaitMs: Long? = null, liveRoute: SessionRoute? = null, error: String? = null,
        fallback: String? = null, cause: Throwable? = null, text: String? = null, pcm: ShortArray? = null,
        diag: RouteDiag? = null,
    ) {
        try {
            val s = start ?: return
            val t = result?.timings
            val trace = DictationTrace(
                    tsMs = s.wallMs, mode = if (s.command) "command" else if (s.hold) "hold" else "tap",
                    route = result?.route?.engine ?: liveRoute?.engine,
                    fallback = result?.route?.fallback ?: liveRoute?.fallback ?: fallback,
                    model = result?.route?.model ?: liveRoute?.model, language = s.stopLanguage ?: s.language,
                    startLanguage = s.language, stopLanguage = s.stopLanguage,
                    rttMs = result?.route?.rttMs,
                    audioMs = t?.audioMs, endpointWaitMs = endpointWaitMs, asrMs = t?.asrMs,
                    postMs = t?.let { it.llmMs + it.postprocessMs }, totalMs = t?.totalMs,
                    perceivedMs = stoppedAt?.let { now() - it },
                    waitMs = t?.waitMs, segments = t?.segments, preAsrMs = t?.preAsrMs, clientMs = result?.route?.clientMs,
                    connectMs = diag?.connectMs, localCold = diag?.localCold?.takeIf { it }, localLoadMs = diag?.localLoadMs,
                    target = s.appId?.ifEmpty { null }, insert = InsertOutcome(outcome),
                    raw = result?.asrText?.ifEmpty { null }, text = text ?: result?.historyText?.ifEmpty { null },
                    error = error ?: result?.error, stack = DebugLogger.stackOf(cause),
                    style = s.style, llm = dev.flowclone.data.LlmOutcome.code(result),
                    speechMs = t?.speechMs, peakDbfs = t?.peakDbfs,
                    llmEngine = result?.route?.llmEngine ?: result?.llm?.engine, llmModel = result?.route?.llmModel ?: result?.llm?.model,
                    llmCostUsd = result?.route?.llmCostUsd ?: result?.llm?.costUsd, llmTtftMs = result?.llm?.ttftMs,
                    llmGenerationId = result?.llm?.generationId,
                    cleanupRoute = result?.route?.cleanupRoute, cloudFallback = result?.route?.cloudFallback,
                    rejectedText = result?.route?.rejectedText, guardDetail = result?.route?.guardDetail,
                    sttCostUsd = result?.route?.takeIf { it.engine == "cloud" }?.costUsd,
                    source = s.source, loadWaitMs = loadWaitMs,
                )
            val audioName = if (pcm == null || pcm.isEmpty()) null else try { host.onDictationAudio(trace, pcm) } catch (_: Throwable) { null }
            host.onDictationTrace(if (audioName != null) trace.copy(audio = audioName) else trace)
        } catch (_: Throwable) {
            // D21: logging never affects dictation.
        }
    }

    private fun restingState(): DictationState = if (fieldFocused) DictationState.Ready else DictationState.Idle

    fun onFieldFocusChanged(focused: Boolean) {
        fieldFocused = focused
        val s = _state.value
        when {
            s == DictationState.Idle || s == DictationState.Ready -> {
                _state.value = restingState()
                if (focused) prewarm()
            }
            // "Still loading" is no error to keep: without a field there is no bubble.
            s is DictationState.Error && s.kind is DictationState.ErrorKind.EngineLoading -> if (!focused) _state.value = restingState()
            else -> Unit // recording / processing / error survive focus changes
        }
    }

    private var lastPrewarmAt = Long.MIN_VALUE / 2

    /**
     * D24: the bubble is about to be used: open the server connection now so the handshake is
     * off the dictation's critical path (and a dead server opens the circuit, so the dictation
     * goes straight to the phone). Off the caller's thread; never affects dictation.
     */
    private fun prewarm() {
        val t = now()
        if (t - lastPrewarmAt < PREWARM_MIN_INTERVAL_MS) return
        lastPrewarmAt = t
        scope.launch(startDispatcher) {
            try { core().prewarm() } catch (_: Throwable) { }
        }
    }

    /** Tap (hold=false) or long-press start (hold=true). Debounced 500 ms. */
    fun toggle(hold: Boolean) {
        val t = now()
        if (t - lastToggleAt < TOGGLE_DEBOUNCE_MS) return
        lastToggleAt = t
        when (_state.value) {
            // A previous start still connecting (it was cancelled): one session start at a time.
            DictationState.Idle, DictationState.Ready, is DictationState.Error -> if (started?.isActive != true && !retrying) start(hold)
            is DictationState.Recording -> stopAndSubmit()
            is DictationState.Processing -> Unit
        }
    }

    /**
     * Command Mode button: starts a hands-free command (the instruction ends on silence or ✓,
     * like a tap dictation), or stops one. Debounced with [toggle].
     */
    fun toggleCommand() {
        val t = now()
        if (t - lastToggleAt < TOGGLE_DEBOUNCE_MS) return
        when (_state.value) {
            DictationState.Idle, DictationState.Ready, is DictationState.Error -> {
                if (started?.isActive == true || retrying) return
                lastToggleAt = t
                when (val target = host.captureCommand()) {
                    is CommandTarget.Refused -> failCommand(target.message)
                    is CommandTarget.Field -> start(hold = false, command = target)
                }
            }
            is DictationState.Recording -> { lastToggleAt = t; stopAndSubmit() }
            is DictationState.Processing -> Unit
        }
    }

    /** Command Mode outcome without text: error tone + the reason; the field is left as it was. */
    private fun failCommand(message: String) {
        host.feedback(FeedbackEvent.CommandFailed)
        _state.value = DictationState.Error(DictationState.ErrorKind.CommandFailed(message))
    }

    /** Finger lifted after a hold: always stop + submit (not debounced). Also the ✓ button. */
    fun release() {
        if (_state.value is DictationState.Recording) stopAndSubmit()
    }

    /** X button: discards a recording, or abandons processing (C4: the core aborts the commit; late result dropped). */
    fun cancel() {
        if (_state.value is DictationState.Processing) {
            trace(traceStart, InsertOutcome.CANCELLED, liveRoute = _route.value)
            generation++
            waiting = null
            loadJob?.cancel(); loadJob = null
            processingJob?.cancel(); processingJob = null
            processingCore?.cancel(); processingCore = null
            _partial.value = null
            _route.value = null
            host.feedback(FeedbackEvent.CancelHaptic)
            _state.value = restingState()
            return
        }
        if (_state.value !is DictationState.Recording) return
        trace(traceStart, InsertOutcome.CANCELLED, liveRoute = _route.value)
        capture = null
        commandTarget = null
        generation++
        // D55: stop waiting for the model (the load itself goes on); the held audio is dropped.
        waiting = null
        loadJob?.cancel(); loadJob = null
        endAudio()
        _partial.value = null
        _hint.value = null
        _route.value = null
        // Started: cancel it now. Still starting: the start sees the new generation and cancels itself.
        if (started?.isCompleted == true) session?.cancel()
        session = null
        host.feedback(FeedbackEvent.CancelHaptic)
        host.feedback(FeedbackEvent.StopSound)
        _state.value = restingState()
    }

    fun pasteRetry() {
        val s = _state.value
        if (s !is DictationState.Error || s.kind !is DictationState.ErrorKind.FailedToPaste) return
        val text = s.kind.text
        scope.launch {
            // S12: an explicit paste may go into another field, never into a protected one.
            if (host.insertTarget(explicit = true) == InsertTarget.PROTECTED) {
                host.feedback(FeedbackEvent.TargetProtected)
                return@launch
            }
            if (host.insert(text)) _state.value = restingState()
            else host.feedback(FeedbackEvent.PasteRetryFailed)
        }
    }

    /**
     * History → "Insert with the bubble": arm the bubble's ✓ insert ([pasteRetry]: the field
     * focused when ✓ is tapped, never a protected one) with a saved [text]. Only while resting,
     * so it never interrupts a dictation or replaces a pending not-inserted text.
     */
    fun offerInsert(text: String): Boolean {
        if (text.isBlank()) return false
        val s = _state.value
        if (s != DictationState.Idle && s != DictationState.Ready) return false
        _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
        return true
    }

    /**
     * The engine finished loading: [failure] = why it failed, null = ready. A "still loading"
     * bubble goes back to rest (or shows the real error); any other state is left alone.
     */
    fun engineSettled(failure: String?) {
        val s = _state.value
        if (s !is DictationState.Error || s.kind !is DictationState.ErrorKind.EngineLoading) return
        _state.value = if (failure == null) restingState() else DictationState.Error(DictationState.ErrorKind.EngineUnavailable(failure))
    }

    fun dismissError() {
        if (_state.value is DictationState.Error) _state.value = restingState()
    }

    /**
     * D35 History "Retry with current settings": transcribe [pcm] again through the full
     * pipeline with the settings active now ([Host.sessionConfig]), styled for [ctx] (the
     * original app; no field text). Never inserts anything. Refused while a dictation records,
     * processes or starts, or another retry runs.
     */
    suspend fun retry(pcm: ShortArray, ctx: FieldContext?): RetryOutcome {
        if (retrying) return RetryOutcome.Failed(dev.flowclone.data.RetryLogic.RUNNING)
        val s = _state.value
        if (s is DictationState.Recording || s is DictationState.Processing || started?.isActive == true) {
            return RetryOutcome.Failed(dev.flowclone.data.RetryLogic.BUSY)
        }
        if (pcm.isEmpty()) return RetryOutcome.Failed(dev.flowclone.data.RetryLogic.GONE)
        retrying = true
        try {
            val c = try {
                core()
            } catch (e: CoreException) {
                return RetryOutcome.Failed(RouterErrors.userMessage(e.kind, e.message ?: "engine unavailable"))
            }
            val cfg = host.sessionConfig().copy(maxSamples = cap.maxSamples, endpointSilenceMs = null, partialIntervalMs = null)
            // Not a child: a hung native call must be abandonable.
            val pending = CoroutineScope(commitDispatcher).async {
                c.startSessionInfo(cfg)
                ctx?.let(c::setContext)
                var i = 0
                while (i < pcm.size) {
                    val n = minOf(RETRY_CHUNK, pcm.size - i)
                    c.pushAudio(pcm.copyOfRange(i, i + n), n)
                    i += n
                }
                c.commit()
            }
            val r = try {
                withTimeoutOrNull(RETRY_TIMEOUT_MS) { pending.await() }
            } catch (e: CoreException) {
                val msg = withClientNote(RouterErrors.userMessage(e.kind, e.message ?: "start failed"), e.kind, runCatching { host.clientFallbackNote() }.getOrNull())
                return RetryOutcome.Failed(msg)
            }
            if (r == null) {
                c.cancel()
                return RetryOutcome.Failed("No result after ${RETRY_TIMEOUT_MS / 1000} s")
            }
            if (isCancelled(r)) return RetryOutcome.Failed("Cancelled")
            if (r.status == CommitStatus.ERROR) {
                val routed = routerFailure(r)?.let { withClientNote(it, CoreException.kindOf(r.error), runCatching { host.clientFallbackNote() }.getOrNull()) }
                return RetryOutcome.Failed(routed ?: dev.flowclone.data.RetryLogic.failure(r)!!)
            }
            dev.flowclone.data.RetryLogic.failure(r)?.let { return RetryOutcome.Failed(it) }
            return RetryOutcome.Done(r)
        } finally {
            retrying = false
        }
    }

    /** Command Mode target of the recording / processing command; null for a dictation. */
    private var commandTarget: CommandTarget.Field? = null

    private fun start(hold: Boolean, command: CommandTarget.Field? = null) {
        if (!host.hasMicPermission()) {
            host.feedback(FeedbackEvent.NeedMicPermission)
            return
        }
        // D55: while the speech model loads, record anyway and wait for it.
        var wait: LoadWait? = null
        val ready: FlowCore? = try {
            core()
        } catch (e: CoreException) {
            if (e.kind == CoreException.KIND_LOADING) wait = try { loadWait() } catch (_: Exception) { null }
            if (wait == null) { refuseStart(hold, e); return }
            null
        }
        val lw = wait?.let { Waiting(it, dictation = command == null) }
        val handsFree = !hold
        val base = host.sessionConfig()
        val cfg = base.copy(
            maxSamples = cap.maxSamples,
            // D19: only hands-free sessions end themselves on silence; push-to-talk leaves it unset.
            endpointSilenceMs = if (handsFree) host.endpointSilenceMs() else null,
            // Command Mode: the instruction is transcribed as spoken (no cleanup; Windows does the same).
            rawMode = base.rawMode || command != null,
        )
        val gen = ++generation
        // A command's field was read by captureCommand; its text is not dictation context.
        val ctx = if (command != null) null else host.captureContext()
        commandTarget = command
        sessionAppId = command?.appId ?: ctx?.appId
        sessionContext = ctx
        traceStart = TraceStart(
            wallClock(), hold, sessionAppId, cfg.languages.firstOrNull() ?: "auto",
            style = cfg.styles.forCategory(ctx?.appCategory ?: "other").wire,
            command = command != null,
        )
        session = ready
        waiting = lw
        loadWaitMs = null
        meter.reset()
        silence.reset()
        captureDebug = runCatching { host.keepDictationAudio() }.getOrDefault(false)
        captureRetry = command == null && runCatching { host.keepRetryAudio() }.getOrDefault(false)
        captureRelang = runCatching { host.followsKeyboard() }.getOrDefault(false)
        startCfg = cfg
        val pcmCapture = if (captureDebug || captureRetry || captureRelang) PcmCapture(cap.maxSamples.toInt()) else null
        capture = pcmCapture
        hintJob?.cancel(); _hint.value = null; _partial.value = null; _route.value = null
        // D55: up to the whole load wait is held (the wait gives up before the gate is full).
        val gate = if (lw != null) StartGate(LOAD_HOLD_SAMPLES) else StartGate()
        val endpointMs = cfg.endpointSilenceMs
        host.onRecordingActive(true)
        liveFrames = gen
        val ok = audio.start({ pcm, n ->
            if (liveFrames != gen) return@start // C15: a late tail of a stopped or older dictation
            gate.push(pcm, n)
            pcmCapture?.append(pcm, n)
            _level.value = meter.update(pcm, n)
            silence.update(pcm, n)
            val h = _hint.value
            if (silence.heardSpeech) { if (h == DictationHint.NoSpeechYet) _hint.value = null }
            else if (h == null && silence.leadingSilenceMs >= NO_SPEECH_HINT_AFTER_MS) _hint.value = DictationHint.NoSpeechYet
            // D55: no engine to endpoint yet; a hands-free dictation ends on the same silence here.
            if (lw != null && lw.endpointing && endpointMs != null && silence.heardSpeech && silence.trailingSilenceMs >= endpointMs) {
                lw.endpointing = false
                scope.launch { if (generation == gen && _state.value is DictationState.Recording) stopAndSubmit() }
            }
        }, {
            scope.launch {
                if (generation == gen && _state.value is DictationState.Recording) {
                    host.feedback(FeedbackEvent.MicLost)
                    stopAndSubmit()
                }
            }
        })
        traceStart = traceStart?.copy(source = runCatching { audio.source }.getOrNull())
        if (!ok) {
            generation++
            waiting = null
            liveFrames = 0
            session = null
            capture = null
            commandTarget = null
            host.onRecordingActive(false)
            trace(traceStart, InsertOutcome.START_FAILED, error = "microphone unavailable")
            _state.value = DictationState.Error(DictationState.ErrorKind.MicUnavailable)
            return
        }
        host.feedback(FeedbackEvent.StartSound)
        _state.value = DictationState.Recording(hold, now(), command = command != null)
        // D55: the bubble names the model the recording waits for.
        if (lw != null) _route.value = SessionRoute("local", loading = true, loadingLabel = lw.wait.label)
        // C2: the (possibly network-bound) start runs off the caller's thread; frames wait in the gate.
        started = scope.async(start = CoroutineStart.UNDISPATCHED) {
            val loaded = if (lw == null) null else awaitLoad(lw, gen) ?: return@async null
            val c = loaded?.core ?: ready!!
            val info = try {
                // D55: audio held for the phone's model is transcribed on the phone, never the server.
                withContext(startDispatcher) { if (loaded?.phoneOnly == true) c.startSessionLocal(cfg) else c.startSessionInfo(cfg) }
            } catch (e: CoreException) {
                if (generation == gen) failStart(e)
                return@async null
            }
            if (generation != gen) { c.cancel(); return@async null } // cancelled while connecting
            if (lw != null) {
                waiting = null
                if (_state.value is DictationState.Recording) session = c // X cancels it from now on
            }
            ctx?.let(c::setContext)
            // Off the caller's thread: up to [LOAD_HOLD_SAMPLES] of held audio replays here.
            if (lw == null) gate.open { pcm, n -> c.pushAudio(pcm, n) }
            else withContext(startDispatcher) { gate.open { pcm, n -> c.pushAudio(pcm, n) } }
            // D24: "Loading phone model…" while the router loads the on-device engine on demand.
            if (_route.value == null || lw != null) _route.value = info.route?.copy(loading = info.loading)
            c
        }
        val preview = cfg.partialIntervalMs != null
        pollJob = scope.launch {
            val c = started?.await() ?: return@launch
            while (generation == gen) {
                delay(PARTIAL_POLL_MS)
                if (generation != gen || _state.value !is DictationState.Recording) break
                val p = c.poll()
                if (preview) p.partial?.takeIf { it.isNotBlank() }?.let { _partial.value = it.trim() }
                // Absent route mid-recording = the session is dead: keep the last label (the commit reports it).
                p.route?.copy(loading = p.loading)?.let { if (it != _route.value) _route.value = it }
                if (handsFree && p.endpointReached) {
                    stopAndSubmit() // D19: exactly as if the user tapped stop
                    break
                }
            }
        }
        capJob = scope.launch {
            delay(cap.warnAtMs)
            host.feedback(FeedbackEvent.CapWarning)
            delay(cap.stopAtMs - cap.warnAtMs)
            host.feedback(FeedbackEvent.CapReached)
            stopAndSubmit()
        }
    }

    /** No engine to start on: the still-loading notice (a tap checks again), or why there is none. */
    private fun refuseStart(hold: Boolean, e: CoreException) {
        val msg = RouterErrors.userMessage(e.kind, e.message ?: "engine unavailable")
        loadWaitMs = null
        trace(TraceStart(wallClock(), hold, null), InsertOutcome.START_FAILED, error = msg, fallback = e.kind, cause = e)
        if (e.kind == CoreException.KIND_LOADING) {
            // Not a failure: the bubble shows "loading" until the engine settles; a tap checks again.
            host.engineLoading(msg)
            _state.value = DictationState.Error(DictationState.ErrorKind.EngineLoading(msg))
            return
        }
        _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(msg))
    }

    /**
     * D55: waits (cancellably, at most [LOAD_WAIT_MS]) for the speech model [w] names. The
     * engine, or null when the dictation was dropped meanwhile or the load failed (shown, and
     * the recording kept for Retry by [failStart]).
     */
    private suspend fun awaitLoad(w: Waiting, gen: Long): LoadedEngine? {
        val t0 = now()
        val out = CompletableDeferred<Result<LoadedEngine?>>()
        val job = scope.launch {
            out.complete(
                try { Result.success(withTimeoutOrNull(LOAD_WAIT_MS) { w.wait.await() }) }
                catch (e: CancellationException) { throw e }
                catch (e: Exception) { Result.failure(e) },
            )
        }
        job.invokeOnCompletion { out.complete(Result.success(null)) } // cancelled: no engine
        loadJob = job
        val r = out.await()
        if (loadJob === job) loadJob = null
        if (generation != gen) return null // cancelled while loading
        w.endpointing = false
        loadWaitMs = now() - t0
        r.exceptionOrNull()?.let { e ->
            failStart(e as? CoreException ?: CoreException(e.message ?: "Speech model failed to load", null))
            return null
        }
        return r.getOrNull() ?: run {
            failStart(CoreException(LOAD_TIMEOUT, null))
            null
        }
    }

    /**
     * The session could not start (router: no engine, server-only and unreachable, …). D55: a
     * dictation recorded while its model loaded is not dropped: it goes to History for Retry (D35).
     */
    private fun failStart(e: CoreException) {
        val msg = withClientNote(RouterErrors.userMessage(e.kind, e.message ?: "start failed"), e.kind, runCatching { host.clientFallbackNote() }.getOrNull())
        trace(traceStart, InsertOutcome.START_FAILED, error = msg, fallback = e.kind, cause = e)
        if (_state.value is DictationState.Recording) endAudio()
        val lw = waiting
        waiting = null
        if (lw != null && lw.dictation) {
            val recording = lw.recording
                ?: capture?.take()?.takeIf { captureRetry && it.isNotEmpty() }?.let { Recording(it, sessionContext?.appCategory) }
            runCatching {
                host.recordHistory(CommitResult(CommitStatus.ERROR, error = msg), false, sessionAppId, InsertOutcome.ERROR, recording)
            }
        }
        session = null
        capture = null
        commandTarget = null
        _route.value = null
        _partial.value = null
        // e.g. router `no_engine_enabled`: "Dictation disabled — enable server or client …"
        _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(msg))
    }

    private fun endAudio() {
        capJob?.cancel(); capJob = null
        pollJob?.cancel(); pollJob = null
        audio.stop() // drains the tail into the session
        liveFrames = 0
        if (_hint.value == DictationHint.NoSpeechYet) _hint.value = null
        _level.value = 0f
        host.onRecordingActive(false)
    }

    private fun stopAndSubmit() {
        val rec = _state.value as? DictationState.Recording ?: return
        val stoppedAt = now()
        endAudio()
        val kept = capture?.take()
        capture = null
        // D58: the keyboard at the stop decides the language; a change since the start re-runs
        // the whole recording in the stop language (hands-free endpoint and cap stops too).
        val begun = startCfg
        val stopLangs = if (captureRelang) runCatching { host.stopLanguages() }.getOrNull() else null
        val relangCfg = if (stopLangs != null && begun != null && stopLangs != begun.languages && kept != null && kept.isNotEmpty()) {
            begun.copy(languages = stopLangs)
        } else null
        // D21 trace inputs, captured before the next dictation can reset them.
        val ts = traceStart?.let { t -> if (stopLangs == null) t else t.copy(stopLanguage = stopLangs.firstOrNull() ?: "auto") }
        val endpointWait = if (silence.heardSpeech) silence.trailingSilenceMs else null
        val live = _route.value
        val pcm = kept?.takeIf { captureDebug }
        val recording = kept?.takeIf { captureRetry && it.isNotEmpty() }?.let { Recording(it, sessionContext?.appCategory) }
        // D24 routing details for the trace, read on the commit thread right after the commit
        // (never on the caller's thread: logging must not block dictation).
        var diag: RouteDiag? = null
        fun done(outcome: String, r: CommitResult? = null, error: String? = null, text: String? = null) =
            trace(ts, outcome, r, stoppedAt, endpointWait, live, error = error, text = text,
                pcm = pcm?.takeIf { outcome != InsertOutcome.CANCELLED },
                diag = diag.takeIf { r != null })
        host.feedback(FeedbackEvent.StopSound)
        // D55: no engine yet while the model loads; the start below hands it over.
        val known = session
        session = null
        waiting?.let { it.endpointing = false; it.recording = recording }
        val appId = sessionAppId
        val ctx = sessionContext
        val cmd = commandTarget
        commandTarget = null
        val start = started ?: return
        val gen = generation
        _state.value = DictationState.Processing(rec.hold, now(), rec.command)
        processingCore = known
        processingJob = scope.launch {
            // The start may still be connecting (or waiting for the model); a failed start already showed its error.
            val first = start.await() ?: return@launch
            processingCore = first
            val c: FlowCore
            val pending: Deferred<CommitResult>
            if (relangCfg == null) {
                c = first
                // Not a child: a hung native commit must be abandonable.
                pending = CoroutineScope(commitDispatcher).async {
                    c.commit().also { diag = try { c.lastRouteDiag() } catch (_: Throwable) { null } }
                }
            } else {
                // D58: the start-language session (and its live preview) is dropped.
                _partial.value = null
                withContext(commitDispatcher) { runCatching { first.cancel() } }
                val got = try {
                    engineForStopLanguage(gen)
                } catch (e: CoreException) {
                    if (generation != gen) return@launch
                    val msg = RouterErrors.userMessage(e.kind, e.message ?: "start failed")
                    if (cmd == null) host.recordHistory(CommitResult(CommitStatus.ERROR, error = msg), false, appId, InsertOutcome.ERROR, recording)
                    _route.value = null
                    _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(msg))
                    done(InsertOutcome.START_FAILED, error = msg)
                    return@launch
                } ?: return@launch
                c = got.core
                processingCore = c
                val pcmAll = kept!!
                pending = CoroutineScope(commitDispatcher).async {
                    try {
                        if (got.phoneOnly) c.startSessionLocal(relangCfg) else c.startSessionInfo(relangCfg)
                        ctx?.let(c::setContext)
                        var i = 0
                        while (i < pcmAll.size) {
                            val n = minOf(RETRY_CHUNK, pcmAll.size - i)
                            c.pushAudio(pcmAll.copyOfRange(i, i + n), n)
                            i += n
                        }
                        c.commit().also { diag = try { c.lastRouteDiag() } catch (_: Throwable) { null } }
                    } catch (e: CoreException) {
                        CommitResult(CommitStatus.ERROR, error = e.message ?: "start failed")
                    }
                }
            }
            // D58: a re-run transcribes the whole recording again: the retry's budget.
            val result = withTimeoutOrNull(if (relangCfg == null) PROCESSING_TIMEOUT_MS else RETRY_TIMEOUT_MS) { pending.await() }
            _partial.value = null
            if (generation != gen) return@launch
            if (result == null) {
                c.cancel() // C4: the core aborts the running commit
                _route.value = null
                _state.value = DictationState.Error(DictationState.ErrorKind.FailedToTranscribe)
                done(InsertOutcome.TIMEOUT, error = "no result after $PROCESSING_TIMEOUT_MS ms")
                return@launch
            }
            _route.value = null
            if (isCancelled(result)) {
                _state.value = restingState()
                done(InsertOutcome.CANCELLED, result)
                return@launch
            }
            if (result.status == CommitStatus.ERROR) {
                if (cmd == null) host.recordHistory(result, false, appId, InsertOutcome.ERROR, recording)
                val routed = routerFailure(result)?.let {
                    withClientNote(it, CoreException.kindOf(result.error), runCatching { host.clientFallbackNote() }.getOrNull())
                }
                _state.value = DictationState.Error(
                    if (routed != null) DictationState.ErrorKind.EngineUnavailable(routed) else DictationState.ErrorKind.FailedToTranscribe,
                )
                done(InsertOutcome.ERROR, result)
                return@launch
            }
            // core-p1: paste `segments` (text runs + Break/Tab/Enter) instead of final_text + press_enter.
            val plan = result.pastePlan()
            if (result.status == CommitStatus.EMPTY || plan.none { it is PasteSegment.Text && it.text.isNotBlank() }) {
                if (cmd == null) host.recordHistory(result, false, appId, InsertOutcome.EMPTY, recording)
                host.feedback(FeedbackEvent.NoSpeech)
                val h = if (result.emptyReason == "only_filler") DictationHint.OnlyFiller else DictationHint.NoSpeechDetected
                _hint.value = h
                hintJob = scope.launch { delay(HINT_MS); if (_hint.value == h) _hint.value = null }
                _state.value = restingState()
                done(InsertOutcome.EMPTY, result, error = result.emptyReason?.let { "empty_reason=$it" })
                return@launch
            }
            if (cmd != null) {
                runCommand(c, cmd, result, appId, gen) { outcome, r, error, text -> done(outcome, r, error, text) }
                return@launch
            }
            // S12: only into the field focused at dictation start, and only while it is still allowed.
            when (host.insertTarget(explicit = false)) {
                InsertTarget.OK -> Unit
                InsertTarget.CHANGED, InsertTarget.PROTECTED -> {
                    val protected = host.insertTarget(explicit = true) == InsertTarget.PROTECTED
                    host.feedback(if (protected) FeedbackEvent.TargetProtected else FeedbackEvent.TargetChanged)
                    val text = plan.filterIsInstance<PasteSegment.Text>().joinToString("") { it.text }
                    val outcome = if (protected) InsertOutcome.TARGET_PROTECTED else InsertOutcome.TARGET_CHANGED
                    // D35: never keep a recording meant for a protected field.
                    host.recordHistory(result, false, appId, outcome, recording.takeIf { !protected })
                    notInserted(result, text, outcome)
                    _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
                    done(outcome, result)
                    return@launch
                }
            }
            val unpasted = paste(plan)
            val inserted = unpasted == null
            host.recordHistory(result, inserted, appId, if (inserted) InsertOutcome.INSERTED else InsertOutcome.PASTE_FAILED, recording)
            if (!inserted) notInserted(result, unpasted!!, InsertOutcome.PASTE_FAILED)
            else try {
                host.onInserted(
                    plan.filterIsInstance<PasteSegment.Text>().joinToString("") { it.text }, ctx,
                    plan.any { it is PasteSegment.Key && it.key == PasteKey.ENTER },
                )
            } catch (_: Throwable) { }
            _state.value = if (inserted) restingState()
            else DictationState.Error(DictationState.ErrorKind.FailedToPaste(unpasted!!))
            done(if (inserted) InsertOutcome.INSERTED else InsertOutcome.PASTE_FAILED, result)
        }
    }

    /**
     * D58: the engine for the stop language. [Host.prepareStopLanguage] reloads the phone model
     * when the stop language needs another one (nothing with both languages kept loaded); while
     * it loads, the bubble shows the model being loaded (as D55) and this waits up to
     * [LOAD_WAIT_MS]. Null = the dictation was dropped meanwhile; throws why there is no engine.
     */
    private suspend fun engineForStopLanguage(gen: Long): LoadedEngine? {
        runCatching { host.prepareStopLanguage() }
        try {
            return LoadedEngine(core(), phoneOnly = false)
        } catch (e: CoreException) {
            if (e.kind != CoreException.KIND_LOADING) throw e
            val w = loadWait() ?: throw e
            _route.value = SessionRoute("local", loading = true, loadingLabel = w.label)
            val t0 = now()
            val loaded = withTimeoutOrNull(LOAD_WAIT_MS) { w.await() }
            loadWaitMs = (loadWaitMs ?: 0L) + (now() - t0)
            if (generation != gen) return null
            _route.value = null
            return loaded ?: throw CoreException(LOAD_TIMEOUT, null)
        }
    }

    /**
     * Command Mode, after the instruction was transcribed: ask the AI (off the caller's thread,
     * bounded by [COMMAND_TIMEOUT_MS]), then replace the selection, or write at the cursor. Any
     * failure leaves the field as it was: error tone + the reason in the bubble.
     */
    private suspend fun runCommand(
        c: FlowCore, target: CommandTarget.Field, result: CommitResult, appId: String?, gen: Long,
        done: (outcome: String, r: CommitResult?, error: String?, text: String?) -> Unit,
    ) {
        val instruction = result.historyText.ifBlank { result.asrText }.trim()
        val selected = target.selected
        fun record(text: String?, outcome: String, error: String? = null) = try {
            host.recordCommand(CommandRecord(instruction, selected, text, appId, outcome, error))
        } catch (_: Throwable) { }
        // Not a child: a hung native call must be abandonable (cancel / timeout).
        val pending = CoroutineScope(commitDispatcher).async { c.command(instruction, selected) }
        val answer = withTimeoutOrNull(COMMAND_TIMEOUT_MS) { pending.await() }
            ?: CommandOutcome.Failed(CommandOutcome.TIMEOUT, "AI didn't answer within ${COMMAND_TIMEOUT_MS / 1000} s")
        if (generation != gen) return
        val text = when (answer) {
            is CommandOutcome.Failed -> {
                record(null, InsertOutcome.COMMAND_FAILED, answer.message)
                failCommand(answer.message)
                done(InsertOutcome.COMMAND_FAILED, result, answer.kind, null)
                return
            }
            is CommandOutcome.Text -> answer.text
        }
        // S12: only into the field the command started in, and only while it is still allowed.
        if (host.insertTarget(explicit = false) != InsertTarget.OK) {
            val protected = host.insertTarget(explicit = true) == InsertTarget.PROTECTED
            host.feedback(if (protected) FeedbackEvent.TargetProtected else FeedbackEvent.TargetChanged)
            val outcome = if (protected) InsertOutcome.TARGET_PROTECTED else InsertOutcome.TARGET_CHANGED
            record(text, outcome)
            try { host.onNotInserted(text, outcome) } catch (_: Throwable) { }
            _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
            done(outcome, result, null, text)
            return
        }
        val applied = try { host.applyCommand(target, text) } catch (e: Exception) { false }
        val outcome = if (applied) InsertOutcome.INSERTED else InsertOutcome.PASTE_FAILED
        record(text, outcome)
        if (applied) {
            _state.value = restingState()
        } else {
            // The selection moved or the app refused the write: nothing changed; keep the answer recoverable.
            host.feedback(FeedbackEvent.CommandFailed)
            try { host.onNotInserted(text, outcome) } catch (_: Throwable) { }
            _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
        }
        done(outcome, result, null, text)
    }

    /**
     * The notification copies what History saved ([CommitResult.historyText], line breaks
     * included) so both recovery paths give the same text; [runs] (the bare text runs) only
     * when the result has no history text.
     */
    private fun notInserted(result: CommitResult, runs: String, outcome: String) {
        val text = result.historyText.ifBlank { runs }
        if (text.isBlank()) return
        try { host.onNotInserted(text, outcome) } catch (_: Throwable) { }
    }

    /** Runs the paste plan. Returns null when every text run went in, else the text not inserted. */
    private suspend fun paste(plan: List<PasteSegment>): String? {
        for ((i, seg) in plan.withIndex()) when (seg) {
            is PasteSegment.Text -> if (seg.text.isNotEmpty() && !host.insert(seg.text)) {
                return plan.drop(i).filterIsInstance<PasteSegment.Text>().joinToString("") { it.text }
            }
            is PasteSegment.Key -> when (seg.key) {
                PasteKey.BREAK -> delay(BREAK_PAUSE_MS)
                // A missing key is not worth an error state: the text is in.
                PasteKey.TAB, PasteKey.ENTER -> host.sendKey(seg.key)
            }
        }
        return null
    }
}

/**
 * 2026-10-24 (docs/findings/2026-10-24-phone-local-only.md): the notification that keeps a
 * dictation which did not go in recoverable (tap = copy). Says why and how long, never the text
 * itself (it shows on the lock screen).
 */
object NotInsertedNotice {
    const val TITLE = "Dictation not inserted"

    fun words(text: String): Int = text.split(Regex("\\s+")).count { it.isNotEmpty() }

    fun text(outcome: String, transcript: String): String {
        val n = words(transcript)
        val size = if (n == 1) "1 word" else "$n words"
        val why = when (outcome) {
            dev.flowclone.debug.InsertOutcome.TARGET_CHANGED -> "You switched fields while dictating"
            dev.flowclone.debug.InsertOutcome.TARGET_PROTECTED -> "The field is protected"
            else -> "It could not be typed into the field"
        }
        return "$why ($size). Tap to copy it."
    }
}
