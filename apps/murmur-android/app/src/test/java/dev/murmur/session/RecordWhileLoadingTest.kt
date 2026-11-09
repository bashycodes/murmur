package dev.murmur.session

import dev.murmur.bubble.RouteLabel
import dev.murmur.core.CommitResult
import dev.murmur.core.CoreException
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.FieldContext
import dev.murmur.core.MurmurCore
import dev.murmur.core.SessionConfig
import dev.murmur.core.SessionRoute
import dev.murmur.core.StartInfo
import dev.murmur.debug.InsertOutcome
import dev.murmur.session.DictationState.ErrorKind
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
 * D55: pressing record while the phone's speech model loads (cold start, language
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
        val traces = mutableListOf<dev.murmur.debug.DictationTrace>()
        var endpointMs: Int? = null
        override fun feedback(e: FeedbackEvent) = Unit
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        /** D35 History entries: (status, outcome, kept recording's samples or null). */
        val history = mutableListOf<Triple<dev.murmur.core.CommitStatus, String?, Int?>>()
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) = Unit
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?, recording: Recording?) {
            history += Triple(result.status, outcome, recording?.pcm?.size)
        }
        override fun keepRetryAudio() = true
        override fun hasMicPermission() = true
        override fun captureContext() = FieldContext(appId = "com.example.notes")
        override fun sessionConfig() = SessionConfig(languages = listOf("he"))
        override fun onRecordingActive(active: Boolean) = Unit
        override fun endpointSilenceMs() = endpointMs
        override fun engineLoading(message: String) { loadingNotices += message }
        override fun onDictationTrace(trace: dev.murmur.debug.DictationTrace) { traces += trace }
        /** D58: the keyboard's languages at the stop (null = not followed) and what preparing them does. */
        var keyboardAtStop: List<String>? = null
        var onPrepare: () -> Unit = {}
        override fun followsKeyboard() = keyboardAtStop != null
        override fun stopLanguage() = keyboardAtStop?.let { DictationController.StopLanguage(it, "keyboard") }
        override fun prepareStopLanguage() = onPrepare()
    }

    /**
     * A hybrid router with a server first in the route order: a normal start would go to the
     * server ([SessionRoute] "remote"); [startSessionLocal] is the phone-only start.
     */
    private class HybridRouter(val inner: FakeMurmurCore = FakeMurmurCore("a synthetic sentence")) : MurmurCore by inner {
        var localStarts = 0
        var serverStarts = 0
        /** The engine a normal start reports ("local": the router chose the phone). */
        var startEngine = "remote"
        override fun startSessionInfo(cfg: SessionConfig): StartInfo {
            serverStarts++
            inner.startSession(cfg)
            return StartInfo(route = SessionRoute(startEngine, server = "192.0.2.1:7313".takeIf { startEngine == "remote" }))
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
    private var load = CompletableDeferred<LoadedEngine>()
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

    /** [phoneOnly] = false: the load was the D24 on-demand route's router, not a phone model. */
    private fun finishLoad(phoneOnly: Boolean = true) { loading = false; load.complete(LoadedEngine(router, phoneOnly)) }

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

    /**
     * D58: the stop language needs the other model: it loads at the stop, the bubble names it
     * while processing, and the recording is still transcribed (on the phone) and inserted.
     */
    @Test fun aStopLanguageWhoseModelIsNotLoadedLoadsItThenInserts() = runTest {
        loading = false
        router.startEngine = "local"
        host.keyboardAtStop = listOf("he")
        host.onPrepare = { loading = true } // the Hebrew model replaces the default one
        val c = controller()
        c.toggle(hold = false)
        assertEquals("he", router.inner.lastConfig!!.languages.single())
        mic.speak(1000)
        host.keyboardAtStop = listOf("en")
        tapAgain(c)
        runCurrent()
        assertTrue(c.state.value is DictationState.Processing)
        assertEquals(LABEL, c.label())
        finishLoad()
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
        assertEquals("en", router.inner.lastConfig!!.languages.single())
        assertEquals("phone model: a local start", 1, router.localStarts)
        assertNotNull(host.traces.single().loadWaitMs)
        assertEquals("en", host.traces.single().stopLanguage)
    }

    /** D58: X while the stop language's model loads drops the dictation; the load's end starts nothing. */
    @Test fun cancelWhileTheStopLanguageLoadsDropsTheDictation() = runTest {
        loading = false
        router.startEngine = "local"
        host.keyboardAtStop = listOf("he")
        host.onPrepare = { loading = true }
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        host.keyboardAtStop = listOf("en")
        tapAgain(c)
        runCurrent()
        assertEquals(LABEL, c.label())
        c.cancel()
        assertEquals(DictationState.Ready, c.state.value)
        finishLoad()
        settle()
        assertTrue(host.inserted.isEmpty())
        assertEquals("no re-run started", 0, router.localStarts)
        assertEquals(1, router.serverStarts)
        assertEquals(InsertOutcome.CANCELLED, host.traces.single().insert.outcome)
    }

    /** D58 hybrid: a dictation the server took is re-run there; the phone model is not swapped. */
    @Test fun aServerDictationReRunsOnTheServerWithoutSwappingThePhoneModel() = runTest {
        loading = false
        host.keyboardAtStop = listOf("he")
        var prepared = 0
        host.onPrepare = { prepared++; loading = true }
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        host.keyboardAtStop = listOf("en")
        tapAgain(c)
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
        assertEquals(0, prepared)
        assertEquals(2, router.serverStarts)
        assertEquals(0, router.localStarts)
        assertEquals("en", router.inner.lastConfig!!.languages.single())
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
        assertEquals("kept for History's Retry", listOf(Triple(dev.murmur.core.CommitStatus.ERROR, InsertOutcome.ERROR, 16_000)), host.history)
    }

    @Test fun aLoadFailureWhileRecordingStopsTheMicAndShowsTheError() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        load.completeExceptionally(CoreException("The phone speech model did not load", null))
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable("The phone speech model did not load")), c.state.value)
        assertNull("microphone stopped", mic.sink)
        assertEquals("kept for History's Retry", listOf(Triple(dev.murmur.core.CommitStatus.ERROR, InsertOutcome.ERROR, 16_000)), host.history)
    }

    @Test fun aLoadThatNeverEndsGivesUpAfterTheBound() = runTest {
        val c = controller()
        c.toggle(hold = true)
        mic.speak(1000)
        c.release()
        advanceTimeBy(DictationController.LOAD_WAIT_MS + 1)
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable(DictationController.LOAD_TIMEOUT)), c.state.value)
        assertEquals(listOf(Triple(dev.murmur.core.CommitStatus.ERROR, InsertOutcome.ERROR, 16_000)), host.history)
    }

    @Test fun recordingPastTheLoadBoundStopsAndGoesToRetry() = runTest {
        val c = controller()
        c.toggle(hold = true)
        // Still loading after the bound: the mic stops there, the recording is kept for Retry.
        repeat((DictationController.LOAD_WAIT_MS / 1000).toInt() + 5) { mic.speak(1000); advanceTimeBy(1000) }
        runCurrent()
        assertEquals(DictationState.Error(ErrorKind.EngineUnavailable(DictationController.LOAD_TIMEOUT)), c.state.value)
        assertNull("microphone stopped", mic.sink)
        val kept = host.history.single()
        assertEquals(InsertOutcome.ERROR, kept.second)
        assertTrue("the recording up to the bound", kept.third!! >= (DictationController.LOAD_WAIT_MS / 1000 * SAMPLE_RATE).toInt())
    }

    @Test fun aCancelledDictationKeepsNothing() = runTest {
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        c.cancel()
        load.completeExceptionally(CoreException("no model", null))
        settle()
        assertTrue(host.history.isEmpty())
    }

    @Test fun theOnDemandRouteStartsOnItsNormalRouteOnceTheRouterIsBack() = runTest {
        // D24 keep-warm off with the server first: the language switch rebuilds the router; no
        // phone model loads, so the held audio starts as that route normally would.
        val c = controller()
        c.toggle(hold = false)
        mic.speak(1000)
        finishLoad(phoneOnly = false)
        runCurrent()
        assertEquals(1, router.serverStarts)
        assertEquals(0, router.localStarts)
        assertEquals(16_000, router.inner.samplesPushed)
        assertEquals("the route the router reported", "remote", c.route.value?.engine)
        tapAgain(c)
        settle()
        assertEquals(listOf("a synthetic sentence"), host.inserted)
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

    @Test fun anEngineThatCannotKeepTheRecordingOnThePhoneRefusesALocalStart() {
        var started = false
        val engine = object : MurmurCore {
            override fun startSession(cfg: SessionConfig) { started = true }
            override fun pushAudio(pcm: ShortArray, length: Int) = true
            override fun setContext(ctx: FieldContext) = Unit
            override fun partial(): String? = null
            override fun commit() = CommitResult(dev.murmur.core.CommitStatus.EMPTY)
            override fun cancel() = Unit
            override fun learnEdit(pasted: String, edited: String) = emptyList<dev.murmur.core.LearnCandidate>()
            override fun unload() = Unit
        }
        val e = runCatching { engine.startSessionLocal(SessionConfig()) }.exceptionOrNull() as CoreException
        assertEquals("local_not_ready", e.kind)
        assertTrue("never fell back to a normal start", !started)
    }

    private companion object {
        const val LABEL = "Loading the Hebrew speech model…"
    }
}
