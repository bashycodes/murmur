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
}
