package dev.flowclone.session

import dev.flowclone.core.CommitResult
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.SessionConfig
import dev.flowclone.session.DictationState.ErrorKind
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class DictationControllerTest {
    private class FakeAudio : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        var startOk = true
        var starts = 0; var stops = 0
        var onError: (() -> Unit)? = null
        var label = "mic"
        override val source: String get() = label
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean {
            starts++
            if (!startOk) return false
            sink = onFrame; return true
        }
        override fun start(onFrame: (ShortArray, Int) -> Unit, onError: () -> Unit): Boolean =
            start(onFrame).also { if (it) this.onError = onError }
        override fun stop() { stops++; sink = null }
        fun speak(ms: Int = 1000) { repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) } }
    }

    private class Recorder : DictationController.Host {
        val events = mutableListOf<FeedbackEvent>()
        val inserted = mutableListOf<String>()
        val history = mutableListOf<Pair<CommitResult, Boolean>>()
        var insertOk = true
        var insertGate: CompletableDeferred<Unit>? = null
        var micGranted = true
        var context: FieldContext? = FieldContext(appId = "com.example.notes", before = "Hi ")
        var recordingActive = mutableListOf<Boolean>()
        override fun feedback(e: FeedbackEvent) { events += e }
        override suspend fun insert(text: String): Boolean { insertGate?.await(); inserted += text; return insertOk }
        val keys = mutableListOf<dev.flowclone.core.PasteKey>()
        override suspend fun sendKey(key: dev.flowclone.core.PasteKey): Boolean { keys += key; inserted += "<${key.wire}>"; return true }
        val outcomes = mutableListOf<String?>()
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) { history += result to inserted; outcomes += outcome }
        var keepRetry = false
        override fun keepRetryAudio() = keepRetry
        /** D35: the recording handed over with each History row (null = not kept). */
        val recordings = mutableListOf<Recording?>()
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?, recording: Recording?) {
            recordings += recording
            recordHistory(result, inserted, appId, outcome)
        }
        override fun hasMicPermission() = micGranted
        override fun captureContext() = context
        var config = SessionConfig(languages = listOf("en"))
        override fun sessionConfig() = config
        override fun onRecordingActive(active: Boolean) { recordingActive += active }
        var endpointMs: Int? = 2000
        override fun endpointSilenceMs() = endpointMs
        var target = InsertTarget.OK
        var explicitTarget = InsertTarget.OK
        override fun insertTarget(explicit: Boolean) = if (explicit) explicitTarget else target
        val notInserted = mutableListOf<Pair<String, String>>()
        override fun onNotInserted(text: String, outcome: String) { notInserted += text to outcome }
        val traces = mutableListOf<dev.flowclone.debug.DictationTrace>()
        override fun onDictationTrace(trace: dev.flowclone.debug.DictationTrace) { traces += trace }
        val insertedFor = mutableListOf<Triple<String, FieldContext?, Boolean>>()
        override fun onInserted(pasted: String, context: FieldContext?, sentKey: Boolean) { insertedFor += Triple(pasted, context, sentKey) }
        val loadingNotices = mutableListOf<String>()
        override fun engineLoading(message: String) { loadingNotices += message }
    }

    /** advanceUntilIdle ignores backgroundScope work, where the controller lives. */
    private fun TestScope.settle() { advanceTimeBy(1_000); runCurrent() }

    private val audio = FakeAudio()
    private val host = Recorder()
    private val core = FakeFlowCore(transcript = "hello world")

    private fun TestScope.controller(
        c: FlowCore? = core,
        err: String = "no engine",
        dispatcher: kotlinx.coroutines.CoroutineDispatcher? = null,
    ): DictationController {
        val d = dispatcher ?: StandardTestDispatcher(testScheduler)
        return DictationController(
            scope = backgroundScope, audio = audio, host = host,
            core = { c ?: throw dev.flowclone.core.CoreException(err) },
            commitDispatcher = d, now = { currentTime },
        ).also { it.onFieldFocusChanged(true) }
    }

    @Test fun startsIdleAndBecomesReadyOnFocus() = runTest {
        val c = DictationController(backgroundScope, audio, host, { core }, StandardTestDispatcher(testScheduler)) { currentTime }
        assertEquals(DictationState.Idle, c.state.value)
        c.onFieldFocusChanged(true)
        assertEquals(DictationState.Ready, c.state.value)
        c.onFieldFocusChanged(false)
        assertEquals(DictationState.Idle, c.state.value)
    }

    @Test fun tapStartsRecordingThenTapSubmitsAndInserts() = runTest {
        val c = controller()
        c.toggle(hold = false)
        assertTrue(c.state.value is DictationState.Recording)
        assertEquals(listOf(true), host.recordingActive)
        assertTrue(FeedbackEvent.StartSound in host.events)
        assertEquals("en", core.lastConfig!!.languages.single())
        assertEquals("context captured at start", "Hi ", core.lastContext!!.before)
        audio.speak()
        advanceTimeBy(600)
        c.toggle(hold = false)
        settle()
        assertEquals(listOf("hello world"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
        assertTrue(FeedbackEvent.StopSound in host.events)
        assertEquals(1, audio.stops)
        assertEquals(listOf(true, false), host.recordingActive)
        assertTrue(host.history.single().second)
    }

    /** D37 / D58: the keyboard language is read once, at start; a keyboard switched mid-recording is not re-read at stop. */
    @Test fun languageIsPickedAtStartNotAtStop() = runTest {
        val c = controller()
        c.toggle(hold = false)
        host.config = SessionConfig(languages = listOf("he")) // keyboard switched while recording
        audio.speak()
        advanceTimeBy(600)
        c.toggle(hold = false)
        settle()
        assertEquals(listOf("hello world"), host.inserted)
        assertEquals("en", core.lastConfig!!.languages.single())
    }

    @Test fun aSuccessfulInsertStartsTheEditWatchWithTheStartContext() = runTest {
        val c = controller()
        c.toggle(hold = false)
        audio.speak()
        advanceTimeBy(600)
        c.toggle(hold = false)
        settle()
        assertEquals(listOf(Triple("hello world", host.context, false)), host.insertedFor)
    }

    @Test fun aFailedInsertStartsNoEditWatch() = runTest {
        host.insertOk = false
        val c = controller()
        c.toggle(hold = false)
        audio.speak()
        advanceTimeBy(600)
        c.toggle(hold = false)
        settle()
        assertTrue(host.insertedFor.isEmpty())
    }

    @Test fun holdStartsAndReleaseSubmitsWithoutDebounce() = runTest {
        val c = controller()
        c.toggle(hold = true)
        audio.speak(200)
        advanceTimeBy(100) // release well inside 500 ms debounce
        c.release()
        settle()
        assertEquals(listOf("hello world"), host.inserted)
    }

    @Test fun tapsWithin500msAreDebounced() = runTest {
        val c = controller()
        c.toggle(hold = false)
        advanceTimeBy(200)
        c.toggle(hold = false)
        assertTrue("second tap ignored", c.state.value is DictationState.Recording)
    }

    @Test fun silenceCommitsEmptyAndInsertsNothing() = runTest {
        val c = controller()
        c.toggle(false); advanceTimeBy(600); c.toggle(false); settle()
        assertTrue(host.inserted.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
        assertFalse(host.history.single().second)
    }

    @Test fun blankTranscriptIsTreatedAsEmpty() = runTest {
        core.transcript = "   "
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertTrue(host.inserted.isEmpty())
    }

    @Test fun insertFailureShowsFailedToPasteAndPasteRetries() = runTest {
        host.insertOk = false
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("hello world")), c.state.value)
        host.insertOk = true
        c.pasteRetry()
        settle()
        assertEquals(listOf("hello world", "hello world"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun pasteRetryFailureToastsAndStaysInError() = runTest {
        host.insertOk = false
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        c.pasteRetry(); settle()
        assertTrue(FeedbackEvent.PasteRetryFailed in host.events)
        assertTrue(c.state.value is DictationState.Error)
    }

    @Test fun cancelDiscardsAudioAndReturnsReady() = runTest {
        val c = controller()
        c.toggle(false); audio.speak()
        c.cancel()
        settle()
        assertTrue(host.inserted.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
        assertTrue(FeedbackEvent.CancelHaptic in host.events)
        assertEquals(listOf(true, false), host.recordingActive)
    }

    @Test fun toggleDuringProcessingIsIgnored() = runTest {
        host.insertGate = CompletableDeferred()
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false)
        runCurrent()
        assertTrue(c.state.value is DictationState.Processing)
        advanceTimeBy(600)
        c.toggle(false)
        assertTrue(c.state.value is DictationState.Processing)
        assertEquals(1, audio.starts)
        host.insertGate!!.complete(Unit)
        settle()
    }

    @Test fun capWarnsAt239sAndAutoSubmitsAt299s() = runTest {
        val c = controller()
        c.toggle(false); audio.speak()
        advanceTimeBy(239_001)
        assertTrue(FeedbackEvent.CapWarning in host.events)
        assertTrue(c.state.value is DictationState.Recording)
        advanceTimeBy(60_000)
        settle()
        assertTrue(FeedbackEvent.CapReached in host.events)
        assertEquals(listOf("hello world"), host.inserted)
    }

    @Test fun maxSamplesPassedToCoreMatchesCap() = runTest {
        val c = controller()
        c.toggle(false)
        assertEquals(300L * 16_000, core.lastConfig!!.maxSamples)
        c.cancel()
    }

    @Test fun noMicPermissionAsksForItInsteadOfRecording() = runTest {
        host.micGranted = false
        val c = controller()
        c.toggle(false)
        assertEquals(DictationState.Ready, c.state.value)
        assertTrue(FeedbackEvent.NeedMicPermission in host.events)
        assertEquals(0, audio.starts)
    }

    @Test fun engineUnavailableIsAnErrorStateNotACrash() = runTest {
        val c = controller(c = null, err = "libflow_ffi.so missing")
        c.toggle(false)
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable("libflow_ffi.so missing")), c.state.value)
        assertEquals(0, audio.starts)
    }

    /** A core that is still loading until [loading] goes false (EngineHolder.current() while Loading). */
    private fun TestScope.loadingController(state: () -> Boolean): DictationController =
        DictationController(
            backgroundScope, audio, host,
            { if (state()) throw dev.flowclone.core.CoreException("Loading the synthetic model…", dev.flowclone.core.CoreException.KIND_LOADING) else core },
            StandardTestDispatcher(testScheduler),
        ) { currentTime }.also { it.onFieldFocusChanged(true) }

    @Test fun aStillLoadingModelShowsLoadingNotAnEngineError() = runTest {
        val c = loadingController { true }
        c.toggle(false)
        assertEquals(DictationState.Error(ErrorKind.EngineLoading("Loading the synthetic model…")), c.state.value)
        assertEquals(listOf("Loading the synthetic model…"), host.loadingNotices)
        assertEquals(0, audio.starts)
    }

    @Test fun theLoadingStateClearsItselfOnceTheModelIsReady() = runTest {
        var loading = true
        val c = loadingController { loading }
        c.toggle(false)
        loading = false
        c.engineSettled(failure = null)
        assertEquals("back to normal without a tap", DictationState.Ready, c.state.value)
    }

    @Test fun aLoadFailureTurnsTheLoadingStateIntoTheRealError() = runTest {
        val c = loadingController { true }
        c.toggle(false)
        c.engineSettled(failure = "Speech model file is missing")
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable("Speech model file is missing")), c.state.value)
    }

    @Test fun engineSettledLeavesOtherStatesAlone() = runTest {
        val c = controller()
        c.engineSettled(failure = "boom")
        assertEquals(DictationState.Ready, c.state.value)
        c.toggle(false)
        c.engineSettled(failure = null)
        assertTrue(c.state.value is DictationState.Recording)
    }

    @Test fun eachTapWhileLoadingChecksAgainAndStartsOnceReady() = runTest {
        var loading = true
        val c = loadingController { loading }
        c.toggle(false)
        advanceTimeBy(600)
        c.toggle(false)
        assertEquals("the second tap says it is still loading too", 2, host.loadingNotices.size)
        assertTrue(c.state.value is DictationState.Error)
        loading = false
        advanceTimeBy(600)
        c.toggle(false)
        assertTrue(c.state.value is DictationState.Recording)
    }

    @Test fun losingTheFieldWhileLoadingHidesTheLoadingBubble() = runTest {
        val c = loadingController { true }
        c.toggle(false)
        c.onFieldFocusChanged(false)
        assertEquals(DictationState.Idle, c.state.value)
    }

    @Test fun micStartFailureCancelsSession() = runTest {
        audio.startOk = false
        val c = controller()
        c.toggle(false)
        assertEquals(DictationState.Error(ErrorKind.MicUnavailable), c.state.value)
    }

    @Test fun processingTimeoutAfter20sIsFailedToTranscribe() = runTest {
        val hung = HangingCore(core)
        // A hung native commit blocks a real thread; the controller must abandon it, not wait.
        val c = controller(c = hung, dispatcher = kotlinx.coroutines.Dispatchers.IO)
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false)
        advanceTimeBy(20_001)
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.FailedToTranscribe), c.state.value)
        assertTrue(hung.cancelled)
        hung.release.complete(Unit)
    }

    @Test fun cancelDuringProcessingDropsTheLateResult() = runTest {
        val hung = HangingCore(core)
        val c = controller(c = hung, dispatcher = kotlinx.coroutines.Dispatchers.IO)
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false)
        runCurrent()
        assertTrue(c.state.value is DictationState.Processing)
        c.cancel()
        assertTrue(hung.cancelled)
        assertEquals(DictationState.Ready, c.state.value)
        Thread.sleep(50) // let the IO-thread commit return
        settle()
        assertTrue(host.inserted.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun focusLossWhileRecordingKeepsRecordingAndEndsIdle() = runTest {
        val c = controller()
        c.toggle(false); audio.speak()
        c.onFieldFocusChanged(false)
        assertTrue(c.state.value is DictationState.Recording)
        advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(DictationState.Idle, c.state.value)
    }

    @Test fun levelFollowsAudio() = runTest {
        val c = controller()
        c.toggle(false)
        audio.sink!!.invoke(ShortArray(1600) { 100 }, 1600)
        audio.sink!!.invoke(ShortArray(1600) { 20000 }, 1600)
        assertEquals(1f, c.level.value, 0.01f)
        c.cancel()
        assertEquals(0f, c.level.value, 0f)
    }

    // ---- live preview + silence hints (07 §7 HeadSilenceTracker threshold; 10 §4 partials) ----

    private fun silence(ms: Int) { repeat(ms / 100) { audio.sink?.invoke(ShortArray(1600), 1600) } }

    @Test fun partialsArePolledWhileRecordingAndClearedOnSubmit() = runTest {
        host.config = host.config.copy(partialIntervalMs = 1000)
        val c = controller()
        c.toggle(false); audio.speak()
        core.partialText = "hello wor"
        advanceTimeBy(DictationController.PARTIAL_POLL_MS + 1); runCurrent()
        assertEquals("hello wor", c.partial.value)
        advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(null, c.partial.value)
    }

    @Test fun partialsNotPolledWhenPreviewOff() = runTest {
        val c = controller()
        c.toggle(false); audio.speak()
        core.partialText = "hello"
        advanceTimeBy(2_000); runCurrent()
        assertEquals(null, c.partial.value)
        c.cancel()
    }

    @Test fun cancelClearsPartial() = runTest {
        host.config = host.config.copy(partialIntervalMs = 1000)
        val c = controller()
        c.toggle(false); audio.speak(); core.partialText = "abc"
        advanceTimeBy(1_000); runCurrent()
        c.cancel()
        assertEquals(null, c.partial.value)
    }

    @Test fun threeSecondsOfLeadingSilenceShowsCantHearYou() = runTest {
        val c = controller()
        c.toggle(false)
        silence(2_900)
        assertEquals(null, c.hint.value)
        silence(200)
        assertEquals(DictationHint.NoSpeechYet, c.hint.value)
        audio.speak(200)
        assertEquals(null, c.hint.value)
        c.cancel()
    }

    @Test fun emptyResultShowsNoSpeechDetectedThenClears() = runTest {
        val c = controller()
        c.toggle(false); silence(1_000); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(DictationHint.NoSpeechDetected, c.hint.value)
        assertTrue(FeedbackEvent.NoSpeech in host.events)
        advanceTimeBy(DictationController.HINT_MS + 1); runCurrent()
        assertEquals(null, c.hint.value)
    }

    @Test fun newRecordingClearsOldHint() = runTest {
        val c = controller()
        c.toggle(false); silence(500); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(DictationHint.NoSpeechDetected, c.hint.value)
        advanceTimeBy(600)
        c.toggle(false)
        assertEquals(null, c.hint.value)
        c.cancel()
    }

    private fun TestScope.dictate(result: CommitResult): DictationController {
        val c = controller(ScriptedCore(core, result))
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        return c
    }

    @Test fun pastesSegmentsInOrderWithKeysAndBreakPause() = runTest {
        val r = CommitResult(
            dev.flowclone.core.CommitStatus.FORMATTED, finalText = "@foo{{TAB}} bar", pressEnter = true, plainText = "@foo bar",
            segments = listOf(
                dev.flowclone.core.PasteSegment.Text("@foo"), dev.flowclone.core.PasteSegment.Key(dev.flowclone.core.PasteKey.TAB),
                dev.flowclone.core.PasteSegment.Text(" bar"), dev.flowclone.core.PasteSegment.Key(dev.flowclone.core.PasteKey.BREAK),
                dev.flowclone.core.PasteSegment.Key(dev.flowclone.core.PasteKey.ENTER),
            ),
        )
        val c = dictate(r)
        assertEquals("press_enter is not acted on twice", listOf("@foo", "<Tab>", " bar", "<Enter>"), host.inserted)
        assertEquals("the edit watch knows Enter was sent", listOf(Triple("@foo bar", host.context, true)), host.insertedFor)
        assertEquals(DictationState.Ready, c.state.value)
        assertTrue(host.history.single().second)
    }

    @Test fun failedRunReportsTheRemainingText() = runTest {
        host.insertOk = false
        val r = CommitResult(
            dev.flowclone.core.CommitStatus.RAW_TRANSCRIPT, finalText = "a{{BREAK}}b",
            segments = listOf(dev.flowclone.core.PasteSegment.Text("a"), dev.flowclone.core.PasteSegment.Key(dev.flowclone.core.PasteKey.BREAK), dev.flowclone.core.PasteSegment.Text("b")),
        )
        val c = dictate(r)
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("ab")), c.state.value)
    }

    @Test fun onlyFillerShowsItsOwnHint() = runTest {
        val c = dictate(CommitResult(dev.flowclone.core.CommitStatus.EMPTY, emptyReason = "only_filler"))
        assertEquals(DictationHint.OnlyFiller, c.hint.value)
        assertTrue(host.inserted.isEmpty())
    }

    @Test fun serverOnlyFailureSaysServerUnreachable() = runTest {
        val c = dictate(CommitResult(dev.flowclone.core.CommitStatus.ERROR, error = "server_unavailable: connect_failed"))
        val s = c.state.value as DictationState.Error
        assertEquals(ErrorKind.EngineUnavailable("Server unreachable (connect_failed)"), s.kind)
    }

    @Test fun bothTogglesOffIsAClearErrorAtStart() = runTest {
        val refusing = object : FlowCore by core {
            override fun startSession(cfg: SessionConfig) = throw dev.flowclone.core.CoreException("no_engine_enabled: both off", "no_engine_enabled")
            override fun startSessionInfo(cfg: SessionConfig) = startSession(cfg).let { dev.flowclone.core.StartInfo() }
        }
        val c = controller(refusing)
        c.toggle(false)
        assertEquals(
            DictationState.Error(ErrorKind.EngineUnavailable("Dictation disabled — enable server or client in Developer settings")),
            c.state.value,
        )
        // C2: the mic opens before the (off-Main) start answers; a refused start closes it again.
        assertEquals(1, audio.starts)
        assertEquals(1, audio.stops)
        assertEquals(listOf(true, false), host.recordingActive)
    }

    // ---- 2026-10-22 shell changelog: hands-free endpointing, live route, off-Main start, cancel ----

    @Test fun handsFreeSendsEndpointSilenceAndPushToTalkDoesNot() = runTest {
        val c = controller()
        c.toggle(hold = false)
        assertEquals(2000, core.lastConfig!!.endpointSilenceMs)
        c.cancel(); advanceTimeBy(600)
        c.toggle(hold = true)
        assertEquals(null, core.lastConfig!!.endpointSilenceMs)
    }

    @Test fun endpointReachedRunsTheNormalFinishPath() = runTest {
        core.endpointAfterSamples = 16_000
        val c = controller()
        c.toggle(hold = false)
        audio.speak(500)
        advanceTimeBy(DictationController.PARTIAL_POLL_MS * 2); runCurrent()
        assertTrue("not yet", c.state.value is DictationState.Recording)
        audio.speak(600)
        advanceTimeBy(DictationController.PARTIAL_POLL_MS + 1); runCurrent()
        assertTrue(FeedbackEvent.StopSound in host.events)
        settle()
        assertEquals(listOf("hello world"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun endpointIsIgnoredForPushToTalk() = runTest {
        core.endpointAfterSamples = 1
        val c = controller()
        c.toggle(hold = true)
        audio.speak(1000)
        advanceTimeBy(2_000); runCurrent()
        assertTrue(c.state.value is DictationState.Recording)
    }

    @Test fun liveRouteShownAtStartAndUpdatedOnFailover() = runTest {
        core.route = dev.flowclone.core.SessionRoute("remote", server = "100.64.0.10:7313")
        val c = controller()
        c.toggle(false)
        assertEquals("start route before the first poll", "remote", c.route.value!!.engine)
        audio.speak(300)
        core.route = dev.flowclone.core.SessionRoute("local", fallback = "disconnected")
        advanceTimeBy(DictationController.PARTIAL_POLL_MS + 1); runCurrent()
        assertEquals(dev.flowclone.core.SessionRoute("local", fallback = "disconnected"), c.route.value)
        core.route = null // dead session mid-recording: keep the last label
        advanceTimeBy(DictationController.PARTIAL_POLL_MS + 1); runCurrent()
        assertEquals("local", c.route.value!!.engine)
        advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(null, c.route.value)
    }

    @Test fun cancelledCommitInsertsNothingAndLeavesNoHistory() = runTest {
        val cancelled = object : FlowCore by core {
            override fun startSessionInfo(cfg: SessionConfig) = core.startSessionInfo(cfg)
            override fun commit() = CommitResult(dev.flowclone.core.CommitStatus.ERROR, error = "cancelled")
        }
        val c = controller(cancelled)
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertTrue(host.inserted.isEmpty())
        assertTrue(host.history.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
    }

    /** StrictMode-style (review C2): the blocking start never runs on the caller's (Main) thread. */
    @Test fun sessionStartRunsOffTheCallerThreadAndKeepsEarlyAudio() = runTest {
        val caller = Thread.currentThread()
        val startThreads = java.util.Collections.synchronizedList(mutableListOf<Thread>())
        val gate = java.util.concurrent.CountDownLatch(1)
        core.onStart = { startThreads += Thread.currentThread(); gate.await(5, java.util.concurrent.TimeUnit.SECONDS) }
        val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
        val io = pool.asCoroutineDispatcherForTest()
        try {
            val c = DictationController(
                backgroundScope, audio, host, { core }, StandardTestDispatcher(testScheduler), SessionCap(), { currentTime }, io,
            ).also { it.onFieldFocusChanged(true) }
            c.toggle(false)
            assertTrue("recording while the start connects", c.state.value is DictationState.Recording)
            audio.speak(500) // captured before the session exists
            gate.countDown()
            val until = System.currentTimeMillis() + 5_000
            while (core.samplesPushed < 8_000 && System.currentTimeMillis() < until) { runCurrent(); Thread.sleep(5) }
            assertEquals("early frames replayed", 8_000, core.samplesPushed)
            assertTrue(startThreads.isNotEmpty() && startThreads.none { it === caller })
            audio.speak(200)
            assertEquals(11_200, core.samplesPushed)
        } finally { pool.shutdownNow() }
    }

    /** The guard itself: an inline (Main) start is reported, the IO start is not. */
    @Test fun blockingCallGuardFlagsAStartOnTheCallerThread() = runTest {
        val caller = Thread.currentThread()
        val violations = java.util.Collections.synchronizedList(mutableListOf<String>())
        dev.flowclone.core.BlockingCalls.isMainThread = { Thread.currentThread() === caller }
        dev.flowclone.core.BlockingCalls.onViolation = { violations += it }
        val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
        try {
            controller().toggle(false) // Unconfined start = on the caller thread
            assertEquals(listOf("startSession"), violations.toList())
            violations.clear()
            core.cancel()
            val ioStarts = java.util.concurrent.atomic.AtomicInteger()
            core.onStart = { ioStarts.incrementAndGet() }
            val c = DictationController(
                backgroundScope, FakeAudio(), host, { core }, StandardTestDispatcher(testScheduler), SessionCap(), { currentTime },
                pool.asCoroutineDispatcherForTest(),
            ).also { it.onFieldFocusChanged(true) }
            c.toggle(false)
            val until = System.currentTimeMillis() + 5_000
            while (ioStarts.get() == 0 && System.currentTimeMillis() < until) Thread.sleep(5)
            assertEquals(1, ioStarts.get())
            assertTrue(violations.toList().toString(), violations.isEmpty())
        } finally {
            pool.shutdownNow()
            dev.flowclone.core.BlockingCalls.isMainThread = { false }
            dev.flowclone.core.BlockingCalls.onViolation = {}
        }
    }

    @Test fun cancelWhileStartingCancelsTheLateSession() = runTest {
        val gate = java.util.concurrent.CountDownLatch(1)
        core.onStart = { gate.await(5, java.util.concurrent.TimeUnit.SECONDS) }
        val pool = java.util.concurrent.Executors.newSingleThreadExecutor()
        try {
            val c = DictationController(
                backgroundScope, audio, host, { core }, StandardTestDispatcher(testScheduler), SessionCap(), { currentTime },
                pool.asCoroutineDispatcherForTest(),
            ).also { it.onFieldFocusChanged(true) }
            c.toggle(false)
            c.cancel()
            assertEquals(DictationState.Ready, c.state.value)
            gate.countDown()
            val until = System.currentTimeMillis() + 5_000
            while (core.lastConfig == null && System.currentTimeMillis() < until) Thread.sleep(5)
            Thread.sleep(50); runCurrent()
            assertFalse("late session cancelled", core.pushAudio(ShortArray(10), 10))
            advanceTimeBy(600); c.toggle(false); runCurrent()
            assertTrue(c.state.value is DictationState.Recording)
        } finally { pool.shutdownNow() }
    }

    @Test fun focusMovedDuringProcessingInsertsNothingUntilPaste() = runTest {
        host.target = InsertTarget.CHANGED
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertTrue(host.inserted.isEmpty())
        assertTrue(FeedbackEvent.TargetChanged in host.events)
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("hello world")), c.state.value)
        assertFalse(host.history.single().second)
        c.pasteRetry(); settle()
        assertEquals("explicit Paste goes into the field focused now", listOf("hello world"), host.inserted)
    }

    /** 2026-10-24 (41-word dictation lost on target_changed): the text always stays recoverable. */
    @Test fun targetChangedKeepsTheTextRecoverable() = runTest {
        host.target = InsertTarget.CHANGED
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(listOf("hello world" to "target_changed"), host.notInserted)
        assertEquals("history keeps the text", "hello world", host.history.single().first.historyText)
        // Dismissing the bubble or starting the next dictation must not be the only copy.
        c.dismissError()
        assertEquals(DictationState.Ready, c.state.value)
        assertEquals(1, host.notInserted.size)
    }

    @Test fun failedPasteAndProtectedTargetAlsoKeepTheTextRecoverable() = runTest {
        host.insertOk = false
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(listOf("hello world" to "paste_failed"), host.notInserted)
        host.notInserted.clear(); host.insertOk = true
        host.target = InsertTarget.PROTECTED; host.explicitTarget = InsertTarget.PROTECTED
        advanceTimeBy(600); c.dismissError()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(listOf("hello world" to "target_protected"), host.notInserted)
    }

    @Test fun insertedOrEmptyDictationsAreNotOffered() = runTest {
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        advanceTimeBy(600)
        c.toggle(false); advanceTimeBy(600); c.toggle(false); settle() // silence → EMPTY
        assertTrue(host.notInserted.isEmpty())
    }

    /** D35: the recording goes with the History row, styled bucket included. */
    @Test fun aDictationKeepsItsRecordingForRetry() = runTest {
        host.keepRetry = true
        host.context = FieldContext(appId = "com.example.notes", appCategory = "document")
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        val rec = host.recordings.single()!!
        assertEquals(16_000, rec.pcm.size)
        assertEquals("document", rec.appCategory)
    }

    @Test fun noRecordingWhenOffOrForAProtectedTarget() = runTest {
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(listOf<Recording?>(null), host.recordings)
        host.keepRetry = true
        host.target = InsertTarget.PROTECTED; host.explicitTarget = InsertTarget.PROTECTED
        advanceTimeBy(600)
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals("target_protected", host.outcomes.last())
        assertEquals(null, host.recordings.last())
    }

    /** D35: the recording runs again with the settings active now; nothing is inserted. */
    @Test fun retryRerunsTheRecordingWithTheSettingsActiveNow() = runTest {
        val c = controller()
        host.config = SessionConfig(languages = listOf("de"))
        core.transcript = "hallo welt"
        val ctx = FieldContext(appId = "com.example.mail", appCategory = "email")
        val out = c.retry(ShortArray(16_000) { 3000 }, ctx)
        assertEquals("hallo welt", (out as RetryOutcome.Done).result.historyText)
        assertEquals("de", core.lastConfig!!.languages.single())
        assertEquals(null, core.lastConfig!!.endpointSilenceMs)
        assertEquals(ctx, core.lastContext)
        assertEquals(16_000, core.samplesPushed)
        assertTrue("never inserts", host.inserted.isEmpty())
        assertTrue("no new History row from the controller", host.history.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
        assertFalse(c.retrying)
    }

    @Test fun retryWaitsItsTurnAndSaysWhy() = runTest {
        val c = controller()
        assertEquals(RetryOutcome.Failed(dev.flowclone.data.RetryLogic.EMPTY), c.retry(ShortArray(16_000), null))
        assertEquals(RetryOutcome.Failed(dev.flowclone.data.RetryLogic.GONE), c.retry(ShortArray(0), null))
        c.toggle(false)
        assertEquals(RetryOutcome.Failed(dev.flowclone.data.RetryLogic.BUSY), c.retry(ShortArray(16_000) { 3000 }, null))
        c.cancel()
    }

    /** blank_asr diagnostics: the core's VAD speech + peak level reach the D21 trace. */
    @Test fun traceCarriesSpeechAndPeakFromTheResult() = runTest {
        val r = CommitResult(
            dev.flowclone.core.CommitStatus.EMPTY, emptyReason = "blank_asr",
            timings = dev.flowclone.core.Timings(audioMs = 3800, speechMs = 1200, peakDbfs = -31),
        )
        val c = controller(ScriptedCore(core, r))
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        val t = host.traces.single()
        assertEquals(1200L, t.speechMs)
        assertEquals(-31, t.peakDbfs)
    }

    /** Debug "WAV as microphone": the trace names the audio source the dictation ran on. */
    @Test fun traceNamesTheAudioSource() = runTest {
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        audio.label = "wav"
        advanceTimeBy(600); c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertEquals(listOf("mic", "wav"), host.traces.map { it.source })
    }

    /** History stores why a dictation did not go in (HistoryPages.reason reads it). */
    @Test fun historyGetsTheInsertOutcome() = runTest {
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        host.target = InsertTarget.CHANGED
        advanceTimeBy(600); c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        c.dismissError(); host.target = InsertTarget.OK; host.insertOk = false
        advanceTimeBy(600); c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        c.dismissError(); host.insertOk = true
        advanceTimeBy(600); c.toggle(false); advanceTimeBy(600); c.toggle(false); settle() // silence
        assertEquals(listOf("inserted", "target_changed", "paste_failed", "empty"), host.outcomes)
        assertEquals(listOf(true, false, false, false), host.history.map { it.second })
    }

    /** The notification copies exactly what History saved (line breaks included), not the bare text runs. */
    @Test fun notInsertedTextIsTheHistoryText() = runTest {
        host.target = InsertTarget.CHANGED
        val r = CommitResult(
            dev.flowclone.core.CommitStatus.FORMATTED, finalText = "1. eggs{{BREAK}}2. milk", plainText = "1. eggs\n2. milk",
            segments = listOf(dev.flowclone.core.PasteSegment.Text("1. eggs"), dev.flowclone.core.PasteSegment.Key(dev.flowclone.core.PasteKey.BREAK), dev.flowclone.core.PasteSegment.Text("2. milk")),
        )
        dictate(r)
        assertEquals(listOf("1. eggs\n2. milk" to "target_changed"), host.notInserted)
        assertEquals(host.history.single().first.historyText, host.notInserted.single().first)
    }

    /** History → "Insert with the bubble": arms the bubble's ✓ insert with a saved text. */
    @Test fun offerInsertArmsTheBubblePasteIntoTheFieldFocusedNow() = runTest {
        val c = controller()
        assertTrue(c.offerInsert("saved text"))
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("saved text")), c.state.value)
        assertTrue("nothing typed until ✓", host.inserted.isEmpty())
        host.target = InsertTarget.CHANGED // explicit paste goes where the user is now
        c.pasteRetry(); settle()
        assertEquals(listOf("saved text"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun offerInsertNeverInterruptsADictationOrTakesBlankText() = runTest {
        val c = controller()
        assertFalse(c.offerInsert("   "))
        c.toggle(false)
        assertFalse(c.offerInsert("saved text"))
        assertTrue(c.state.value is DictationState.Recording)
    }

    @Test fun offerInsertNeverTypesIntoAProtectedField() = runTest {
        host.explicitTarget = InsertTarget.PROTECTED
        val c = controller()
        assertTrue(c.offerInsert("saved text"))
        c.pasteRetry(); settle()
        assertTrue(host.inserted.isEmpty())
        assertTrue(FeedbackEvent.TargetProtected in host.events)
    }

    @Test fun protectedFieldIsNeverInsertedEvenOnPaste() = runTest {
        host.target = InsertTarget.PROTECTED
        host.explicitTarget = InsertTarget.PROTECTED
        val c = controller()
        c.toggle(false); audio.speak(); advanceTimeBy(600); c.toggle(false); settle()
        assertTrue(FeedbackEvent.TargetProtected in host.events)
        c.pasteRetry(); settle()
        assertTrue(host.inserted.isEmpty())
        assertTrue(c.state.value is DictationState.Error)
    }

    @Test fun micErrorSubmitsWhatWasHeardAndIgnoresLateFrames() = runTest {
        val c = controller()
        c.toggle(false); audio.speak(500)
        val sink = audio.sink!!
        audio.onError!!.invoke(); runCurrent()
        assertTrue(FeedbackEvent.MicLost in host.events)
        assertTrue(c.state.value is DictationState.Processing || c.state.value == DictationState.Ready)
        val before = core.samplesPushed
        sink(ShortArray(1600) { 3000 }, 1600) // tail of the dead capture thread
        assertEquals(before, core.samplesPushed)
        settle()
        assertEquals(listOf("hello world"), host.inserted)
    }
}

/** A core whose commit blocks until [release] — simulates a hung ASR worker. */
class HangingCore(private val inner: FakeFlowCore) : FlowCore by inner {
    val release = CompletableDeferred<Unit>()
    @Volatile var cancelled = false
    override fun commit(): CommitResult {
        kotlinx.coroutines.runBlocking { release.await() }
        return inner.commit()
    }
    override fun cancel() { cancelled = true; release.complete(Unit); inner.cancel() }
}

/** Returns [result] from commit (after the inner fake has seen the audio). */
class ScriptedCore(private val inner: FakeFlowCore, private val result: CommitResult) : FlowCore by inner {
    override fun commit(): CommitResult { inner.commit(); return result }
}

private fun java.util.concurrent.ExecutorService.asCoroutineDispatcherForTest() = asCoroutineDispatcher()
