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
}
