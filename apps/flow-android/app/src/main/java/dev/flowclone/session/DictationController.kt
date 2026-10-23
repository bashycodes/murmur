package dev.flowclone.session

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.PasteKey
import dev.flowclone.core.PasteSegment
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
    data class Recording(val hold: Boolean, val startedAt: Long) : DictationState
    data class Processing(val hold: Boolean, val since: Long) : DictationState
    data class Error(val kind: ErrorKind) : DictationState

    sealed interface ErrorKind {
        data class FailedToPaste(val text: String) : ErrorKind
        data object FailedToTranscribe : ErrorKind
        data object MicUnavailable : ErrorKind
        data class EngineUnavailable(val message: String) : ErrorKind
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
        fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?)
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
    }

    /** What the debug trace needs from the dictation's start. */
    private data class TraceStart(val wallMs: Long, val hold: Boolean, val appId: String?, val language: String? = null)

    companion object {
        const val TOGGLE_DEBOUNCE_MS = 500L
        const val PROCESSING_TIMEOUT_MS = 20_000L
        /** How often the preview tick polls the core (partial, endpoint, live route). */
        const val PARTIAL_POLL_MS = 250L
        const val NO_SPEECH_HINT_AFTER_MS = 3_000L
        const val HINT_MS = 2_500L
        /** `Break` segment: let the app react (e.g. open an @-mention popup) before the next run. */
        const val BREAK_PAUSE_MS = 150L

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
    /** Bumped by every start / cancel: late frames, start results and mic errors of an older dictation are ignored. */
    @Volatile private var generation = 0L
    /** Generation whose mic frames are accepted; 0 once its audio stopped (C15: no stale tail). */
    @Volatile private var liveFrames = 0L
    private val meter = LevelMeter()
    private var traceStart: TraceStart? = null

    /** D21: build + hand off the trace; never throws, never suspends. */
    private fun trace(
        start: TraceStart?, outcome: String, result: CommitResult? = null, stoppedAt: Long? = null,
        endpointWaitMs: Long? = null, liveRoute: SessionRoute? = null, error: String? = null,
        fallback: String? = null, cause: Throwable? = null, text: String? = null,
    ) {
        try {
            val s = start ?: return
            val t = result?.timings
            host.onDictationTrace(
                DictationTrace(
                    tsMs = s.wallMs, mode = if (s.hold) "hold" else "tap",
                    route = result?.route?.engine ?: liveRoute?.engine,
                    fallback = result?.route?.fallback ?: liveRoute?.fallback ?: fallback,
                    model = result?.route?.model ?: liveRoute?.model, language = s.language,
                    rttMs = result?.route?.rttMs,
                    audioMs = t?.audioMs, endpointWaitMs = endpointWaitMs, asrMs = t?.asrMs,
                    postMs = t?.let { it.llmMs + it.postprocessMs }, totalMs = t?.totalMs,
                    perceivedMs = stoppedAt?.let { now() - it },
                    waitMs = t?.waitMs, segments = t?.segments, preAsrMs = t?.preAsrMs, clientMs = result?.route?.clientMs,
                    target = s.appId?.ifEmpty { null }, insert = InsertOutcome(outcome),
                    raw = result?.asrText?.ifEmpty { null }, text = text ?: result?.historyText?.ifEmpty { null },
                    error = error ?: result?.error, stack = DebugLogger.stackOf(cause),
                ),
            )
        } catch (_: Throwable) {
            // D21: logging never affects dictation.
        }
    }

    private fun restingState(): DictationState = if (fieldFocused) DictationState.Ready else DictationState.Idle

    fun onFieldFocusChanged(focused: Boolean) {
        fieldFocused = focused
        when (_state.value) {
            DictationState.Idle, DictationState.Ready -> _state.value = restingState()
            else -> Unit // recording / processing / error survive focus changes
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

    fun dismissError() {
        if (_state.value is DictationState.Error) _state.value = restingState()
    }

    private fun start(hold: Boolean) {
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
        val cfg = host.sessionConfig().copy(
            maxSamples = cap.maxSamples,
            // D19: only hands-free sessions end themselves on silence; push-to-talk leaves it unset.
            endpointSilenceMs = if (handsFree) host.endpointSilenceMs() else null,
        )
        val gen = ++generation
        val ctx = host.captureContext()
        sessionAppId = ctx?.appId
        traceStart = TraceStart(wallClock(), hold, ctx?.appId, cfg.languages.firstOrNull() ?: "auto")
        session = c
        meter.reset()
        silence.reset()
        hintJob?.cancel(); _hint.value = null; _partial.value = null; _route.value = null
        val gate = StartGate()
        host.onRecordingActive(true)
        liveFrames = gen
        val ok = audio.start({ pcm, n ->
            if (liveFrames != gen) return@start // C15: a late tail of a stopped or older dictation
            gate.push(pcm, n)
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
        if (!ok) {
            generation++
            liveFrames = 0
            session = null
            host.onRecordingActive(false)
            trace(traceStart, InsertOutcome.START_FAILED, error = "microphone unavailable")
            _state.value = DictationState.Error(DictationState.ErrorKind.MicUnavailable)
            return
        }
        host.feedback(FeedbackEvent.StartSound)
        _state.value = DictationState.Recording(hold, now())
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
            if (_route.value == null) _route.value = info.route
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
                p.route?.let { if (it != _route.value) _route.value = it }
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
        val msg = RouterErrors.userMessage(e.kind, e.message ?: "start failed")
        trace(traceStart, InsertOutcome.START_FAILED, error = msg, fallback = e.kind, cause = e)
        if (_state.value is DictationState.Recording) endAudio()
        session = null
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
        fun done(outcome: String, r: CommitResult? = null, error: String? = null, text: String? = null) =
            trace(ts, outcome, r, stoppedAt, endpointWait, live, error = error, text = text)
        host.feedback(FeedbackEvent.StopSound)
        val c = session ?: return
        session = null
        val appId = sessionAppId
        val start = started
        val gen = generation
        _state.value = DictationState.Processing(rec.hold, now())
        processingCore = c
        processingJob = scope.launch {
            // The start may still be connecting; a failed start already showed its error.
            if (start != null && !start.await()) return@launch
            // Not a child: a hung native commit must be abandonable.
            val pending = CoroutineScope(commitDispatcher).async { c.commit() }
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
                host.recordHistory(result, false, appId)
                val routed = routerFailure(result)
                _state.value = DictationState.Error(
                    if (routed != null) DictationState.ErrorKind.EngineUnavailable(routed) else DictationState.ErrorKind.FailedToTranscribe,
                )
                done(InsertOutcome.ERROR, result)
                return@launch
            }
            // core-p1: paste `segments` (text runs + Break/Tab/Enter) instead of final_text + press_enter.
            val plan = result.pastePlan()
            if (result.status == CommitStatus.EMPTY || plan.none { it is PasteSegment.Text && it.text.isNotBlank() }) {
                host.recordHistory(result, false, appId)
                host.feedback(FeedbackEvent.NoSpeech)
                val h = if (result.emptyReason == "only_filler") DictationHint.OnlyFiller else DictationHint.NoSpeechDetected
                _hint.value = h
                hintJob = scope.launch { delay(HINT_MS); if (_hint.value == h) _hint.value = null }
                _state.value = restingState()
                done(InsertOutcome.EMPTY, result, error = result.emptyReason?.let { "empty_reason=$it" })
                return@launch
            }
            // S12: only into the field focused at dictation start, and only while it is still allowed.
            when (host.insertTarget(explicit = false)) {
                InsertTarget.OK -> Unit
                InsertTarget.CHANGED, InsertTarget.PROTECTED -> {
                    val protected = host.insertTarget(explicit = true) == InsertTarget.PROTECTED
                    host.feedback(if (protected) FeedbackEvent.TargetProtected else FeedbackEvent.TargetChanged)
                    host.recordHistory(result, false, appId)
                    val text = plan.filterIsInstance<PasteSegment.Text>().joinToString("") { it.text }
                    _state.value = DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
                    done(if (protected) InsertOutcome.TARGET_PROTECTED else InsertOutcome.TARGET_CHANGED, result)
                    return@launch
                }
            }
            val unpasted = paste(plan)
            val inserted = unpasted == null
            host.recordHistory(result, inserted, appId)
            _state.value = if (inserted) restingState()
            else DictationState.Error(DictationState.ErrorKind.FailedToPaste(unpasted!!))
            done(if (inserted) InsertOutcome.INSERTED else InsertOutcome.PASTE_FAILED, result)
        }
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
