package dev.flowclone.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Runs only with `-Pflow.jniLibDir=…` (tools/probes/host_native/run.sh): loads a host x86_64
 * build of the REAL crates/flow-ffi (not the C stub) through the shipped [NativeFlowCore]
 * binding and [CoreJson] payloads. With `-Pflow.model` + `-Pflow.wav` it also runs whisper on
 * the WAV, pushing 100 ms frames exactly like MicRecorder, and writes timings to
 * `build/reports/host-native/timings.txt`.
 */
class NativeFlowCoreHostTest {
    private fun requireLib() = assumeTrue("host flow-ffi not built", System.getProperty("flow.jniReal") == "1")

    @Test fun mockEngineRoundTripsThroughRealCore() {
        requireLib()
        val lib = NativeFlowCore.loadLibrary()
        assertTrue(lib.toString(), lib is NativeFlowCore.LibraryState.Loaded)
        assertTrue((lib as NativeFlowCore.LibraryState.Loaded).version.startsWith("flow-ffi "))
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = "", mockText = "hello btw", threads = 2))
        core.startSession(
            SessionConfig(languages = listOf("en"), dictionary = listOf(DictEntry("btw", "by the way"))),
        )
        assertTrue(core.pushAudio(sine(16_000), 16_000))
        core.setContext(FieldContext(appId = "com.whatsapp", appCategory = "personal_chat"))
        val r = core.commit()
        assertEquals(r.toString(), CommitStatus.RAW_TRANSCRIPT, r.status)
        assertTrue(r.finalText, r.finalText.contains("by the way"))
        assertEquals(1000L, r.timings.audioMs)
        assertEquals(listOf(LearnCandidate("whisper", "Wispr")), core.learnEdit("ask whisper now", "ask Wispr now"))
        // Bad session JSON can't happen from CoreJson, but a core-side rejection must surface as CoreException.
        core.cancel()
        assertFalse(core.pushAudio(ShortArray(10), 10))
        core.unload()
        assertEquals(CommitStatus.ERROR, core.commit().status)
    }

    /**
     * D17: the shell's real engine assembly (EngineRoute → nativeLoadModels → nativeOpenRouter)
     * against the host library: per flavour toggles, the session goes through the router and
     * the result carries `route`; both-off and bad configs come back as typed errors.
     * With `-Pflow.server=host:port[/token]` (a running flow-server) the server route is used.
     */
    @Test fun routerRoundTripThroughTheShellsAssembly() {
        requireLib()
        val caps = NativeFlowCore.capabilities()!!
        assertEquals(1, caps.proto)
        val loader = FlowCoreLoader { NativeFlowCore.loadModels(it.copy(mockText = "hello btw")) }
        val plan = dev.flowclone.data.EnginePlan.Plan.Native(ModelConfig(asrPath = "", threads = 2), dev.flowclone.data.EnginePlan.LlmStatus.Off)
        val server = System.getProperty("flow.server")?.takeIf { it.isNotBlank() }
        val dev0 = dev.flowclone.data.DevSettings(allowServer = server != null, allowClient = true)
        val dev1 = if (server == null) dev0 else dev0.copy(
            host = server.substringBefore('/').substringBeforeLast(':'), port = server.substringBefore('/').substringAfterLast(':').toInt(),
            token = server.substringAfter('/', ""),
        )
        val routed = dev.flowclone.data.EngineRoute.plan(plan, dev1, dev.flowclone.data.BuildCaps(true, caps.remote)) as dev.flowclone.data.EngineRoute.Plan.Routed
        val a = dev.flowclone.data.EngineRoute.assemble(routed, loader, NativeFlowCore.routerOpener)
        val r = a.core as NativeFlowCore
        assertTrue(r.isRouter)
        r.startSession(SessionConfig(languages = listOf("en"), dictionary = listOf(DictEntry("btw", "by the way"))))
        assertTrue(r.pushAudio(sine(16_000), 16_000))
        val res = r.commit()
        assertEquals(res.toString(), CommitStatus.RAW_TRANSCRIPT, res.status)
        assertEquals(listOf(PasteSegment.Text("hello by the way ")), res.pastePlan())
        assertEquals(if (server != null && caps.remote) "remote" else "local", res.route!!.engine)
        assertEquals(res.route, r.routeStatus().last)
        // Settings change in place, then a bad one is refused with its kind.
        r.updateRouter(dev1.copy(routeOrder = dev.flowclone.data.DevSettings.orderFor(dev.flowclone.data.DevSettings.Prefer.CLIENT)).toRouterConfigJson().toString())
        val bad = runCatching { r.updateRouter("""{"server":{"port":0}}""") }.exceptionOrNull() as CoreException
        assertEquals("bad_config", bad.kind)
        r.resetCircuit()
        r.unload()
        // Both routes off: a typed error before any session.
        val off = runCatching { NativeFlowCore.routerOpener.open("""{"allow_server":false,"allow_client":false}""", null) }.exceptionOrNull() as CoreException
        assertEquals("no_engine_enabled", off.kind)
        assertTrue(RouterErrors.userMessage(off.kind, off.message!!).startsWith("Dictation disabled"))
        // Test connection to a closed port: never throws.
        val h = NativeFlowCore.probeServer("""{"host":"127.0.0.1","port":9,"connect_timeout_ms":300}""")
        assertFalse(h.ok)
        assertEquals(if (caps.remote) "connect_failed" else "server_not_configured", h.error)
        if (server != null) {
            val ok = NativeFlowCore.probeServer(dev1.toServerConfigJson().toString())
            assertTrue(ok.toString(), ok.ok)
            println("HOST-ROUTER probe: ${DevText.probe(ok)}")
        }
    }

    @Test fun missingModelSurfacesCoreError() {
        requireLib()
        val e = runCatching { NativeFlowCore.loadModels(ModelConfig(asrPath = "/nonexistent/ggml.bin")) }.exceptionOrNull()
        assertTrue(e.toString(), e is CoreException && !e.message.isNullOrBlank())
    }

    @Test fun whisperTranscribesWavThroughKotlinBinding() {
        requireLib()
        val model = System.getProperty("flow.model").orEmpty()
        val wav = System.getProperty("flow.wav").orEmpty()
        assumeTrue("no model/wav", File(model).isFile && File(wav).isFile)
        val pcm = readWav16kMono(File(wav))
        val threads = System.getProperty("flow.threads")?.toIntOrNull() ?: 4

        val t0 = System.nanoTime()
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = model, threads = threads))
        val loadMs = (System.nanoTime() - t0) / 1_000_000
        try {
            core.startSession(
                SessionConfig(languages = listOf("en"), dictionary = listOf(DictEntry("Kennedy", null))),
            )
            core.setContext(FieldContext(appId = "com.google.android.keep", appCategory = "document"))
            var pushMaxUs = 0L
            val frame = ShortArray(1600)
            var off = 0
            while (off < pcm.size) {
                val n = minOf(1600, pcm.size - off)
                System.arraycopy(pcm, off, frame, 0, n)
                val p0 = System.nanoTime()
                assertTrue("push rejected at $off", core.pushAudio(frame, n))
                pushMaxUs = maxOf(pushMaxUs, (System.nanoTime() - p0) / 1000)
                off += n
            }
            val c0 = System.nanoTime()
            val r = core.commit()
            val commitMs = (System.nanoTime() - c0) / 1_000_000
            val report = "model=${File(model).name} threads=$threads load_ms=$loadMs commit_wall_ms=$commitMs " +
                "push_max_us=$pushMaxUs core_timings=${r.timings} status=${r.status}\ntext=${r.finalText}\n"
            println(report)
            File(System.getProperty("flow.reportDir") ?: "build/reports/host-native").apply { mkdirs() }
                .resolve("timings.txt").appendText(report)
            assertTrue(r.toString(), r.status == CommitStatus.RAW_TRANSCRIPT || r.status == CommitStatus.FORMATTED)
            assertTrue(r.finalText, r.finalText.lowercase().contains("what your country can do for you"))
            assertEquals(pcm.size / 16L, r.timings.audioMs)
            assertEquals(0L, r.timings.droppedSamples)
        } finally {
            core.unload()
        }
    }

    // ---- LLM cleanup through the real binding (flow-ffi `llm` + the flow-llm plugin) ----

    private fun llm(): Pair<String, String>? {
        val model = System.getProperty("flow.llmModel").orEmpty()
        val lib = System.getProperty("flow.llmLib").orEmpty()
        return if (File(model).isFile && File(lib).isFile) model to lib else null
    }

    /** The shell's own settings path: LLM on, light cleanup (the Android default when enabled). */
    private fun llmSession(budgetMs: Long) =
        dev.flowclone.data.Settings(language = "en", llmEnabled = true).toSessionConfig(emptyList()).copy(llmTimeoutMs = budgetMs)

    private fun report(line: String) {
        println(line)
        File(System.getProperty("flow.reportDir") ?: "build/reports/host-native").apply { mkdirs() }.resolve("timings.txt").appendText(line + "\n")
    }

    @Test fun llmCleansTranscriptThroughKotlinBinding() {
        requireLib()
        val (model, lib) = llm() ?: return assumeTrue("no LLM model/plugin", false)
        val raw = "so um i think we should meet tomorrow at noon and uh bring the slides"
        val t0 = System.nanoTime()
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = "", mockText = raw, llmPath = model, llmLib = lib, threads = 4))
        val loadMs = (System.nanoTime() - t0) / 1_000_000
        try {
            core.startSession(llmSession(budgetMs = 60_000))
            core.setContext(FieldContext(appId = "com.google.android.gm", appCategory = "email"))
            assertTrue(core.pushAudio(sine(16_000), 16_000))
            val r = core.commit()
            report("llm model=${File(model).name} load_ms=$loadMs llm_ms=${r.timings.llmMs} status=${r.status} fallback=${r.fallbackReason}\nraw=$raw\nllm=${r.finalText}")
            assertEquals(r.toString(), CommitStatus.FORMATTED, r.status)
            assertTrue(r.timings.llmMs > 0)
            assertTrue(r.finalText, r.finalText.contains("tomorrow", ignoreCase = true) && r.finalText.contains("slides"))
        } finally {
            core.unload()
        }
    }

    @Test fun llmOffInSettingsNeverCallsTheModel() {
        requireLib()
        val (model, lib) = llm() ?: return assumeTrue("no LLM model/plugin", false)
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = "", mockText = "um hello there", llmPath = model, llmLib = lib, threads = 4))
        try {
            core.startSession(dev.flowclone.data.Settings(language = "en", llmEnabled = false).toSessionConfig(emptyList()))
            core.pushAudio(sine(16_000), 16_000)
            val r = core.commit()
            assertEquals(CommitStatus.RAW_TRANSCRIPT, r.status)
            assertEquals(0L, r.timings.llmMs)
        } finally {
            core.unload()
        }
    }

    @Test fun missingPluginFallsBackToAsrOnlyThroughEnginePlan() {
        requireLib()
        val model = llm()?.first ?: return assumeTrue("no LLM model", false)
        val plan = dev.flowclone.data.EnginePlan.Plan.Native(
            ModelConfig(asrPath = "", mockText = "hello", llmPath = model, llmLib = "/nonexistent/libflow_llm.so", threads = 2),
            dev.flowclone.data.EnginePlan.LlmStatus.Loaded(File(model).name),
        )
        val r = dev.flowclone.data.EnginePlan.load(plan, NativeFlowCore)
        assertTrue(r.toString(), r.core != null && r.llm is dev.flowclone.data.EnginePlan.LlmStatus.Failed)
        report("llm fallback: ${(r.llm as dev.flowclone.data.EnginePlan.LlmStatus.Failed).message}")
        r.core!!.unload()
    }

    @Test fun whisperThenLlmInOneJvmProcess() {
        requireLib()
        val (llmModel, lib) = llm() ?: return assumeTrue("no LLM model/plugin", false)
        val model = System.getProperty("flow.model").orEmpty()
        val wav = System.getProperty("flow.wav").orEmpty()
        assumeTrue("no model/wav", File(model).isFile && File(wav).isFile)
        val pcm = readWav16kMono(File(wav))
        val core = NativeFlowCore.loadModels(ModelConfig(asrPath = model, llmPath = llmModel, llmLib = lib, threads = 4))
        try {
            core.startSession(llmSession(budgetMs = 60_000))
            core.setContext(FieldContext(appId = "com.google.android.keep", appCategory = "document"))
            val frame = ShortArray(1600)
            var off = 0
            while (off < pcm.size) {
                val n = minOf(1600, pcm.size - off)
                System.arraycopy(pcm, off, frame, 0, n)
                core.pushAudio(frame, n); off += n
            }
            val r = core.commit()
            report("whisper+llm asr=${File(model).name} llm=${File(llmModel).name} asr_ms=${r.timings.asrMs} llm_ms=${r.timings.llmMs} status=${r.status} fallback=${r.fallbackReason}\ntext=${r.finalText}")
            assertTrue(r.toString(), r.status == CommitStatus.FORMATTED || r.status == CommitStatus.RAW_TRANSCRIPT)
            assertTrue(r.toString(), r.fallbackReason == null || r.fallbackReason == "llm_timeout")
            assertTrue(r.finalText, r.finalText.lowercase().contains("country"))
        } finally {
            core.unload()
        }
    }

    /**
     * Review C1 on the real library: pushAudio (AudioRecord thread), poll/partial (timer), commit
     * (worker) and unload (Settings save / onTrimMemory) race on the same handle. Before the lease,
     * unload freed the box under a running call (use-after-free → SIGSEGV / garbage); now every call
     * either runs on a live handle or sees "unloaded", and the native free happens exactly once.
     */
    @Test fun unloadRacingPushPollCommitNeverFreesUnderACall() {
        requireLib()
        repeat(40) { round ->
            val core = NativeFlowCore.loadModels(ModelConfig(asrPath = "", mockText = "hello btw", threads = 2)) as NativeFlowCore
            core.startSession(SessionConfig(languages = listOf("en")))
            val stop = java.util.concurrent.atomic.AtomicBoolean(false)
            val errors = java.util.concurrent.ConcurrentLinkedQueue<Throwable>()
            fun loop(body: () -> Unit) = kotlin.concurrent.thread {
                try { while (!stop.get()) body() } catch (t: Throwable) { errors += t }
            }
            val frame = sine(1600)
            val threads = listOf(
                loop { core.pushAudio(frame, frame.size) },
                loop { core.partial() },
                loop {
                    runCatching { core.startSession(SessionConfig(languages = listOf("en"))) }
                        .onFailure { if (it !is CoreException) throw it }
                    core.pushAudio(frame, frame.size)
                    val r = core.commit()
                    check(r.status != CommitStatus.ERROR || r.error in setOf("unloaded", "cancelled") || r.error!!.contains("session")) { r.toString() }
                },
                loop { core.cancel() },
            )
            Thread.sleep(5L + round % 7)
            core.unload()
            Thread.sleep(5)
            stop.set(true)
            threads.forEach { it.join(10_000) }
            assertTrue(errors.joinToString { it.toString() }, errors.isEmpty())
            assertTrue("native engine freed", core.awaitFreed(10_000))
            assertFalse(core.isLoaded)
            assertEquals(CommitStatus.ERROR, core.commit().status)
            assertFalse(core.pushAudio(frame, frame.size))
            assertEquals(null, core.partial())
        }
    }

    private fun sine(n: Int) = ShortArray(n) { (Math.sin(it * 0.05) * 8000).toInt().toShort() }

    /** Minimal RIFF reader: PCM16, 16 kHz, mono only (what MicRecorder produces). */
    private fun readWav16kMono(f: File): ShortArray {
        val b = ByteBuffer.wrap(f.readBytes()).order(ByteOrder.LITTLE_ENDIAN)
        require(String(f.readBytes(), 0, 4, Charsets.US_ASCII) == "RIFF")
        var pos = 12
        var data: ShortArray? = null
        while (pos + 8 <= b.limit()) {
            val id = String(ByteArray(4) { b.get(pos + it) }, Charsets.US_ASCII)
            val len = b.getInt(pos + 4)
            if (id == "fmt ") {
                require(b.getShort(pos + 8).toInt() == 1 && b.getShort(pos + 10).toInt() == 1 &&
                    b.getInt(pos + 12) == 16_000 && b.getShort(pos + 22).toInt() == 16) { "need PCM16 16k mono" }
            } else if (id == "data") {
                data = ShortArray(len / 2) { b.getShort(pos + 8 + it * 2) }
            }
            pos += 8 + len + (len and 1)
        }
        return requireNotNull(data) { "no data chunk" }
    }
}
