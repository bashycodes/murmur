package dev.flowclone.session

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.SessionConfig
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** Microphone capture. [start] delivers frames on its own thread until [stop] (which drains the tail). */
interface AudioSource {
    fun start(onFrame: (ShortArray, Int) -> Unit): Boolean
    fun stop()
}

enum class FeedbackEvent { StartSound, StopSound, CancelHaptic, CapWarning, CapReached, NeedMicPermission, PasteRetryFailed, NoSpeech }

/** Transient hint shown next to the bubble (live preview line). */
enum class DictationHint {
    /** Recording, but nothing above the speech threshold for 3 s: "Can't hear you". */
    NoSpeechYet,
    /** The core returned EMPTY (silence gate / blank ASR): nothing was inserted. */
    NoSpeechDetected,
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
) {
    constructor(
        scope: CoroutineScope, audio: AudioSource, host: Host, core: () -> FlowCore,
        commitDispatcher: CoroutineDispatcher, now: () -> Long,
    ) : this(scope, audio, host, core, commitDispatcher, SessionCap(), now)

    interface Host {
        fun feedback(e: FeedbackEvent)
        /** insertTextAndVerify (incl. clipboard fallback on failure). */
        suspend fun insert(text: String): Boolean
        fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?)
        fun hasMicPermission(): Boolean
        /** captureFieldSnapshot at dictation start; null when extraction isn't allowed. */
        fun captureContext(): FieldContext?
        fun sessionConfig(): SessionConfig
        /** true at record start (promote FGS to microphone, arm keep-alive), false when audio stops. */
        fun onRecordingActive(active: Boolean)
    }

    companion object {
        const val TOGGLE_DEBOUNCE_MS = 500L
        const val PROCESSING_TIMEOUT_MS = 20_000L
        /** How often the live preview reads the core's latest partial (a cached string). */
        const val PARTIAL_POLL_MS = 250L
        const val NO_SPEECH_HINT_AFTER_MS = 3_000L
        const val HINT_MS = 2_500L
    }

    private val _state = MutableStateFlow<DictationState>(DictationState.Idle)
    val state: StateFlow<DictationState> = _state
    private val _level = MutableStateFlow(0f)
    val level: StateFlow<Float> = _level
    private val _partial = MutableStateFlow<String?>(null)
    /** Latest live-preview text while recording / processing (only with `partialIntervalMs`). */
    val partial: StateFlow<String?> = _partial
    private val _hint = MutableStateFlow<DictationHint?>(null)
    val hint: StateFlow<DictationHint?> = _hint
    private var partialJob: Job? = null
    private var hintJob: Job? = null
    private val silence = SilenceTracker()

    private var fieldFocused = false
    private var lastToggleAt = Long.MIN_VALUE / 2
    private var capJob: Job? = null
    private var processingJob: Job? = null
    private var processingCore: FlowCore? = null
    private var session: FlowCore? = null
    private var sessionAppId: String? = null
    private val meter = LevelMeter()

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
            DictationState.Idle, DictationState.Ready, is DictationState.Error -> start(hold)
            is DictationState.Recording -> stopAndSubmit()
            is DictationState.Processing -> Unit
        }
    }

    /** Finger lifted after a hold: always stop + submit (not debounced). Also the ✓ button. */
    fun release() {
        if (_state.value is DictationState.Recording) stopAndSubmit()
    }

    /** X button: discards a recording, or abandons processing (late result dropped). */
    fun cancel() {
        if (_state.value is DictationState.Processing) {
            processingJob?.cancel(); processingJob = null
            processingCore?.cancel(); processingCore = null
            _partial.value = null
            host.feedback(FeedbackEvent.CancelHaptic)
            _state.value = restingState()
            return
        }
        if (_state.value !is DictationState.Recording) return
        endAudio()
        _partial.value = null
        _hint.value = null
        session?.cancel()
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
            _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(e.message ?: "engine unavailable"))
            return
        }
        val cfg = host.sessionConfig().copy(maxSamples = cap.maxSamples)
        try {
            c.startSession(cfg)
        } catch (e: CoreException) {
            _state.value = DictationState.Error(DictationState.ErrorKind.EngineUnavailable(e.message ?: "start failed"))
            return
        }
        val ctx = host.captureContext()
        sessionAppId = ctx?.appId
        ctx?.let(c::setContext)
        session = c
        meter.reset()
        silence.reset()
        hintJob?.cancel(); _hint.value = null; _partial.value = null
        host.onRecordingActive(true)
        val ok = audio.start { pcm, n ->
            c.pushAudio(pcm, n)
            _level.value = meter.update(pcm, n)
            silence.update(pcm, n)
            val h = _hint.value
            if (silence.heardSpeech) { if (h == DictationHint.NoSpeechYet) _hint.value = null }
            else if (h == null && silence.leadingSilenceMs >= NO_SPEECH_HINT_AFTER_MS) _hint.value = DictationHint.NoSpeechYet
        }
        if (!ok) {
            c.cancel(); session = null
            host.onRecordingActive(false)
            _state.value = DictationState.Error(DictationState.ErrorKind.MicUnavailable)
            return
        }
        host.feedback(FeedbackEvent.StartSound)
        _state.value = DictationState.Recording(hold, now())
        if (cfg.partialIntervalMs != null) partialJob = scope.launch {
            while (true) {
                delay(PARTIAL_POLL_MS)
                c.partial()?.takeIf { it.isNotBlank() }?.let { _partial.value = it.trim() }
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

    private fun endAudio() {
        capJob?.cancel(); capJob = null
        partialJob?.cancel(); partialJob = null
        audio.stop()
        if (_hint.value == DictationHint.NoSpeechYet) _hint.value = null
        _level.value = 0f
        host.onRecordingActive(false)
    }

    private fun stopAndSubmit() {
        val rec = _state.value as? DictationState.Recording ?: return
        endAudio()
        host.feedback(FeedbackEvent.StopSound)
        val c = session ?: return
        session = null
        val appId = sessionAppId
        _state.value = DictationState.Processing(rec.hold, now())
        processingCore = c
        processingJob = scope.launch {
            // Not a child: a hung native commit must be abandonable.
            val pending = CoroutineScope(commitDispatcher).async { c.commit() }
            val result = withTimeoutOrNull(PROCESSING_TIMEOUT_MS) { pending.await() }
            _partial.value = null
            if (result == null) {
                c.cancel()
                _state.value = DictationState.Error(DictationState.ErrorKind.FailedToTranscribe)
                return@launch
            }
            if (result.status == CommitStatus.ERROR) {
                host.recordHistory(result, false, appId)
                _state.value = DictationState.Error(DictationState.ErrorKind.FailedToTranscribe)
                return@launch
            }
            val text = result.finalText
            if (result.status == CommitStatus.EMPTY || text.isBlank()) {
                host.recordHistory(result, false, appId)
                host.feedback(FeedbackEvent.NoSpeech)
                _hint.value = DictationHint.NoSpeechDetected
                hintJob = scope.launch { delay(HINT_MS); if (_hint.value == DictationHint.NoSpeechDetected) _hint.value = null }
                _state.value = restingState()
                return@launch
            }
            val inserted = host.insert(text)
            host.recordHistory(result, inserted, appId)
            _state.value = if (inserted) restingState()
            else DictationState.Error(DictationState.ErrorKind.FailedToPaste(text))
        }
    }
}
