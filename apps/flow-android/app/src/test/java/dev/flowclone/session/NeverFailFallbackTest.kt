package dev.flowclone.session

import dev.flowclone.bubble.RouteLabel
import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.CorePoll
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FieldContext
import dev.flowclone.core.FlowCore
import dev.flowclone.core.RouteDiag
import dev.flowclone.core.RouteInfo
import dev.flowclone.core.SessionConfig
import dev.flowclone.core.SessionRoute
import dev.flowclone.core.StartInfo
import dev.flowclone.debug.DictationTrace
import dev.flowclone.debug.InsertOutcome
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * D24 never-fail fallback, shell side (mock engine). The router (Rust, `policy.rs` "D24") does
 * the buffering + on-demand load; this pins what the controller does with it: the session
 * starts while the phone model loads, the bubble says so, the text goes in, a cancel during the
 * load inserts nothing, and when no route is possible the error says exactly why.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class NeverFailFallbackTest {
    private class Audio : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { sink = onFrame; return true }
        override fun stop() { sink = null }
        fun speak(ms: Int) = repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) }
    }

    private class Host : DictationController.Host {
        val inserted = mutableListOf<String>()
        val traces = mutableListOf<DictationTrace>()
        var note: String? = null
        override fun feedback(e: FeedbackEvent) = Unit
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?) = Unit
        override fun hasMicPermission() = true
        override fun captureContext() = FieldContext(appId = "com.android.chrome")
        override fun sessionConfig() = SessionConfig()
        override fun onRecordingActive(active: Boolean) = Unit
        override fun onDictationTrace(trace: DictationTrace) { traces += trace }
        override fun clientFallbackNote() = note
    }

    /** What the router does with the server down and keep_local_warm off: starts at once, loading. */
    private class ColdRouter(val loadPolls: Int = 2) : FlowCore {
        val inner = FakeFlowCore("hello from the phone")
        var polls = 0; var cancels = 0; var commits = 0; var prewarms = 0
        val route = SessionRoute("local", fallback = "connect_failed")
        override fun startSession(cfg: SessionConfig) = inner.startSession(cfg)
        override fun startSessionInfo(cfg: SessionConfig): StartInfo { inner.startSession(cfg); return StartInfo(route = route, loading = true) }
        override fun poll(): CorePoll { polls++; return CorePoll(route = route, loading = polls <= loadPolls) }
        override fun pushAudio(pcm: ShortArray, length: Int) = inner.pushAudio(pcm, length)
        override fun setContext(ctx: FieldContext) = inner.setContext(ctx)
        override fun partial(): String? = null
        override fun commit(): CommitResult { commits++; return inner.commit().copy(route = RouteInfo(engine = "local", fallback = "connect_failed")) }
        override fun cancel() { cancels++; inner.cancel() }
        override fun learnEdit(pasted: String, edited: String) = inner.learnEdit(pasted, edited)
        override fun unload() = inner.unload()
        override fun prewarm(): Boolean { prewarms++; return false }
        override fun lastRouteDiag() = RouteDiag(connectMs = 2503, localCold = true, localLoadMs = 1840)
    }

    private val audio = Audio()
    private val host = Host()
    private fun TestScope.settle() { advanceTimeBy(1_000); runCurrent() }
    private fun TestScope.controller(c: () -> FlowCore) = DictationController(
        backgroundScope, audio, host, c, StandardTestDispatcher(testScheduler), now = { currentTime },
    ).also { it.onFieldFocusChanged(true) }

    @Test fun serverDownAndPhoneModelColdStillInsertsTheText() = runTest {
        val core = ColdRouter()
        val c = controller { core }
        c.toggle(hold = false)
        runCurrent()
        assertEquals("bubble says the phone model is loading", RouteLabel.LOADING_PHONE_MODEL + " · server unreachable", RouteLabel.of(c.route.value))
        audio.speak(1200)
        advanceTimeBy(DictationController.PARTIAL_POLL_MS * 4); runCurrent()
        assertEquals("via phone · server unreachable", RouteLabel.of(c.route.value))
        c.toggle(hold = false)
        settle()
        assertEquals(listOf("hello from the phone"), host.inserted)
        assertEquals(DictationState.Ready, c.state.value)
        val t = host.traces.single()
        assertEquals(InsertOutcome.INSERTED, t.insert.outcome)
        assertEquals("connect_failed", t.fallback)
        assertEquals(2503L, t.connectMs)
        assertEquals(true, t.localCold)
        assertEquals(1840L, t.localLoadMs)
    }

    @Test fun cancelWhileThePhoneModelLoadsInsertsNothing() = runTest {
        val core = ColdRouter(loadPolls = 1000)
        val c = controller { core }
        c.toggle(hold = false)
        audio.speak(500)
        advanceTimeBy(600); runCurrent()
        assertTrue(c.route.value!!.loading)
        c.cancel()
        settle()
        assertTrue(host.inserted.isEmpty())
        assertEquals(0, core.commits)
        assertEquals(1, core.cancels)
        assertEquals(InsertOutcome.CANCELLED, host.traces.single().insert.outcome)
    }

    @Test fun noRoutePossibleSaysExactlyWhy() = runTest {
        host.note = "No speech model imported. Open Flow Clone → Speech model."
        val dead = object : FlowCore by FakeFlowCore() {
            override fun startSessionInfo(cfg: SessionConfig): StartInfo =
                throw CoreException("server_unavailable: connect_failed: no handshake within 2500 ms", "server_unavailable")
        }
        val c = controller { dead }
        c.toggle(hold = false)
        settle()
        val s = c.state.value as DictationState.Error
        val msg = (s.kind as DictationState.ErrorKind.EngineUnavailable).message
        assertEquals("Server unreachable (connect_failed: no handshake within 2500 ms) — phone fallback unavailable: No speech model imported. Open Flow Clone → Speech model.", msg)
    }

    @Test fun phoneModelLoadFailureIsNamedInTheError() = runTest {
        val failed = object : FlowCore by FakeFlowCore("x") {
            override fun commit() = CommitResult(
                CommitStatus.ERROR,
                error = "server_unavailable: connect_failed: no handshake within 2500 ms; phone model failed to load: out of memory",
                route = RouteInfo(engine = "local", fallback = "connect_failed"),
            )
        }
        val c = controller { failed }
        c.toggle(hold = false); audio.speak(500); advanceTimeBy(600); c.toggle(hold = false)
        settle()
        val msg = ((c.state.value as DictationState.Error).kind as DictationState.ErrorKind.EngineUnavailable).message
        assertEquals("Server unreachable (connect_failed: no handshake within 2500 ms) and the phone model failed to load (out of memory)", msg)
    }

    @Test fun focusPrewarmsTheServerAtMostEvery20s() = runTest {
        val core = ColdRouter()
        val c = controller { core }
        runCurrent()
        assertEquals(1, core.prewarms)
        c.onFieldFocusChanged(false); c.onFieldFocusChanged(true); runCurrent()
        assertEquals("throttled", 1, core.prewarms)
        advanceTimeBy(DictationController.PREWARM_MIN_INTERVAL_MS + 1)
        c.onFieldFocusChanged(false); c.onFieldFocusChanged(true); runCurrent()
        assertEquals(2, core.prewarms)
    }

    @Test fun prewarmFailureNeverBreaksDictation() = runTest {
        var calls = 0
        val c = controller { calls++; if (calls == 1) throw CoreException("Speech model is loading…") else FakeFlowCore("ok words") }
        runCurrent()
        c.toggle(hold = false); audio.speak(500); advanceTimeBy(600); c.toggle(hold = false)
        settle()
        assertEquals(listOf("ok words"), host.inserted)
    }
}
