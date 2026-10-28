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
    /** Completes true once the session started (false: start failed or the dictation was dropped). */
    private var started: Deferred<Boolean>? = null
    private var sessionAppId: String? = null
    private var sessionContext: FieldContext? = null
    /** Bumped by every start / cancel: late frames, start results and mic errors of an older dictation are ignored. */
    @Volatile private var generation = 0L
    /** Generation whose mic frames are accepted; 0 once its audio stopped (C15: no stale tail). */
    @Volatile private var liveFrames = 0L
    private val meter = LevelMeter()
    private var traceStart: TraceStart? = null
    /** "Keep last dictation audio (debug)": this dictation's PCM; null when the option is off. */
    private var capture: PcmCapture? = null

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
                    model = result?.route?.model ?: liveRoute?.model, language = s.language,
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
                    source = s.source,
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
        when (_state.value) {
            DictationState.Idle, DictationState.Ready -> {
                _state.value = restingState()
                if (focused) prewarm()
            }
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
            DictationState.Idle, DictationState.Ready, is DictationState.Error -> if (started?.isActive != true) start(hold)
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
                if (started?.isActive == true) return
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
        endAudio()
        _partial.value = null
        _hint.value = null
        _route.value = null
        // Started: cancel it now. Still starting: the start sees the new generation and cancels itself.
        if (started?.let { it.isCompleted && !it.isCancelled } == true) session?.cancel()
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

    fun dismissError() {
        if (_state.value is DictationState.Error) _state.value = restingState()
    }

    /** Command Mode target of the recording / processing command; null for a dictation. */
    private var commandTarget: CommandTarget.Field? = null

    private fun start(hold: Boolean, command: CommandTarget.Field? = null) {
        if (!host.hasMicPermission()) {
            host.feedback(FeedbackEvent.NeedMicPermission)
            return
        }
        val c = try {
            core()
        } catch (e: CoreException) {
            val msg = RouterErrors.userMessage(e.kind, e.message ?: "engine unavailable")
            trace(TraceStart(wallClock(), hold, null), InsertOutcome.START_FAILED, error = msg, fallback = e.kind, cause = e)
            _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(msg))
            return
        }
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
        session = c
        meter.reset()
        silence.reset()
        val pcmCapture = if (runCatching { host.keepDictationAudio() }.getOrDefault(false)) PcmCapture(cap.maxSamples.toInt()) else null
        capture = pcmCapture
        hintJob?.cancel(); _hint.value = null; _partial.value = null; _route.value = null
        val gate = StartGate()
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
        // C2: the (possibly network-bound) start runs off the caller's thread; frames wait in the gate.
        started = scope.async(start = CoroutineStart.UNDISPATCHED) {
            val info = try {
                withContext(startDispatcher) { c.startSessionInfo(cfg) }
            } catch (e: CoreException) {
                if (generation == gen) failStart(e)
                return@async false
            }
            if (generation != gen) { c.cancel(); return@async false } // cancelled while connecting
            ctx?.let(c::setContext)
            gate.open { pcm, n -> c.pushAudio(pcm, n) }
            // D24: "Loading phone model…" while the router loads the on-device engine on demand.
            if (_route.value == null) _route.value = info.route?.copy(loading = info.loading)
            true
        }
        val preview = cfg.partialIntervalMs != null
        pollJob = scope.launch {
            if (started?.await() != true) return@launch
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

    /** The session could not start (router: no engine, server-only and unreachable, …). */
    private fun failStart(e: CoreException) {
        val msg = withClientNote(RouterErrors.userMessage(e.kind, e.message ?: "start failed"), e.kind, runCatching { host.clientFallbackNote() }.getOrNull())
        trace(traceStart, InsertOutcome.START_FAILED, error = msg, fallback = e.kind, cause = e)
        if (_state.value is DictationState.Recording) endAudio()
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
        // D21 trace inputs, captured before the next dictation can reset them.
        val ts = traceStart
        val endpointWait = if (silence.heardSpeech) silence.trailingSilenceMs else null
        val live = _route.value
        val pcm = capture?.take()
        capture = null
        // D24 routing details for the trace, read on the commit thread right after the commit
        // (never on the caller's thread: logging must not block dictation).
        var diag: RouteDiag? = null
        fun done(outcome: String, r: CommitResult? = null, error: String? = null, text: String? = null) =
            trace(ts, outcome, r, stoppedAt, endpointWait, live, error = error, text = text,
                pcm = pcm?.takeIf { outcome != InsertOutcome.CANCELLED },
                diag = diag.takeIf { r != null })
        host.feedback(FeedbackEvent.StopSound)
        val c = session ?: return
        session = null
        val appId = sessionAppId
        val ctx = sessionContext
        val cmd = commandTarget
        commandTarget = null
        val start = started
        val gen = generation
        _state.value = DictationState.Processing(rec.hold, now(), rec.command)
        processingCore = c
        processingJob = scope.launch {
            // The start may still be connecting; a failed start already showed its error.
            if (start != null && !start.await()) return@launch
            // Not a child: a hung native commit must be abandonable.
            val pending = CoroutineScope(commitDispatcher).async {
                c.commit().also { diag = try { c.lastRouteDiag() } catch (_: Throwable) { null } }
            }
            val result = withTimeoutOrNull(PROCESSING_TIMEOUT_MS) { pending.await() }
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
                if (cmd == null) host.recordHistory(result, false, appId, InsertOutcome.ERROR)
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
                if (cmd == null) host.recordHistory(result, false, appId, InsertOutcome.EMPTY)
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
                    host.recordHistory(result, false, appId, outcome)
                    notInserted(result, text, outcome)
                    _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
                    done(outcome, result)
                    return@launch
                }
            }
            val unpasted = paste(plan)
            val inserted = unpasted == null
            host.recordHistory(result, inserted, appId, if (inserted) InsertOutcome.INSERTED else InsertOutcome.PASTE_FAILED)
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
