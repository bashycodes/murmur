package dev.flowclone.session

import dev.flowclone.core.CommitResult
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.SessionConfig
import dev.flowclone.session.DictationState.ErrorKind
import kotlinx.coroutines.CompletableDeferred
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
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean {
            starts++
            if (!startOk) return false
            sink = onFrame; return true
        }
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
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?) { history += result to inserted }
        override fun hasMicPermission() = micGranted
        override fun captureContext() = context
        var config = SessionConfig(languages = listOf("en"))
        override fun sessionConfig() = config
        override fun onRecordingActive(active: Boolean) { recordingActive += active }
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
        }
        val c = controller(refusing)
        c.toggle(false)
        assertEquals(
            DictationState.Error(ErrorKind.EngineUnavailable("Dictation disabled — enable server or client in Developer settings")),
            c.state.value,
        )
        assertEquals(0, audio.starts)
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
