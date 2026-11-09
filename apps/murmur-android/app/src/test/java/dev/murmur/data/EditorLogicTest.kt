package dev.murmur.data

import dev.murmur.core.CleanupLevel
import dev.murmur.core.CommitStatus
import dev.murmur.core.CoreException
import dev.murmur.core.DictEntry
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.MurmurCoreLoader
import dev.murmur.core.ModelConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LlmSettingsTest {
    @Test fun llmOffMeansNoCleanupWhateverTheLevel() {
        val c = Settings(llmEnabled = false, cleanup = CleanupLevel.MEDIUM).toSessionConfig(emptyList())
        assertEquals(CleanupLevel.NONE, c.cleanupLevel)
        assertTrue(c.rawMode)
    }

    @Test fun llmOnUsesLevelAndDefaultsToLight() {
        assertEquals(CleanupLevel.MEDIUM, Settings(llmEnabled = true, cleanup = CleanupLevel.MEDIUM).toSessionConfig(emptyList()).cleanupLevel)
        val c = Settings(llmEnabled = true, cleanup = CleanupLevel.NONE).toSessionConfig(emptyList())
        assertEquals(CleanupLevel.LIGHT, c.cleanupLevel)
        assertFalse(c.rawMode)
        assertEquals(Settings.ANDROID_LLM_TIMEOUT_MS, c.llmTimeoutMs)
    }

    @Test fun defaultsAreLlmOffPreviewOnShakeOnMinimiseOn() {
        val s = Settings()
        assertFalse(s.llmEnabled)
        assertTrue(s.livePreview && s.shakeToReveal && s.shrinkBubble && s.shrinkToDot)
    }

    @Test fun livePreviewSetsPartialInterval() {
        assertEquals(Settings.PARTIAL_INTERVAL_MS, Settings(livePreview = true).toSessionConfig(emptyList()).partialIntervalMs)
        assertNull(Settings(livePreview = false).toSessionConfig(emptyList()).partialIntervalMs)
    }
}

class ModelFilesTest {
    @Test fun sniffsGgufAndWhisperGgml() {
        assertEquals(ModelFiles.Kind.LLM_GGUF, ModelFiles.kindOf("GGUF\u0003\u0000".toByteArray()))
        assertEquals(ModelFiles.Kind.ASR_GGML, ModelFiles.kindOf(byteArrayOf(0x6c, 0x6d, 0x67, 0x67, 1, 0)))
        assertEquals(ModelFiles.Kind.UNKNOWN, ModelFiles.kindOf("PK\u0003\u0004".toByteArray()))
        assertEquals(ModelFiles.Kind.UNKNOWN, ModelFiles.kindOf(ByteArray(2)))
    }

    @Test fun wrongKindExplainsWhereItBelongs() {
        assertNull(ModelFiles.mismatch(expected = ModelFiles.Kind.ASR_GGML, actual = ModelFiles.Kind.ASR_GGML))
        assertTrue(ModelFiles.mismatch(ModelFiles.Kind.ASR_GGML, ModelFiles.Kind.LLM_GGUF)!!.contains("AI cleanup"))
        assertTrue(ModelFiles.mismatch(ModelFiles.Kind.LLM_GGUF, ModelFiles.Kind.ASR_GGML)!!.contains("speech model"))
        assertNotNull(ModelFiles.mismatch(ModelFiles.Kind.LLM_GGUF, ModelFiles.Kind.UNKNOWN))
    }

    /** Device bug (S25+, ggml-base-q5_1.bin): a valid header made `mismatch` null and the elvis said "Could not read". */
    @Test fun importDecisionAcceptsValidHeader() {
        val ggml = byteArrayOf(0x6c, 0x6d, 0x67, 0x67)
        assertNull(ModelFiles.rejection(ModelFiles.Kind.ASR_GGML, ggml))
        assertNull(ModelFiles.rejection(ModelFiles.Kind.LLM_GGUF, "GGUF".toByteArray()))
    }

    @Test fun importDecisionExplainsWrongKind() {
        assertEquals(
            ModelFiles.mismatch(ModelFiles.Kind.ASR_GGML, ModelFiles.Kind.LLM_GGUF),
            ModelFiles.rejection(ModelFiles.Kind.ASR_GGML, "GGUF".toByteArray()),
        )
        assertTrue(ModelFiles.rejection(ModelFiles.Kind.ASR_GGML, "PK\u0003\u0004".toByteArray())!!.contains("ggml"))
    }

    @Test fun importDecisionUnreadableFile() =
        assertEquals("Could not read the file.", ModelFiles.rejection(ModelFiles.Kind.ASR_GGML, null))

    /** A content provider stream may return fewer bytes per read() than asked (pipes, cloud providers). */
    @Test fun readHeaderLoopsOverShortReads() {
        val oneByteAtATime = object : java.io.InputStream() {
            private val data = byteArrayOf(0x6c, 0x6d, 0x67, 0x67, 1, 2)
            private var i = 0
            override fun read(): Int = if (i < data.size) data[i++].toInt() and 0xff else -1
            override fun read(b: ByteArray, off: Int, len: Int): Int {
                if (len == 0) return 0
                val c = read(); if (c < 0) return -1
                b[off] = c.toByte(); return 1
            }
        }
        val h = ModelFiles.readHeader(oneByteAtATime)
        assertEquals(4, h.size)
        assertEquals(ModelFiles.Kind.ASR_GGML, ModelFiles.kindOf(h))
    }

    @Test fun readHeaderOfTinyFileIsShortAndRejected() {
        val h = ModelFiles.readHeader(java.io.ByteArrayInputStream(byteArrayOf(0x6c, 0x6d)))
        assertEquals(2, h.size)
        assertNotNull(ModelFiles.rejection(ModelFiles.Kind.ASR_GGML, h))
    }

    @Test fun recommendationsAreSmallQwenQ4KM() {
        val r = ModelFiles.LLM_RECOMMENDATIONS
        assertTrue(r.first().file.startsWith("qwen2.5-0.5b-instruct-q4_k_m"))
        assertTrue(r.all { it.file.endsWith(".gguf") && it.url.startsWith("https://huggingface.co/") })
    }
}

class EnginePlanTest {
    private val exists = { _: String -> true }

    @Test fun mockWins() {
        val p = EnginePlan.plan(Settings(mockEngine = true), "/m/asr.bin", "/m/llm.gguf", exists, threads = 4)
        assertTrue(p is EnginePlan.Plan.Mock)
    }

    @Test fun noAsrModelIsMissing() {
        val p = EnginePlan.plan(Settings(), null, null, exists, 4)
        assertTrue(p is EnginePlan.Plan.Missing)
        val q = EnginePlan.plan(Settings(), "/gone.bin", null, { false }, 4)
        assertTrue(q is EnginePlan.Plan.Missing)
    }

    @Test fun llmPathOnlyWhenEnabledAndPresent() {
        val off = EnginePlan.plan(Settings(llmEnabled = false), "/a.bin", "/l.gguf", exists, 4) as EnginePlan.Plan.Native
        assertNull(off.config.llmPath)
        val on = EnginePlan.plan(Settings(llmEnabled = true), "/a.bin", "/l.gguf", exists, 4) as EnginePlan.Plan.Native
        assertEquals("/l.gguf", on.config.llmPath)
        val noFile = EnginePlan.plan(Settings(llmEnabled = true), "/a.bin", "/l.gguf", { it == "/a.bin" }, 4) as EnginePlan.Plan.Native
        assertNull(noFile.config.llmPath)
        assertEquals(EnginePlan.LlmStatus.NoModel, noFile.llm)
    }

    @Test fun llmLoadFailureFallsBackToAsrOnly() {
        val tried = mutableListOf<ModelConfig>()
        val loader = MurmurCoreLoader { cfg ->
            tried += cfg
            if (cfg.llmPath != null) throw CoreException("murmur-llm plugin not found") else FakeMurmurCore("x")
        }
        val plan = EnginePlan.plan(Settings(llmEnabled = true), "/a.bin", "/l.gguf", exists, 4) as EnginePlan.Plan.Native
        val r = EnginePlan.load(plan, loader)
        assertNotNull(r.core)
        assertEquals(2, tried.size)
        assertTrue(r.llm is EnginePlan.LlmStatus.Failed)
        assertTrue((r.llm as EnginePlan.LlmStatus.Failed).message.contains("plugin"))
    }

    @Test fun llmLoadSuccess() {
        val plan = EnginePlan.plan(Settings(llmEnabled = true), "/a.bin", "/l.gguf", exists, 4) as EnginePlan.Plan.Native
        val r = EnginePlan.load(plan) { FakeMurmurCore("x") }
        assertEquals(EnginePlan.LlmStatus.Loaded("l.gguf"), r.llm)
    }

    @Test fun asrFailureIsReported() {
        val plan = EnginePlan.plan(Settings(), "/a.bin", null, exists, 4) as EnginePlan.Plan.Native
        val r = EnginePlan.load(plan) { throw CoreException("bad model") }
        assertNull(r.core)
        assertEquals("bad model", r.error)
    }
}

class DictionaryEditTest {
    private val all = listOf(DictEntry("Tailscale"), DictEntry("brb", "be right back", true), DictEntry("kubectl", "kubectl"))

    @Test fun validation() {
        assertNotNull(DictionaryEdit.validate(all, null, DictEntry(" ")))
        assertNotNull(DictionaryEdit.validate(all, null, DictEntry("x".repeat(61))))
        assertNull(DictionaryEdit.validate(all, null, DictEntry("x".repeat(60))))
        assertNotNull(DictionaryEdit.validate(all, null, DictEntry("sig", null, isSnippet = true)))
        assertNotNull("duplicate", DictionaryEdit.validate(all, null, DictEntry("tailscale")))
        assertNull("editing itself is fine", DictionaryEdit.validate(all, "Tailscale", DictEntry("tailscale", "Tailscale")))
        assertNotNull("rename onto another", DictionaryEdit.validate(all, "Tailscale", DictEntry("BRB")))
    }

    @Test fun applyEditKeepsPositionAndRenames() {
        val out = DictionaryEdit.apply(all, "brb", DictEntry("brb2", "be right back!", true))
        assertEquals(listOf("Tailscale", "brb2", "kubectl"), out.map { it.word })
        assertEquals("be right back!", out[1].replacement)
    }

    @Test fun applyNewAppends() {
        val out = DictionaryEdit.apply(all, null, DictEntry("new"))
        assertEquals(4, out.size)
        assertEquals("new", out.last().word)
    }

    @Test fun filterByTabAndQuery() {
        assertEquals(listOf("kubectl", "Tailscale"), DictionaryEdit.filter(all, "", DictionaryEdit.Tab.WORDS).map { it.word })
        assertEquals(listOf("brb"), DictionaryEdit.filter(all, "", DictionaryEdit.Tab.SNIPPETS).map { it.word })
        assertEquals(listOf("brb"), DictionaryEdit.filter(all, "RIGHT", DictionaryEdit.Tab.ALL).map { it.word })
        assertEquals(listOf("Tailscale"), DictionaryEdit.filter(all, "tai", DictionaryEdit.Tab.ALL).map { it.word })
    }

    @Test fun normalizesWhitespaceAndSnippetWithoutTextIsAWord() {
        assertEquals(DictEntry("hi there", null, false), DictionaryEdit.normalize("  hi there ", "  ", true))
        assertEquals(DictEntry("sig", "Best,\nMe", true), DictionaryEdit.normalize("sig", "Best,\nMe", true))
    }
}

class HistoryViewTest {
    private fun row(id: Long, asr: String, fin: String, status: CommitStatus = CommitStatus.FORMATTED) =
        HistoryRow(id, id, "com.x", status.name, asr, fin, true, 1000)

    private val rows = listOf(
        row(1, "um hello world", "Hello world."),
        row(2, "meet at noon", "Meet at noon.", CommitStatus.RAW_TRANSCRIPT),
        row(3, "", "", CommitStatus.EMPTY),
    )

    @Test fun searchMatchesRawOrFormatted() {
        assertEquals(listOf(1L), HistoryView.filter(rows, "UM ").map { it.id })
        assertEquals(listOf(2L), HistoryView.filter(rows, "noon.").map { it.id })
        assertEquals(3, HistoryView.filter(rows, " ").size)
    }

    @Test fun rawVersusFormattedText() {
        assertEquals("um hello world", HistoryView.text(rows[0], raw = true))
        assertEquals("Hello world.", HistoryView.text(rows[0], raw = false))
        assertEquals("(no speech)", HistoryView.text(rows[2], raw = false))
    }

    @Test fun labelsAndDiff() {
        assertEquals("AI cleaned", HistoryView.label(rows[0]))
        assertEquals("Transcript", HistoryView.label(rows[1]))
        assertEquals("No speech", HistoryView.label(rows[2]))
        assertTrue(HistoryView.hasRawVariant(rows[0]))
        assertFalse(HistoryView.hasRawVariant(row(9, "Same", "Same")))
    }
}
