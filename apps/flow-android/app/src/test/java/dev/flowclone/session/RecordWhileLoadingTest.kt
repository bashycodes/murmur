package dev.flowclone.session

import dev.flowclone.bubble.RouteLabel
import dev.flowclone.core.CommitResult
import dev.flowclone.core.CoreException
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.SessionConfig
import dev.flowclone.core.SessionRoute
import dev.flowclone.core.StartInfo
import dev.flowclone.debug.InsertOutcome
import dev.flowclone.session.DictationState.ErrorKind
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Load 2 (D55): pressing record while the phone's speech model loads (cold start, language
 * switch) records at once; the held audio is transcribed on the phone once the model is loaded.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class RecordWhileLoadingTest {
    private class Mic : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        var starts = 0
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { starts++; sink = onFrame; return true }
        override fun stop() { sink = null }
        fun speak(ms: Int) = repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) }
        fun silence(ms: Int) = repeat(ms / 100) { sink?.invoke(ShortArray(1600), 1600) }
    }

    private class Host : DictationController.Host {
        val inserted = mutableListOf<String>()
        val loadingNotices = mutableListOf<String>()
        val traces = mutableListOf<dev.flowclone.debug.DictationTrace>()
        var endpointMs: Int? = null
        override fun feedback(e: FeedbackEvent) = Unit
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) = Unit
        override fun hasMicPermission() = true
        override fun captureContext() = FieldContext(appId = "com.example.notes")
        override fun sessionConfig() = SessionConfig(languages = listOf("he"))
        override fun onRecordingActive(active: Boolean) = Unit
        override fun endpointSilenceMs() = endpointMs
        override fun engineLoading(message: String) { loadingNotices += message }
        override fun onDictationTrace(trace: dev.flowclone.debug.DictationTrace) { traces += trace }
    }

    /**
     * A hybrid router with a server first in the route order: a normal start would go to the
     * server ([SessionRoute] "remote"); [startSessionLocal] is the phone-only start.
     */
    private class HybridRouter(val inner: FakeFlowCore = FakeFlowCore("a synthetic sentence")) : FlowCore by inner {
        var localStarts = 0
        var serverStarts = 0
        override fun startSessionInfo(cfg: SessionConfig): StartInfo {
            serverStarts++
            inner.startSession(cfg)
            return StartInfo(route = SessionRoute("remote", server = "192.0.2.1:7313"))
        }
        override fun startSessionLocal(cfg: SessionConfig): StartInfo {
            localStarts++
            inner.startSession(cfg)
            return StartInfo(route = SessionRoute("local"))
        }
    }

    private val mic = Mic()
    private val host = Host()
    private val router = HybridRouter()
    @Volatile private var loading = true
    private var load = CompletableDeferred<FlowCore>()
    private var waits = 0

    private fun TestScope.controller(wait: Boolean = true) = DictationController(
        scope = backgroundScope, audio = mic, host = host,
        core = { if (loading) throw CoreException(LABEL, CoreException.KIND_LOADING) else router },
        commitDispatcher = StandardTestDispatcher(testScheduler), now = { currentTime },
        loadWait = { if (!wait) null else LoadWait(LABEL) { waits++; load.await() } },
    ).also { it.onFieldFocusChanged(true) }

    private fun TestScope.settle() { advanceTimeBy(1_000); runCurrent() }

    /** A second tap, past the 500 ms toggle debounce. */
    private fun TestScope.tapAgain(c: DictationController) { advanceTimeBy(DictationController.TOGGLE_DEBOUNCE_MS); c.toggle(hold = false) }

    private fun finishLoad() { loading = false; load.complete(router) }

    private fun DictationController.label() = RouteLabel.of(route.value)

    @Test fun recordingStartsAtOnceAndTheBubbleNamesTheLoadingModel() = runTest {
        val c = controller()
        c.toggle(hold = false)
        assertTrue(c.state.value is DictationState.Recording)
        assertEquals(1, mic.starts)
        assertEquals(LABEL, c.label())
        assertTrue("no 'still loading' refusal", host.loadingNotices.isEmpty())
        mic.speak(1000)
        finishLoad()
        runCurrent()
        assertEquals("the held second reached the phone engine", 16_000, router.inner.samplesPushed)
        assertEquals("via phone", c.label())
        mic.speak(500)
        assertEquals(24_000, router.inner.samplesPushed)
        tapAgain(c)
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun heldAudioIsTranscribedOnThePhoneNeverByTheServer() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        finishLoad()
        runCurrent()
        assertEquals(1, router.localStarts)
        assertEquals("the server-first route order was not used", 0, router.serverStarts)
        tapAgain(c)
        settle()
        // The next dictation (model loaded) follows the route order again.
        advanceTimeBy(DictationController.TOGGLE_DEBOUNCE_MS)
        c.toggle(hold = false)
        runCurrent()
        assertEquals(1, router.serverStarts)
        assertEquals(1, router.localStarts)
    }

    @Test fun stoppingBeforeTheLoadEndsWaitsThenInsertsEverything() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(3000)
        tapAgain(c)
        assertTrue(c.state.value is DictationState.Processing)
        assertEquals("still says which model it waits for", LABEL, c.label())
        settle()
        assertTrue(c.state.value is DictationState.Processing)
        finishLoad()
        settle()
        assertEquals(48_000, router.inner.samplesPushed)
        assertEquals(listOf("a synthetic sentence"), host.inserted)
        val t = host.traces.single()
        assertEquals(InsertOutcome.INSERTED, t.insert.outcome)
        assertNotNull("debug log says it waited for the model", t.loadWaitMs)
    }

    @Test fun moreThanThirtySecondsAreHeldWhileLoading() = runTest {
        val c = controller()
        c.toggle(hold = true)
        mic.speak(40_000)
        finishLoad()
        runCurrent()
        assertEquals("nothing dropped (the 30 s connect buffer does not apply)", 640_000, router.inner.samplesPushed)
        c.release()
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
    }

    @Test fun cancellingWhileLoadingDropsTheAudioAndAllowsANewDictation() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        c.cancel()
        assertEquals(DictationState.Ready, c.state.value)
        assertNull(c.route.value)
        advanceTimeBy(DictationController.TOGGLE_DEBOUNCE_MS)
        c.toggle(hold = false)
        assertTrue("a new start is not blocked by the abandoned wait", c.state.value is DictationState.Recording)
        mic.speak(500)
        finishLoad()
        runCurrent()
        assertEquals("only the second dictation started a session", 1, router.localStarts)
        assertEquals(8_000, router.inner.samplesPushed)
    }

    @Test fun aLoadFailureAfterTheRecordingEndedShowsTheError() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        tapAgain(c)
        load.completeExceptionally(CoreException("Speech model file is missing: synthetic.bin", "model_not_found"))
        settle()
        assertEquals(
            DictationState.Error(ErrorKind.EngineUnavailable("Speech model file is missing (synthetic.bin). Re-import it.")),
            c.state.value,
        )
        assertEquals(0, router.localStarts + router.serverStarts)
        assertEquals(InsertOutcome.START_FAILED, host.traces.single().insert.outcome)
    }

    @Test fun aLoadFailureWhileRecordingStopsTheMicAndShowsTheError() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        load.completeExceptionally(CoreException("The phone speech model did not load", null))
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable("The phone speech model did not load")), c.state.value)
        assertNull("microphone stopped", mic.sink)
    }

    @Test fun aLoadThatNeverEndsGivesUpAfterTheBound() = runTest {
        val c = controller()
        c.toggle(hold = true)
        mic.speak(1000)
        c.release()
        advanceTimeBy(DictationController.LOAD_WAIT_MS + 1)
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable(DictationController.LOAD_TIMEOUT)), c.state.value)
    }

    @Test fun handsFreeEndsOnSilenceWhileTheModelLoads() = runTest {
        host.endpointMs = 2000
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        mic.silence(2100)
        runCurrent()
        assertTrue("stopped like a live endpoint", c.state.value is DictationState.Processing)
        finishLoad()
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
    }

    @Test fun withoutAPhoneModelToWaitForTheStartIsRefusedAsBefore() = runTest {
        val c = controller(wait = false)
        c.toggle(hold = false)
        assertEquals(DictationState.Error(ErrorKind.EngineLoading(LABEL)), c.state.value)
        assertEquals(listOf(LABEL), host.loadingNotices)
        assertEquals(0, mic.starts)
        assertEquals(0, waits)
    }

    private companion object {
        const val LABEL = "Loading the Hebrew speech model…"
    }
}
