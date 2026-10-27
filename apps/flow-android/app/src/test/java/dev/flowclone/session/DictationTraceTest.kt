package dev.flowclone.session

import dev.flowclone.core.CommitResult
import dev.flowclone.core.CoreException
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.FieldContext
import dev.flowclone.core.SessionConfig
import dev.flowclone.core.SessionRoute
import dev.flowclone.debug.CommitLine
import dev.flowclone.debug.DebugLogFile
import dev.flowclone.debug.DebugLogger
import dev.flowclone.debug.DictationTrace
import dev.flowclone.debug.InsertOutcome
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

/** D21: the debug trace per dictation, and that logging can never change a dictation. */
@OptIn(ExperimentalCoroutinesApi::class)
class DictationTraceTest {
    private class Audio : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { sink = onFrame; return true }
        override fun stop() { sink = null }
        fun speak(ms: Int) = repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) }
        fun quiet(ms: Int) = repeat(ms / 100) { sink?.invoke(ShortArray(1600), 1600) }
        /** Real phone "silence": room noise / mic self-noise, never digital zero. */
        fun roomNoise(ms: Int, amp: Int, seed: Int = 7) {
            val r = java.util.Random(seed.toLong())
            repeat(ms / 100) { sink?.invoke(ShortArray(1600) { (r.nextGaussian() * amp).toInt().toShort() }, 1600) }
        }
    }

    /** Mirrors FlowService.Host: history = the `commit …` logcat line; traces go to [onTrace]. */
    private class Host(val onTrace: (DictationTrace) -> Unit) : DictationController.Host {
        val inserted = mutableListOf<String>()
        val commitLines = mutableListOf<String>()
        override fun feedback(e: FeedbackEvent) = Unit
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) { commitLines += CommitLine.of(result) }
        override fun hasMicPermission() = true
        override fun captureContext() = FieldContext(appId = "com.example.notes")
        override fun sessionConfig() = SessionConfig()
        override fun onRecordingActive(active: Boolean) = Unit
        override fun onDictationTrace(trace: DictationTrace) = onTrace(trace)
    }

    private val audio = Audio()
    private val core = FakeFlowCore(transcript = "my secret phrase")
    private fun TestScope.settle() { advanceTimeBy(1_000); runCurrent() }

    private fun TestScope.controller(host: DictationController.Host) = DictationController(
        scope = backgroundScope, audio = audio, host = host, core = { core },
        commitDispatcher = StandardTestDispatcher(testScheduler), now = { currentTime }, wallClock = { 1_000_000L },
    ).also { it.onFieldFocusChanged(true) }

    private fun TestScope.dictate(c: DictationController, hold: Boolean = false) {
        c.toggle(hold)
        audio.speak(800)
        audio.quiet(300)
        advanceTimeBy(600)
        if (hold) c.release() else c.toggle(false)
        settle()
    }

    @Test fun insertedDictationProducesOneFullTrace() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val host = Host { traces += it }
        dictate(controller(host))
        val t = traces.single()
        assertEquals("tap", t.mode)
        assertEquals(1_000_000L, t.tsMs)
        assertEquals("com.example.notes", t.target)
        assertEquals(InsertOutcome.INSERTED, t.insert.outcome)
        assertEquals("my secret phrase", t.raw)
        assertEquals("my secret phrase", t.text)
        assertEquals(300L, t.endpointWaitMs)
        assertEquals(1_100L, t.audioMs)
        assertNotNull(t.perceivedMs)
        assertTrue(t.perceivedMs!! >= 0)
        assertNull(t.error)
    }

    @Test fun holdModeIsRecorded() = runTest {
        val traces = mutableListOf<DictationTrace>()
        dictate(controller(Host { traces += it }), hold = true)
        assertEquals("hold", traces.single().mode)
    }

    @Test fun offlineFallbackRouteIsRecorded() = runTest {
        val traces = mutableListOf<DictationTrace>()
        core.route = SessionRoute(engine = "local", fallback = "connect_failed")
        dictate(controller(Host { traces += it }))
        assertEquals("local", traces.single().route)
        assertEquals("connect_failed", traces.single().fallback)
    }

    @Test fun serverUnreachableStartIsRecorded() = runTest {
        val traces = mutableListOf<DictationTrace>()
        core.onStart = { throw CoreException("server_unavailable: connect refused", "server_unavailable") }
        val c = controller(Host { traces += it })
        c.toggle(false); settle()
        val t = traces.single()
        assertEquals(InsertOutcome.START_FAILED, t.insert.outcome)
        assertEquals("server_unavailable", t.fallback)
        assertTrue(t.error!!.contains("unreachable"))
        assertNotNull(t.stack)
    }

    @Test fun aThrowingTraceHookNeverChangesTheDictation() = runTest {
        val host = Host { throw IllegalStateException("log broke") }
        val c = controller(host)
        dictate(c)
        assertEquals(listOf("my secret phrase"), host.inserted)
        assertEquals(1, host.commitLines.size)
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun aSinkThatThrowsOnEveryWriteStillInsertsAndLogsTheCommitLine() = runTest {
        val logcat = mutableListOf<String>()
        val log = DebugLogger({ true }, { throw java.io.IOException("No space left on device") }, { logcat += it }, runner = { it.run() })
        val host = Host(log::dictation)
        val c = controller(host)
        dictate(c)
        dictate(c)
        assertEquals(listOf("my secret phrase", "my secret phrase"), host.inserted)
        assertEquals(2, host.commitLines.size)
        assertTrue(host.commitLines.all { it.startsWith("commit RAW_TRANSCRIPT") })
        assertEquals(2, log.errors.get())
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun offNoTranscriptReachesLogcatOrFile() = runTest {
        val dir = Files.createTempDirectory("flowdebug-off").toFile()
        val file = DebugLogFile.inFilesDir(dir)
        val logcat = mutableListOf<String>()
        val log = DebugLogger({ false }, file, { logcat += it }, runner = { it.run() })
        val host = Host(log::dictation)
        dictate(controller(host))
        assertEquals(listOf("my secret phrase"), host.inserted)
        assertTrue("FlowDebug got $logcat", logcat.isEmpty())
        assertFalse(file.file.exists())
        assertFalse(File(dir, "debug").exists())
        // The always-on FlowService line carries timings only.
        val line = host.commitLines.single()
        assertFalse(line, line.contains("secret"))
        assertTrue(line, line.contains("audio=1100ms"))
    }

    @Test fun onTheLineReachesLogcatAndFile() = runTest {
        val dir = Files.createTempDirectory("flowdebug-on").toFile()
        val file = DebugLogFile.inFilesDir(dir)
        val logcat = mutableListOf<String>()
        val log = DebugLogger({ true }, file, { logcat += it }, runner = { it.run() })
        dictate(controller(Host(log::dictation)))
        val o = JSONObject(file.readLines().single())
        assertEquals("my secret phrase", o.getString("text"))
        assertEquals(logcat.single(), file.readLines().single())
        assertEquals("inserted", o.getJSONObject("insert").getString("outcome"))
    }

    @Test fun cancelIsTraced() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val c = controller(Host { traces += it })
        c.toggle(false); audio.speak(300); c.cancel(); settle()
        assertEquals(InsertOutcome.CANCELLED, traces.single().insert.outcome)
    }

    @Test fun streamingTimingsReachTheTrace() = runTest {
        val traces = mutableListOf<DictationTrace>()
        core.shapeResult = { it.copy(timings = it.timings.copy(waitMs = 90, segments = 2, preAsrMs = 1300), route = dev.flowclone.core.RouteInfo("local", clientMs = 450)) }
        dictate(controller(Host { traces += it }))
        val t = traces.single()
        assertEquals(90L, t.waitMs)
        assertEquals(2L, t.segments)
        assertEquals(1300L, t.preAsrMs)
        assertEquals(450L, t.clientMs)
    }

    /** 2026-10-27: the unused AI cleanup answer and the guard's detail reach the trace. */
    @Test fun rejectedAiAnswerReachesTheTrace() = runTest {
        val traces = mutableListOf<DictationTrace>()
        core.shapeResult = {
            it.copy(route = dev.flowclone.core.RouteInfo("local", cleanupRoute = "local", cloudFallback = "llm_unsupported",
                rejectedText = "Please buy a car.", guardDetail = "unsupported: Please buy a car"))
        }
        dictate(controller(Host { traces += it }))
        val t = traces.single()
        assertEquals("llm_unsupported", t.cloudFallback)
        assertEquals("Please buy a car.", t.rejectedText)
        assertEquals("unsupported: Please buy a car", t.guardDetail)
    }

    /**
     * 2026-10-23 phone diag (docs/findings/2026-10-23-phone-quality-regression.md): every device
     * line had endpoint_wait_ms=0. SilenceTracker's fixed bar (mean s^2 > 33856, RMS 184 ~ -45 dBFS,
     * Wispr's "can't hear you" threshold) is below a real mic's room-noise floor, so trailing
     * silence after the last word still counts as signal. Expected: ~300 ms. Fixed by the noise-floor-adaptive
     * bar for the trailing measure (SilenceTracker).
     */
    @Test fun endpointWaitCountsRoomNoiseAfterSpeechAsSilence() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val c = controller(Host { traces += it })
        c.toggle(false)
        audio.speak(800)            // RMS 3000 ~ -21 dBFS
        audio.roomNoise(300, 400)   // RMS ~400 ~ -38 dBFS: a quiet room on a phone mic
        advanceTimeBy(600)
        c.toggle(false); settle()
        val wait = traces.single().endpointWaitMs!!
        assertTrue("endpoint_wait_ms=$wait", wait in 200L..400L)
    }

    @Test fun cloudModelAndSessionLanguageReachTheTrace() = runTest {
        val traces = mutableListOf<DictationTrace>()
        core.shapeResult = { it.copy(route = dev.flowclone.core.RouteInfo("cloud", provider = "openai", model = "whisper-1")) }
        val host = object : DictationController.Host by Host({ traces += it }) {
            override fun sessionConfig() = SessionConfig(languages = listOf("en"))
            override fun onDictationTrace(trace: DictationTrace) { traces += trace }
        }
        dictate(controller(host))
        assertEquals("whisper-1", traces.single().model)
        assertEquals("en", traces.single().language)
    }

    @Test fun autoLanguageIsRecordedAsAuto() = runTest {
        val traces = mutableListOf<DictationTrace>()
        dictate(controller(Host { traces += it }))
        assertEquals("auto", traces.single().language)
    }

    // ---- "Keep last dictation audio (debug)" ----

    private class AudioHost(val keep: Boolean, val traces: MutableList<DictationTrace>, val throwOnAudio: Boolean = false) :
        DictationController.Host by Host({ traces += it }) {
        val saved = mutableListOf<Pair<Long, Int>>()
        override fun keepDictationAudio() = keep
        override fun onDictationAudio(trace: DictationTrace, pcm: ShortArray): String? {
            if (throwOnAudio) throw IllegalStateException("disk full")
            saved += trace.tsMs to pcm.size
            return "a.wav"
        }
        override fun onDictationTrace(trace: DictationTrace) { traces += trace }
    }

    @Test fun keptAudioIsTheWholeRecordingAndLinkedFromTheTrace() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val host = AudioHost(keep = true, traces)
        dictate(controller(host))
        assertEquals(listOf(1_000_000L to 1_100 * 16), host.saved)
        assertEquals("a.wav", traces.single().audio)
    }

    @Test fun audioIsNotKeptWhenOff() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val host = AudioHost(keep = false, traces)
        dictate(controller(host))
        assertTrue(host.saved.isEmpty())
        assertNull(traces.single().audio)
    }

    @Test fun cancelledDictationKeepsNoAudio() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val host = AudioHost(keep = true, traces)
        val c = controller(host)
        c.toggle(false); audio.speak(300); c.cancel(); settle()
        assertTrue(host.saved.isEmpty())
    }

    @Test fun aThrowingAudioHookNeverChangesTheDictation() = runTest {
        val traces = mutableListOf<DictationTrace>()
        val c = controller(AudioHost(keep = true, traces, throwOnAudio = true))
        dictate(c)
        assertEquals(InsertOutcome.INSERTED, traces.single().insert.outcome)
        assertNull(traces.single().audio)
        assertEquals(DictationState.Ready, c.state.value)
    }
}
