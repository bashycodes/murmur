package dev.murmur.debug

import dev.murmur.core.AsrEngine
import dev.murmur.core.CleanupLevel
import dev.murmur.core.CoreException
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.MurmurCore
import dev.murmur.core.MurmurCoreLoader
import dev.murmur.core.ModelConfig
import dev.murmur.core.ParakeetDecoding
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class BenchRunnerTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun tone(ms: Int) = ShortArray(ms * 16) { i -> ((i % 32) * 600 - 9600).toShort() }

    /** files/ with the given model paths present, and a bundle with 2 en + 1 he clips and 3 cases. */
    private fun setup(vararg models: String): Pair<File, File> {
        val files = tmp.newFolder("files")
        for (m in models) File(files, m).apply { parentFile!!.mkdirs(); if (m.endsWith(".bin") || m.endsWith(".gguf")) writeText("x") else mkdirs() }
        val inDir = tmp.newFolder("in")
        File(inDir, "audio").mkdirs()
        for (id in listOf("e1", "e2", "h1")) File(inDir, "audio/$id.wav").writeBytes(Wav.encode(tone(300)))
        File(inDir, "manifest.json").writeText(
            """{"stt":[{"set":"en","id":"e1","wav":"audio/e1.wav"},{"set":"en","id":"e2","wav":"audio/e2.wav"},""" +
                """{"set":"he","id":"h1","wav":"audio/h1.wav"}],"cleanup":"cases.jsonl"}""",
        )
        File(inDir, "cases.jsonl").writeText(
            (1..3).joinToString("\n") { """{"id":"c$it","app_id":"com.example.chat","app_category":"personal_chat","input":"hello number $it"}""" } + "\n",
        )
        return files to inDir
    }

    private class Loader : MurmurCoreLoader {
        val loaded = mutableListOf<ModelConfig>()
        val cores = mutableListOf<FakeMurmurCore>()
        override fun loadModels(cfg: ModelConfig): MurmurCore {
            loaded += cfg
            if (cfg.asrPath.endsWith("parakeet-v3-he")) throw CoreException("model load failed: out of memory")
            return FakeMurmurCore("hello world").also { cores += it }
        }
    }

    private fun lines(f: File) = f.readLines().filter { it.isNotBlank() }.map { JSONObject(it) }

    @Test fun runsSttAndCleanupAndWritesRawRows() {
        val (files, inDir) = setup("parakeet/parakeet-unified-en-0.6b-int8", "hebrew/zipformer-he", "hebrew/parakeet-v3-he", "llm/qwen2.5-1.5b-instruct-q4_k_m.gguf")
        val out = File(tmp.root, "out")
        val loader = Loader()
        val requests = mutableListOf<JSONObject>()
        val meta = BenchRunner(
            BenchArgs(threads = 4), files, inDir, out, loader,
            clean = { _, req -> requests += JSONObject(req); """{"mode":"x","llm_raw":"Hello.","llm_ms":12,"output":"Hello."}""" },
        ).run()

        // English: parakeet ran both en clips (+ an untimed warm-up), whisper-base was missing.
        val en = lines(File(out, "stt-parakeet-en.jsonl"))
        assertEquals(listOf("e1", "e2"), en.map { it.getString("id") })
        assertEquals("hello world", en[0].getString("asr_text"))
        assertEquals(300L, en[0].getLong("audio_ms"))
        assertTrue(en[0].has("commit_ms") && en[0].has("asr_ms"))
        assertEquals("RAW_TRANSCRIPT", en[0].getString("status"))
        assertFalse(File(out, "stt-whisper-base.jsonl").exists())
        // Hebrew: zipformer ran h1 only; parakeet-he failed to load and is reported, not fatal.
        assertEquals(listOf("h1"), lines(File(out, "stt-zipformer-he.jsonl")).map { it.getString("id") })
        val models = meta.getJSONArray("models")
        val byKey = (0 until models.length()).associate { models.getJSONObject(it).let { m -> m.getString("key") to m } }
        assertTrue(byKey.getValue("whisper-base").getString("skipped").startsWith("missing"))
        assertTrue(byKey.getValue("parakeet-he").getString("skipped").contains("out of memory"))
        assertFalse(byKey.getValue("qwen0.5b").optString("skipped").isEmpty())
        // Every loaded core was unloaded.
        assertTrue(loader.cores.all { it.unloaded })

        // Hebrew config as the app builds it: a `he` entry loaded alone.
        val he = loader.loaded.first { it.asrPath.endsWith("zipformer-he") }
        assertEquals("he", he.loadLanguage)
        assertEquals(AsrEngine.SHERPA_ONLINE, he.languages.getValue("he").asrEngine)
        val pk = loader.loaded.first { it.asrPath.endsWith("parakeet-unified-en-0.6b-int8") }
        assertEquals(ParakeetDecoding.GREEDY, pk.parakeetDecoding)
        assertEquals(4, pk.threads)
        // LLM handle: mock ASR + the GGUF.
        val llm = loader.loaded.first { it.llmPath != null }
        assertTrue(llm.llmPath!!.endsWith("qwen2.5-1.5b-instruct-q4_k_m.gguf"))
        assertEquals("bench", llm.mockText)

        // Cleanup: one warm-up + 3 cases per mode; rows carry the id and a null error.
        for (mode in listOf("prod", "v4c")) {
            val rows = lines(File(out, "cleanup-qwen1.5b-$mode.jsonl"))
            assertEquals(listOf("c1", "c2", "c3"), rows.map { it.getString("id") })
            assertTrue(rows.all { it.isNull("error") && it.getString("llm_raw") == "Hello." })
        }
        assertEquals(8, requests.size)
        val r = requests.first { it.getString("input") == "hello number 1" && it.getString("mode") == "prod" }
        assertEquals("personal_chat", r.getJSONObject("context").getString("app_category"))
        assertEquals(3000, r.getJSONObject("session").getInt("llm_timeout_ms"))
        assertEquals("light", r.getJSONObject("session").getString("cleanup_level"))
        val v = requests.first { it.getString("mode") == "v4c" && it.getString("input") != "Warm up the cache." }
        assertEquals(30000, v.getJSONObject("session").getInt("llm_timeout_ms"))

        val status = JSONObject(File(out, "status.json").readText())
        assertEquals("done", status.getString("state"))
        assertEquals(status.getInt("total"), status.getInt("done"))
        assertTrue(File(out, "meta.json").isFile)
    }

    @Test fun limitAndSuiteNarrowTheRun() {
        val (files, inDir) = setup("parakeet/parakeet-unified-en-0.6b-int8", "llm/qwen2.5-1.5b-instruct-q4_k_m.gguf")
        val out = File(tmp.root, "out")
        var calls = 0
        BenchRunner(BenchArgs.parse(mapOf("suite" to "cleanup", "limit" to "2", "modes" to "v4c")::get), files, inDir, out, Loader(), { _, _ -> calls++; "{}" }).run()
        assertFalse(File(out, "stt-parakeet-en.jsonl").exists())
        assertEquals(2, lines(File(out, "cleanup-qwen1.5b-v4c.jsonl")).size)
        assertFalse(File(out, "cleanup-qwen1.5b-prod.jsonl").exists())
        assertEquals(3, calls)
    }

    @Test fun realtimePaceSleepsUntilEachFrameWasSpoken() {
        val (files, inDir) = setup("parakeet/parakeet-unified-en-0.6b-int8")
        var now = 0L
        val sleeps = mutableListOf<Long>()
        BenchRunner(
            BenchArgs(suite = "stt", stt = listOf("parakeet-en"), limit = 1, realtime = true), files, inDir, File(tmp.root, "out"), Loader(),
            { _, _ -> "{}" }, clockMs = { now }, sleepMs = { sleeps += it; now += it },
        ).run()
        // 300 ms of audio = 3 frames, each pushed once its 100 ms had passed (warm-up + 1 clip).
        assertEquals(listOf(100L, 100L, 100L, 100L, 100L, 100L), sleeps)
    }

    @Test fun argsParseAndRejectNonsense() {
        val a = BenchArgs.parse(mapOf("stt" to "parakeet-en, zipformer-he", "llm" to "qwen1.5b", "threads" to "6", "pace" to "realtime", "segment" to "800", "run" to "r-1")::get)
        assertEquals(listOf("parakeet-en", "zipformer-he"), a.stt)
        assertEquals(6, a.threads)
        assertTrue(a.realtime)
        assertEquals(800, a.segmentPauseMs)
        assertEquals("r-1", a.run)
        val d = BenchArgs.parse { null }
        assertEquals(BenchPlan.STT.size, d.stt.size)
        assertNull(d.segmentPauseMs)
        for (bad in listOf("suite" to "x", "stt" to "nope", "llm" to "gpt", "modes" to "ai", "pace" to "slow", "threads" to "many", "run" to "../x")) {
            try {
                BenchArgs.parse(mapOf(bad)::get)
                fail("accepted $bad")
            } catch (_: IllegalArgumentException) {
            }
        }
    }

    @Test fun sessionsMatchTheApp() {
        val s = BenchPlan.sttSession("he")
        assertEquals(listOf("he"), s.languages)
        assertEquals(CleanupLevel.NONE, s.cleanupLevel)
        assertTrue(s.vad && s.hallucinationGuard && s.spokenCleanup)
        assertEquals(CleanupLevel.LIGHT, BenchPlan.cleanupSession("prod").cleanupLevel)
        assertEquals(4, BenchPlan.appThreads(8))
        assertEquals(1, BenchPlan.appThreads(1))
    }
}
