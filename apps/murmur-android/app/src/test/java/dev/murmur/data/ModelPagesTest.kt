package dev.murmur.data

import dev.murmur.data.ModelPages.Screen
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Model import screens (speech / Parakeet / AI cleanup): status card, recommendations, imported rows. */
class ModelPagesTest {
    @Test fun everyScreenRecommendsAtLeastOneModelWithAnHttpsLink() {
        for (s in Screen.entries) {
            val r = ModelPages.recommended(s)
            assertTrue("$s", r.isNotEmpty())
            // H5: Hebrew models are four files: the link is the pinned revision's folder on Hugging Face.
            if (s == Screen.HEBREW) for (m in r) assertTrue(m.url, m.url.startsWith("https://huggingface.co/") && Regex("/tree/[0-9a-f]{40}").containsMatchIn(m.url))
            else for (m in r) assertTrue(m.url, m.url.startsWith("https://") && m.url.endsWith(m.file.substringAfterLast('/')) || m.url.endsWith("${m.file}.tar.bz2"))
        }
    }

    /** The AI cleanup list is the one murmur-core's llm_e2e measures (ModelFiles.LLM_RECOMMENDATIONS). */
    @Test fun aiCleanupRecommendationsAreTheMeasuredPair() {
        assertEquals(ModelFiles.LLM_RECOMMENDATIONS.map { it.file to it.url }, ModelPages.recommended(Screen.LLM).map { it.file to it.url })
    }

    @Test fun recommendationSubtitleHasSizeSpeedAndQuality() {
        val base = ModelPages.recommended(Screen.WHISPER).first()
        assertEquals("ggml-base.en-q5_1.bin", base.file)
        assertEquals("~60 MB · Fast · Good quality · English only", ModelPages.subtitle(base))
        val qwen = ModelPages.recommended(Screen.LLM).first()
        assertTrue(ModelPages.subtitle(qwen), ModelPages.subtitle(qwen).startsWith("~490 MB · Fastest · "))
    }

    @Test fun sizes() {
        assertEquals("61 MB", ModelPages.size(61_000_000))
        assertEquals("1.1 GB", ModelPages.size(1_120_000_000))
        assertEquals("0 MB", ModelPages.size(0))
    }

    @Test fun statusCard() {
        assertEquals("None imported", ModelPages.active(null))
        assertEquals("ggml-base.en-q5_1.bin", ModelPages.active("ggml-base.en-q5_1.bin"))
        assertEquals("whisper.cpp (ggml)", ModelPages.engine(Screen.WHISPER))
        assertEquals("Parakeet-TDT via sherpa-onnx", ModelPages.engine(Screen.PARAKEET))
        assertEquals("llama.cpp (GGUF)", ModelPages.engine(Screen.LLM))
    }

    @Test fun importedRow() {
        assertEquals("61 MB · In use", ModelPages.imported(61_000_000, active = true))
        assertEquals("61 MB", ModelPages.imported(61_000_000, active = false))
    }

    @Test fun importActionLabel() {
        assertEquals("Import model file…", ModelPages.importLabel(Screen.WHISPER, busy = false))
        assertEquals("Import model folder…", ModelPages.importLabel(Screen.PARAKEET, busy = false))
        assertEquals("Importing…", ModelPages.importLabel(Screen.LLM, busy = true))
    }

    /** The owner's v1.1.2 setup: the copy claimed "only this phone", "nothing from the internet" next to Hugging Face links. */
    @Test fun introsMatchWhatTheAppDoes() {
        for (server in listOf(false, true)) {
            val llm = ModelPages.intro(Screen.LLM, server)
            for (stale in listOf("runs only on this phone", "off until you turn it on")) assertTrue(llm, stale !in llm)
            assertTrue(llm, "\"This phone\"" in llm && "GGUF" in llm)
            assertEquals(llm, server, "your own server" in llm)
            val whisper = ModelPages.intro(Screen.WHISPER, server)
            assertTrue(whisper, "Nothing ever comes from the internet" !in whisper)
            assertEquals(whisper, server, "your own server" in whisper)
        }
    }

    private fun phone(gib: Double, free: Long? = null) = ModelFit.Machine((gib * (1L shl 30)).toLong(), free)

    /** D117: the table marks one file per screen Recommended and says whether each fits; the old static "flagship phones" notes are gone. */
    @Test fun recommendedBadgeAndFitFollowThePhone() {
        val big = phone(11.2, 50_000_000_000)
        val llm = ModelPages.recommended(Screen.LLM)
        assertEquals(listOf(false, true), llm.map { ModelPages.rowFit(Screen.LLM, it, big)!!.recommended })
        val small = phone(5.5, 50_000_000_000)
        val qwen15 = ModelPages.rowFit(Screen.LLM, llm[1], small)!!
        assertEquals(ModelPages.RowFit(recommended = false, fits = false, line = "Doesn't fit: needs 8 GB RAM"), qwen15)
        assertEquals("Fits this phone", ModelPages.rowFit(Screen.PARAKEET, ModelPages.recommended(Screen.PARAKEET)[0], small)!!.line)
        val tight = ModelPages.rowFit(Screen.PARAKEET, ModelPages.recommended(Screen.PARAKEET)[0], phone(2.8, 300_000_000))!!
        assertEquals("Doesn't fit: needs 761 MB free, needs 4 GB RAM", tight.line)
        // Below 4 GB whisper base is the pick (only the file for the language); tiny only below 2 GB;
        // from 4 GB Parakeet is, so the whisper page badges nothing.
        val whisper = ModelPages.recommended(Screen.WHISPER)
        assertEquals(listOf(true, false, false, false), whisper.map { ModelPages.rowFit(Screen.WHISPER, it, phone(2.8))!!.recommended })
        assertEquals(listOf(false, true, false, false), whisper.map { ModelPages.rowFit(Screen.WHISPER, it, phone(2.8), english = false)!!.recommended })
        assertEquals(listOf(false, false, true, false), whisper.map { ModelPages.rowFit(Screen.WHISPER, it, phone(1.5))!!.recommended })
        assertEquals(listOf(false, false, false, false), whisper.map { ModelPages.rowFit(Screen.WHISPER, it, big)!!.recommended })
        val hebrew = ModelPages.recommended(Screen.HEBREW).single { ModelPages.rowFit(Screen.HEBREW, it, big)!!.recommended }
        assertTrue(hebrew.file, hebrew.file.startsWith("zipformer-he"))
        for (s in Screen.entries) for (r in ModelPages.recommended(s)) {
            assertTrue(r.file, r.model != null)
            for (stale in listOf("flagship", "older phones", "free ram")) assertTrue(r.note, stale !in r.note.lowercase())
        }
    }

    @Test fun englishWhisperFollowsTheLanguageSetting() {
        assertTrue(ModelPages.englishWhisper("en", "de"))
        assertEquals(false, ModelPages.englishWhisper("de", "en"))
        assertTrue(ModelPages.englishWhisper(Settings.LANGUAGE_AUTO, "en"))
        assertEquals(false, ModelPages.englishWhisper(Settings.LANGUAGE_KEYBOARD, "iw"))
        assertEquals(false, ModelPages.englishWhisper("", "fr"))
    }

    @Test fun installedModelsSkipTheDiskCheck() {
        val full = phone(11.2, 100_000_000)
        val hebrew = ModelPages.recommended(Screen.HEBREW)
        val parakeetHe = hebrew.single { it.model == ModelFit.Model.PARAKEET_HE }
        // Hebrew folders keep no recognisable name: the imported kind decides.
        val kinds = setOf(ModelPages.hebrewModel(HebrewModels.Kind.PARAKEET))
        assertTrue(ModelPages.installed(Screen.HEBREW, parakeetHe, listOf("parakeet-he"), kinds))
        assertEquals(false, ModelPages.installed(Screen.HEBREW, parakeetHe, listOf(parakeetHe.file), emptySet()))
        val fit = ModelPages.rowFit(Screen.HEBREW, parakeetHe, full, ModelPages.installed(Screen.HEBREW, parakeetHe, listOf("parakeet-he"), kinds))!!
        assertEquals("Fits this phone", fit.line)
        assertEquals("Doesn't fit: needs 773 MB free", ModelPages.rowFit(Screen.HEBREW, parakeetHe, full)!!.line)
        val parakeet = ModelPages.recommended(Screen.PARAKEET)[0]
        assertTrue(ModelPages.installed(Screen.PARAKEET, parakeet, listOf("my-renamed-folder")))
        val base = ModelPages.recommended(Screen.WHISPER)[0]
        assertTrue(ModelPages.installed(Screen.WHISPER, base, listOf(base.file)))
        assertEquals(false, ModelPages.installed(Screen.WHISPER, base, listOf("ggml-tiny.en-q5_1.bin")))
    }
}
