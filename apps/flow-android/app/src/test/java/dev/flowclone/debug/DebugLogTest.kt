package dev.flowclone.debug

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class DebugLogTest {
    private val trace = DictationTrace(
        tsMs = 1_761_214_502_123, mode = "tap", route = "local", fallback = "connect_failed", rttMs = null,
        audioMs = 2_400, endpointWaitMs = 2_050, asrMs = 640, postMs = 12, totalMs = 700, perceivedMs = 820,
        target = "com.example.notes", insert = InsertOutcome(InsertOutcome.INSERTED, listOf("commitText", "verify")),
        raw = "hello world", text = "Hello world.",
    )

    private fun tmpDir(): File = Files.createTempDirectory("flowdebug").toFile().apply { deleteOnExit() }

    @Test fun dictationLineHasTheSharedSchema() {
        val o = JSONObject(DebugFormat.dictation(trace))
        for (k in listOf("ts", "mode", "route", "fallback", "rtt_ms", "audio_ms", "endpoint_wait_ms", "asr_ms", "post_ms",
            "total_ms", "perceived_ms", "target", "insert", "raw", "text", "error")) assertTrue("missing $k", o.has(k))
        assertEquals("2025-10-23T10:15:02.123Z", o.getString("ts"))
        assertEquals("tap", o.getString("mode"))
        assertEquals("connect_failed", o.getString("fallback"))
        assertTrue(o.isNull("rtt_ms"))
        assertEquals(820, o.getInt("perceived_ms"))
        assertEquals("inserted", o.getJSONObject("insert").getString("outcome"))
        assertEquals(2, o.getJSONObject("insert").getJSONArray("steps").length())
        assertEquals("Hello world.", o.getString("text"))
        assertEquals("dictation", o.getString("kind"))
    }

    /** D27: the style sent and what the LLM did; llm_skipped_clean is a normal result in the viewer. */
    @Test fun styleAndLlmFieldsAreAdditive() {
        val o = JSONObject(DebugFormat.dictation(trace))
        assertTrue(o.has("style") && o.isNull("style"))
        assertTrue(o.has("llm") && o.isNull("llm"))
        val t = trace.copy(style = "casual", llm = "llm_skipped_clean")
        val p = JSONObject(DebugFormat.dictation(t))
        assertEquals("casual", p.getString("style"))
        assertEquals("llm_skipped_clean", p.getString("llm"))
        val sum = DebugFormat.summary(DebugFormat.dictation(t))
        assertTrue(sum, sum.contains("· casual") && sum.contains("AI not needed (already clean)"))
    }

    /** 2026-10-24: route order + blank_asr diagnostics, additive (null when unknown). */
    @Test fun routeOrderAndSpeechDiagnosticsAreAdditive() {
        val o = JSONObject(DebugFormat.dictation(trace))
        for (k in listOf("route_order", "speech_ms", "peak_dbfs")) assertTrue(k, o.has(k) && o.isNull(k))
        val p = JSONObject(DebugFormat.dictation(trace.copy(routeOrder = "client,server", speechMs = 1200, peakDbfs = -31)))
        assertEquals("client,server", p.getString("route_order"))
        assertEquals(1200, p.getInt("speech_ms"))
        assertEquals(-31, p.getInt("peak_dbfs"))
        val sum = DebugFormat.summary(DebugFormat.dictation(trace.copy(speechMs = 1200, peakDbfs = -31)))
        assertTrue(sum, sum.contains("speech 1200 ms") && sum.contains("peak -31 dBFS"))
    }

    /** D26: each line pins the exact APK (version + commit), so a log excerpt identifies the build. */
    @Test fun everyLineNamesTheBuild() {
        val cur = dev.flowclone.data.AppVersion.current
        for (line in listOf(DebugFormat.dictation(trace), DebugFormat.event(DebugEvent(0, "engine_load", null)))) {
            val o = JSONObject(line)
            assertEquals(line, cur.version, o.getString("app_version"))
            assertEquals(line, cur.build, o.getString("build"))
        }
        val keep = DebugFormat.build
        try {
            DebugFormat.build = dev.flowclone.data.AppBuild("0.2.0", 200, "abc1234-dirty", "0.2.0+abc1234.dirty")
            val o = JSONObject(DebugFormat.dictation(trace))
            assertEquals("0.2.0", o.getString("app_version"))
            assertEquals("abc1234-dirty", o.getString("build"))
            assertTrue(DebugFormat.dictation(trace).startsWith("{\"kind\":\"dictation\",\"ts\":\"2025-10-23T10:15:02.123Z\",\"app_version\":\"0.2.0\",\"build\":"))
        } finally {
            DebugFormat.build = keep
        }
    }

    @Test fun quotesAndNewlinesStayOneLine() {
        val line = DebugFormat.dictation(trace.copy(text = "a \"quote\"\nnext line"))
        assertFalse(line.contains('\n'))
        assertEquals("a \"quote\"\nnext line", JSONObject(line).getString("text"))
    }

    @Test fun protectedTargetsAreRedacted() {
        for ((field, app) in listOf(true to false, false to true)) {
            val r = DebugRedact.apply(trace, protectedField = field, protectedApp = app)
            assertEquals(DebugRedact.REDACTED, r.target)
            assertEquals(DebugRedact.REDACTED, r.raw)
            assertEquals(DebugRedact.REDACTED, r.text)
            assertEquals(820L, r.perceivedMs)
        }
        val guarded = DebugRedact.apply(trace.copy(insert = InsertOutcome(InsertOutcome.TARGET_PROTECTED)), false, false)
        assertEquals(DebugRedact.REDACTED, guarded.target)
        assertEquals(trace, DebugRedact.apply(trace, false, false))
    }

    @Test fun protectedFieldsAndAppsAreRecognised() {
        assertTrue(DebugRedact.isProtectedField(0x81, false, false)) // TEXT | TEXT_VARIATION_PASSWORD
        assertTrue(DebugRedact.isProtectedField(0x12, false, false)) // NUMBER | NUMBER_VARIATION_PASSWORD
        assertTrue(DebugRedact.isProtectedField(0x01, true, false))
        assertTrue(DebugRedact.isProtectedField(0x01, false, true))
        assertFalse(DebugRedact.isProtectedField(0x01, false, false))
        assertTrue(DebugRedact.isProtectedApp("com.chase.sig.android", emptySet()))
        assertTrue(DebugRedact.isProtectedApp("com.example.secret", setOf("com.example.secret")))
        assertFalse(DebugRedact.isProtectedApp("com.example.notes", emptySet()))
        assertFalse(DebugRedact.isProtectedApp(null, emptySet()))
    }

    @Test fun secretsAreScrubbedFromEveryLine() {
        val token = "0123456789abcdef0123"
        val lines = mutableListOf<String>()
        val log = DebugLogger({ true }, { lines += it }, { lines += it }, secrets = { listOf(token, "sk-cloudkey-123456") }, runner = { it.run() })
        log.event("server", "auth failed for token $token and sk-cloudkey-123456")
        assertEquals(2, lines.size)
        for (l in lines) { assertFalse(l.contains(token)); assertFalse(l.contains("sk-cloudkey")) }
    }

    @Test fun offWritesNothingAnywhere() {
        val dir = tmpDir()
        val file = DebugLogFile(File(dir, "d.jsonl"))
        val logcat = mutableListOf<String>()
        var ran = 0
        val log = DebugLogger({ false }, file, { logcat += it }, runner = { ran++; it.run() })
        log.dictation(trace)
        log.event("engine_load", "boom", RuntimeException("x"))
        assertTrue(logcat.isEmpty())
        assertFalse(file.file.exists())
        assertEquals("nothing even queued", 0, ran)
    }

    @Test fun ringBufferKeepsTheNewest() {
        val f = DebugLogFile(File(tmpDir(), "debug/d.jsonl"), maxEntries = 5)
        repeat(12) { f.write("{\"n\":$it}") }
        assertEquals(5, f.size())
        assertEquals("{\"n\":11}", f.newestFirst().first())
        assertEquals("{\"n\":7}", f.newestFirst().last())
        f.clear()
        assertEquals(0, f.size())
        f.write("{\"n\":99}")
        assertEquals(1, f.size())
    }

    @Test fun throwingSinkIsSwallowedAndCounted() {
        val logcat = mutableListOf<String>()
        val log = DebugLogger({ true }, { throw java.io.IOException("disk full") }, { logcat += it }, runner = { it.run() })
        log.dictation(trace)
        log.dictation(trace)
        assertEquals(2, log.errors.get())
        assertEquals("logcat still gets the line", 2, logcat.size)
    }

    @Test fun throwingEnabledOrLogcatNeverEscapes() {
        DebugLogger({ throw IllegalStateException() }, { }, { }, runner = { it.run() }).dictation(trace)
        val log = DebugLogger({ true }, { }, { throw RuntimeException() }, runner = { it.run() })
        log.dictation(trace)
        assertEquals(1, log.errors.get())
        val rejecting = DebugLogger({ true }, { }, { }, runner = { throw java.util.concurrent.RejectedExecutionException() })
        rejecting.dictation(trace)
        assertEquals(1, rejecting.errors.get())
    }

    @Test fun fullQueueDropsTheOldest() {
        val pending = mutableListOf<Runnable>()
        val written = mutableListOf<String>()
        val log = DebugLogger({ true }, { written += JSONObject(it).getString("text") }, { }, runner = { pending += it }, capacity = 2)
        for (t in listOf("one", "two", "three")) log.dictation(trace.copy(text = t))
        assertEquals(1, log.dropped.get())
        assertEquals("one drain scheduled", 1, pending.size)
        pending.single().run()
        assertEquals(listOf("two", "three"), written)
    }

    @Test fun aBlockedWriterNeverBlocksTheCaller() {
        val release = CountDownLatch(1)
        val wrote = CountDownLatch(3)
        val log = DebugLogger({ true }, { release.await(); wrote.countDown() }, { }, runner = DebugLogger.defaultRunner())
        val t0 = System.nanoTime()
        repeat(3) { log.dictation(trace) }
        val ms = (System.nanoTime() - t0) / 1_000_000
        assertTrue("caller waited $ms ms", ms < 500)
        release.countDown()
        assertTrue(wrote.await(5, TimeUnit.SECONDS))
    }

    @Test fun viewerSummaryShowsTimingsAndText() {
        val s = DebugFormat.summary(DebugFormat.dictation(trace))
        assertTrue(s, s.contains("wait 820 ms"))
        assertTrue(s, s.contains("Hello world."))
        assertTrue(s, s.contains("local (connect_failed)"))
        assertEquals("garbage", DebugFormat.summary("garbage"))
        assertTrue(DebugFormat.summary(DebugFormat.event(DebugEvent(0, "a11y_health", "GHOST"))).contains("a11y_health: GHOST"))
    }

    // ---- D22 fields (same names as apps/flow-windows) ----

    @Test fun dictationLineCarriesStreamingFields() {
        val o = JSONObject(DebugFormat.dictation(trace.copy(waitMs = 120, segments = 3, preAsrMs = 2400, clientMs = 610)))
        assertEquals(120, o.getInt("wait_ms"))
        assertEquals(3, o.getInt("segments"))
        assertEquals(2400, o.getInt("pre_asr_ms"))
        assertEquals(610, o.getInt("client_ms"))
        val none = JSONObject(DebugFormat.dictation(trace))
        for (k in listOf("wait_ms", "segments", "pre_asr_ms", "client_ms")) assertTrue("missing $k", none.has(k) && none.isNull(k))
    }

    @Test fun commitLineHasStreamingTimingsAndNoText() {
        val r = dev.flowclone.core.CommitResult(
            dev.flowclone.core.CommitStatus.RAW_TRANSCRIPT, asrText = "secret words", finalText = "secret words",
            timings = dev.flowclone.core.Timings(audioMs = 30_000, asrMs = 400, totalMs = 520, waitMs = 120, segments = 4, preAsrMs = 2400),
            route = dev.flowclone.core.RouteInfo("remote", clientMs = 610),
        )
        val line = CommitLine.of(r)
        for (part in listOf("wait=120ms", "segments=4", "pre_asr=2400ms", "client=610ms")) assertTrue(line, line.contains(part))
        assertFalse(line.contains("secret"))
        val local = CommitLine.of(r.copy(route = null))
        assertTrue(local, local.contains("client=-"))
    }

    // ---- 2026-10-23: which engine / model produced the line ----

    @Test fun dictationLineCarriesEngineModelAndLanguage() {
        val o = JSONObject(DebugFormat.dictation(trace.copy(engine = "whisper", model = "ggml-base-q5_1.bin", language = "en")))
        assertEquals("whisper", o.getString("engine"))
        assertEquals("ggml-base-q5_1.bin", o.getString("model"))
        assertEquals("en", o.getString("language"))
        val none = JSONObject(DebugFormat.dictation(trace))
        for (k in listOf("engine", "model", "language")) assertTrue("missing $k", none.has(k) && none.isNull(k))
        val line = DebugFormat.dictation(trace)
        assertTrue(line, line.contains("\"fallback\":\"connect_failed\",\"engine\":null,\"model\":null,\"language\":null,\"rtt_ms\""))
    }

    @Test fun engineAndModelPerRoute() {
        val whisper = dev.flowclone.data.LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-base-q5_1.bin")
        val parakeet = dev.flowclone.data.LoadedAsr(dev.flowclone.core.AsrEngine.PARAKEET, "parakeet-int8")
        assertEquals("whisper" to "ggml-base-q5_1.bin", DebugEngine.of("local", mock = false, asr = whisper, serverModel = "x", routeModel = null))
        assertEquals("parakeet" to "parakeet-int8", DebugEngine.of("local", false, parakeet, null, null))
        assertEquals("server" to "parakeet-tdt-0.6b-v2", DebugEngine.of("remote", false, whisper, "parakeet-tdt-0.6b-v2", null))
        assertEquals("server, model not probed yet", "server" to null, DebugEngine.of("remote", false, whisper, null, null))
        assertEquals("cloud" to "whisper-1", DebugEngine.of("cloud", false, whisper, null, "whisper-1"))
        assertEquals("mock" to null, DebugEngine.of("local", true, null, null, null))
        assertEquals("mock" to null, DebugEngine.of(null, true, null, null, null))
        assertEquals("no engine ran", null to null, DebugEngine.of(null, false, whisper, null, null))
    }

    @Test fun viewerSummaryShowsEngineAndModel() {
        val s = DebugFormat.summary(DebugFormat.dictation(trace.copy(engine = "whisper", model = "ggml-base-q5_1.bin", language = "auto")))
        assertTrue(s, s.contains("local (connect_failed) whisper ggml-base-q5_1.bin"))
        assertTrue(s, s.contains("lang auto"))
    }
}
