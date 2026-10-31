package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CoreException
import dev.flowclone.core.CoreJson
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.LanguageModel
import dev.flowclone.core.ModelConfig
import dev.flowclone.core.ParakeetDecoding
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** H5: Hebrew model slot: file checks, import, routing and the load config. */
class HebrewModelsTest {
    private fun facts(kind: HebrewModels.Kind, hashes: Boolean = true) =
        kind.pins.associate { it.name to HebrewModels.Facts(it.size, if (hashes) it.sha256 else null) }

    @Test fun pinsMatchTheFindingsDoc() {
        assertEquals(72_316_960L, HebrewModels.Kind.ZIPFORMER.totalBytes)
        assertEquals(672_825_290L, HebrewModels.Kind.PARAKEET.totalBytes)
        assertEquals(AsrEngine.SHERPA_ONLINE, HebrewModels.Kind.ZIPFORMER.engine)
        assertEquals(AsrEngine.PARAKEET, HebrewModels.Kind.PARAKEET.engine)
    }

    @Test fun identifiesEitherModelBySizeAndHash() {
        assertEquals(HebrewModels.Check.Ok(HebrewModels.Kind.ZIPFORMER), HebrewModels.identify(facts(HebrewModels.Kind.ZIPFORMER)))
        assertEquals(HebrewModels.Check.Ok(HebrewModels.Kind.PARAKEET), HebrewModels.identify(facts(HebrewModels.Kind.PARAKEET)))
        // Sizes alone (the folder pre-check, `kindOf`) are enough to tell them apart.
        assertEquals(HebrewModels.Check.Ok(HebrewModels.Kind.PARAKEET), HebrewModels.identify(facts(HebrewModels.Kind.PARAKEET, hashes = false)))
        // Extra files (bpe.model, fp32 exports) are ignored.
        val extra = facts(HebrewModels.Kind.ZIPFORMER) + ("bpe.model" to HebrewModels.Facts(1))
        assertTrue(HebrewModels.identify(extra) is HebrewModels.Check.Ok)
    }

    @Test fun missingFilesAreNamed() {
        val c = HebrewModels.identify(facts(HebrewModels.Kind.ZIPFORMER) - "tokens.txt" - "joiner.int8.onnx")
        assertTrue(c is HebrewModels.Check.Bad)
        val m = (c as HebrewModels.Check.Bad).message
        assertTrue(m, m.contains("joiner.int8.onnx") && m.contains("tokens.txt"))
    }

    @Test fun wrongSizeOrHashIsRejectedWithTheFile() {
        val size = facts(HebrewModels.Kind.ZIPFORMER) + ("decoder.int8.onnx" to HebrewModels.Facts(5))
        val m = (HebrewModels.identify(size) as HebrewModels.Check.Bad).message
        assertTrue(m, m.contains("decoder.int8.onnx is 5 bytes, expected 1049105") && m.contains("Zipformer"))
        val hash = facts(HebrewModels.Kind.PARAKEET) + ("tokens.txt" to HebrewModels.Facts(107590, "00"))
        val h = (HebrewModels.identify(hash) as HebrewModels.Check.Bad).message
        assertTrue(h, h.contains("tokens.txt") && h.contains("SHA-256") && h.contains("Parakeet"))
        // The files of one model mixed with another's: no model matches.
        val mixed = facts(HebrewModels.Kind.ZIPFORMER) + ("encoder.int8.onnx" to facts(HebrewModels.Kind.PARAKEET).getValue("encoder.int8.onnx"))
        assertTrue(HebrewModels.identify(mixed) is HebrewModels.Check.Bad)
    }

    @Test fun kindOfAnImportedDirectory() {
        val d = kotlin.io.path.createTempDirectory("he").toFile()
        try {
            assertNull(HebrewModels.kindOf(d))
            for (p in HebrewModels.Kind.ZIPFORMER.pins) java.io.RandomAccessFile(java.io.File(d, p.name), "rw").use { it.setLength(p.size) }
            assertEquals(HebrewModels.Kind.ZIPFORMER, HebrewModels.kindOf(d))
            java.io.File(d, "tokens.txt").delete()
            assertNull(HebrewModels.kindOf(d))
        } finally { d.deleteRecursively() }
    }

    @Test fun aCustomServerModelIsKnownByItsEngineFile() {
        val d = kotlin.io.path.createTempDirectory("hecustom").toFile()
        try {
            for (n in listOf("encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt")) java.io.File(d, n).writeText("x")
            assertNull("no engine file: not a model", HebrewModels.installed(d))
            java.io.File(d, HebrewModels.ENGINE_FILE).writeText("""{"engine":"parakeet","title":"My Hebrew"}""")
            assertEquals(HebrewModels.Installed(AsrEngine.PARAKEET, "My Hebrew"), HebrewModels.installed(d))
            assertEquals(LanguageModel(AsrEngine.PARAKEET, "/h/c", ParakeetDecoding.GREEDY), HebrewModels.languageModel("/h/c", HebrewModels.installed(d)!!))
            java.io.File(d, HebrewModels.ENGINE_FILE).writeText("""{"engine":"sherpa_online"}""")
            assertEquals(AsrEngine.SHERPA_ONLINE, HebrewModels.installed(d)!!.engine)
            java.io.File(d, HebrewModels.ENGINE_FILE).writeText("""{"engine":"whisper"}""")
            assertNull(HebrewModels.installed(d))
            java.io.File(d, HebrewModels.ENGINE_FILE).writeText("""{"engine":"parakeet"}""")
            java.io.File(d, "joiner.onnx").delete()
            assertNull("incomplete folder", HebrewModels.installed(d))
        } finally { d.deleteRecursively() }
    }

    @Test fun aPinnedModelNeedsNoEngineFile() {
        val d = kotlin.io.path.createTempDirectory("hepinned").toFile()
        try {
            for (p in HebrewModels.Kind.PARAKEET.pins) java.io.RandomAccessFile(java.io.File(d, p.name), "rw").use { it.setLength(p.size) }
            assertEquals(HebrewModels.Installed(AsrEngine.PARAKEET, HebrewModels.Kind.PARAKEET.label, HebrewModels.Kind.PARAKEET), HebrewModels.installed(d))
        } finally { d.deleteRecursively() }
    }

    @Test fun zipImportKeepsOnlyModelFilesAtAnyDepthAndHashesThem() {
        val bytes = ByteArrayOutputStream().also { b ->
            ZipOutputStream(b).use { z ->
                for ((name, data) in listOf("m/sherpa-onnx/tokens.txt" to "abc", "../../evil/encoder.int8.onnx" to "e", "m/README.md" to "x", "m/" to "")) {
                    z.putNextEntry(ZipEntry(name)); z.write(data.toByteArray()); z.closeEntry()
                }
            }
        }.toByteArray()
        val d = kotlin.io.path.createTempDirectory("hezip").toFile()
        try {
            var seen = 0L
            val f = HebrewImport.extractZip(ByteArrayInputStream(bytes), d) { seen += it }
            assertEquals(setOf("tokens.txt", "encoder.int8.onnx"), f.keys)
            assertEquals(HebrewModels.Facts(3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"), f["tokens.txt"])
            assertEquals(4L, seen)
            // Written inside the staging directory only, under the plain file name.
            assertEquals(setOf("tokens.txt", "encoder.int8.onnx"), d.list()!!.toSet())
            assertTrue(HebrewModels.identify(f) is HebrewModels.Check.Bad)
        } finally { d.deleteRecursively() }
    }

    @Test fun routeOnlyForHebrewWithAModel() {
        assertEquals("he", HebrewModels.route("he", installed = true))
        assertEquals("he", HebrewModels.route("he-IL", installed = true))
        assertEquals("he", HebrewModels.route("iw", installed = true))
        assertNull(HebrewModels.route("he", installed = false))
        assertNull(HebrewModels.route("en", installed = true))
        assertNull(HebrewModels.route("auto", installed = true))
        assertNull(HebrewModels.route(null, installed = true))
    }

    @Test fun languageModelDecodesParakeetGreedyOnly() {
        assertEquals(LanguageModel(AsrEngine.SHERPA_ONLINE, "/h/z", ParakeetDecoding.AUTO), HebrewModels.languageModel("/h/z", HebrewModels.Kind.ZIPFORMER))
        assertEquals(LanguageModel(AsrEngine.PARAKEET, "/h/p", ParakeetDecoding.GREEDY), HebrewModels.languageModel("/h/p", HebrewModels.Kind.PARAKEET))
    }
}

class HebrewRoutingTest {
    private val exists = { _: String -> true }
    private val he = LanguageModel(AsrEngine.SHERPA_ONLINE, "/f/hebrew/zipformer-he")

    @Test fun hebrewLoadsOnlyTheHebrewModelWithWhisperAsFallback() {
        val p = EnginePlan.plan(Settings(language = "he"), "/m/base.bin", null, exists, 4, language = "he", hebrew = he) as EnginePlan.Plan.Native
        assertEquals("he", p.config.loadLanguage)
        assertEquals(mapOf("he" to he), p.config.languages)
        assertEquals(he, p.config.routed)
        assertEquals("/m/base.bin", p.whisperFallback?.asrPath)
        assertNull(p.whisperFallback?.loadLanguage)
        assertEquals(LoadedAsr(AsrEngine.SHERPA_ONLINE, "zipformer-he"), EnginePlan.loadedAsrOf(p.config))
        assertEquals(AsrEngine.SHERPA_ONLINE, EnginePlan.selectedEngine(Settings(), p))
    }

    @Test fun otherLanguagesLoadTheDefaultEngineUnchanged() {
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "en", hebrew = he) as EnginePlan.Plan.Native
        assertEquals(ModelConfig(asrPath = "/m/base.bin", threads = 4), p.config)
        val pk = EnginePlan.plan(
            Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/pk", isModelDir = { true },
            language = "auto", hebrew = he,
        ) as EnginePlan.Plan.Native
        assertEquals(AsrEngine.PARAKEET, pk.config.asrEngine)
        assertTrue(pk.config.languages.isEmpty())
        assertEquals(AsrEngine.PARAKEET, EnginePlan.selectedEngine(Settings(asrEngine = AsrEngine.PARAKEET), pk))
    }

    @Test fun hebrewWithoutModelUsesWhisperAndHebrewModelAloneIsEnough() {
        val w = EnginePlan.plan(Settings(language = "he"), "/m/base.bin", null, exists, 4, language = "he", hebrew = null) as EnginePlan.Plan.Native
        assertNull(w.config.loadLanguage)
        // No whisper / Parakeet imported: Hebrew still dictates, English says what is missing.
        val only = EnginePlan.plan(Settings(language = "he"), null, null, exists, 4, language = "he", hebrew = he) as EnginePlan.Plan.Native
        assertEquals(he, only.config.routed)
        assertNull(only.whisperFallback)
        assertTrue(EnginePlan.plan(Settings(), null, null, exists, 4, language = "en", hebrew = he) is EnginePlan.Plan.Missing)
    }

    @Test fun hebrewFailureFallsBackToWhisperAndSaysWhy() {
        val p = EnginePlan.plan(Settings(language = "he"), "/m/base.bin", null, exists, 4, language = "he", hebrew = he) as EnginePlan.Plan.Native
        val r = EnginePlan.load(p) { cfg -> if (cfg.loadLanguage != null) throw CoreException("model load failed: oom") else FakeFlowCore("x") }
        assertEquals(LoadedAsr(AsrEngine.WHISPER, "base.bin"), r.asr)
        assertTrue(r.description, r.description.contains("(Hebrew model failed: model load failed: oom)"))
        val m = EngineMismatch.message(EnginePlan.selectedEngine(Settings(language = "he"), p), p, r.asr, r.description)
        assertEquals("selected sherpa_online, loaded whisper base.bin: model load failed: oom", m)
        val ok = EnginePlan.load(p) { FakeFlowCore("x") }
        assertEquals("Hebrew zipformer-he", ok.description)
        assertNull(EngineMismatch.message(EnginePlan.selectedEngine(Settings(), p), p, ok.asr, ok.description))
    }

    @Test fun onDemandLoaderGetsTheHebrewConfigFirst() {
        val p = EnginePlan.plan(Settings(language = "he"), "/m/base.bin", null, exists, 4, language = "he", hebrew = he) as EnginePlan.Plan.Native
        val a = EnginePlan.attempts(p)
        assertEquals(listOf("he", null), a.map { it.loadLanguage })
    }

    @Test fun modelConfigJsonCarriesLanguagesAndLoadLanguage() {
        val p = EnginePlan.plan(Settings(language = "he"), "/m/base.bin", null, exists, 4, language = "he",
            hebrew = LanguageModel(AsrEngine.PARAKEET, "/f/hebrew/parakeet-v3-he", ParakeetDecoding.GREEDY)) as EnginePlan.Plan.Native
        val j = JSONObject(CoreJson.encode(p.config))
        assertEquals("he", j.getString("load_language"))
        val e = j.getJSONObject("languages").getJSONObject("he")
        assertEquals("parakeet", e.getString("asr_backend"))
        assertEquals("/f/hebrew/parakeet-v3-he", e.getString("asr_path"))
        assertEquals("greedy", e.getString("parakeet_decoding"))
        val z = JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m", languages = mapOf("he" to he), loadLanguage = "he")))
        assertEquals("sherpa_online", z.getJSONObject("languages").getJSONObject("he").getString("asr_backend"))
        assertFalse(z.getJSONObject("languages").getJSONObject("he").has("parakeet_decoding"))
        // Old configs are byte-identical: neither key appears.
        val old = JSONObject(CoreJson.encode(ModelConfig(asrPath = "/m")))
        assertFalse(old.has("languages") || old.has("load_language"))
    }

    @Test fun phoneLanguageHebrewSelectsTheHebrewModel() {
        assertEquals("he", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "iw", null, setOf("he")))
        assertEquals("auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "iw", null))
        assertEquals("en", LanguageDefault.effective("en", "iw", null, setOf("he")))
    }

    @Test fun languagePageNote() {
        assertNull(SettingsPages.hebrewNote("en", HebrewModels.Kind.ZIPFORMER.label, local = true))
        assertNull(SettingsPages.hebrewNote("he", null, local = false))
        assertTrue(SettingsPages.hebrewNote("he", null, local = true)!!.contains("Import a Hebrew model"))
        assertTrue(SettingsPages.hebrewNote("he", HebrewModels.Kind.ZIPFORMER.label, local = true)!!.contains("reloads"))
    }
}
