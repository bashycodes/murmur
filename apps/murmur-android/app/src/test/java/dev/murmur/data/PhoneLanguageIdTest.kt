package dev.murmur.data

import dev.murmur.core.AsrEngine
import dev.murmur.core.CoreJson
import dev.murmur.core.LanguageModel
import dev.murmur.core.ModelConfig
import dev.murmur.core.RouterJson
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/** D119: Automatic identifies the spoken language on the phone with the language-ID model. */
class PhoneLanguageIdTest {
    private val exists = { _: String -> true }
    private val he = LanguageModel(AsrEngine.SHERPA_ONLINE, "/f/hebrew/zipformer-he")
    private val lid = "/f/hebrew/ggml-tiny-q5_1.bin"
    private val multi = LoadedAsr(AsrEngine.WHISPER, "ggml-base.bin")
    private val installed = setOf("he")
    private val auto = Settings.LANGUAGE_AUTO

    private fun resolve(keyboard: String? = null, heard: String? = null, offPhone: Boolean = false, lid: Boolean = true) =
        LanguageDefault.resolve(auto, "en", multi, installed, keyboard = keyboard, offPhone = offPhone, lid = lid, heard = heard)

    @Test fun automaticOnThePhoneDetectsAndLoadsTheLanguageLastHeard() {
        assertEquals(LanguageDefault.Resolved("auto", LanguageDefault.Source.LID, load = "he"), resolve(heard = "he"))
        assertEquals("English heard: the default model", "auto", resolve(keyboard = "he", heard = "en").load)
        assertTrue("the session detects", resolve(heard = "he").sessionLanguages.isEmpty())
        assertEquals("lid", resolve().source.wire)
    }

    @Test fun beforeAnythingIsHeardTheKeyboardIsTheFirstGuess() {
        assertEquals("he", resolve(keyboard = "he").load)
        assertEquals("auto", resolve(keyboard = "en").load)
        assertEquals("auto", resolve().load)
        assertEquals("auto", resolve(keyboard = "he").language)
    }

    @Test fun withoutTheModelAutomaticFollowsTheKeyboardAsBefore() {
        assertEquals(LanguageDefault.Resolved("he", LanguageDefault.Source.KEYBOARD), resolve(keyboard = "he", heard = "en", lid = false))
        assertEquals(LanguageDefault.Resolved("auto", LanguageDefault.Source.FIXED), resolve(keyboard = "en", lid = false))
    }

    @Test fun offThePhoneAutomaticSendsNoLanguage() {
        // D113 unchanged: the server or the cloud identifies the language; the phone's model is not used.
        val r = resolve(keyboard = "he", heard = "he", offPhone = true)
        assertEquals(LanguageDefault.Resolved("auto", LanguageDefault.Source.FIXED), r)
        assertTrue(r.sessionLanguages.isEmpty())
    }

    @Test fun otherSettingsIgnoreLanguageId() {
        assertEquals(
            LanguageDefault.Resolved("he", LanguageDefault.Source.KEYBOARD),
            LanguageDefault.resolve(Settings.LANGUAGE_KEYBOARD, "en", multi, installed, keyboard = "he", lid = true, heard = "en"),
        )
        assertEquals("en", LanguageDefault.resolve("en", "en", multi, installed, lid = true, heard = "he").load)
    }

    @Test fun theDefaultModelLoadsWithTheHebrewRouteAndTheLanguageIdModel() {
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "auto", hebrew = he, lidPath = lid) as EnginePlan.Plan.Native
        assertEquals(mapOf("he" to he), p.config.languages)
        assertEquals(EnginePlan.DEFAULT_ROUTE, p.config.loadLanguage)
        assertEquals(lid, p.config.lidPath)
        assertEquals("the default model loads", LanguageModel(AsrEngine.WHISPER, "/m/base.bin"), p.config.routed)
        assertFalse(p.config.loadsRoute)
        assertEquals(EnginePlan.FirstChoice.PARAKEET, EnginePlan.FirstChoice.of(p.config))
        assertFalse(EnginePlan.loadsBoth(p.config))
        assertNull("the fallback has no language ID", p.whisperFallback?.lidPath)
        val json = JSONObject(CoreJson.encode(p.config))
        assertEquals(lid, json.getString("lid_path"))
        assertEquals("en", json.getString("load_language"))
    }

    @Test fun theHebrewModelLoadsWithTheLanguageIdModel() {
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "he", hebrew = he, lidPath = lid) as EnginePlan.Plan.Native
        assertEquals("he", p.config.loadLanguage)
        assertEquals(he, p.config.routed)
        assertEquals(lid, p.config.lidPath)
    }

    @Test fun bothLoadedKeepsLanguageIdSoTheCoreRoutesItself() {
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "auto", hebrew = he, keepBoth = true, lidPath = lid) as EnginePlan.Plan.Native
        assertNull(p.config.loadLanguage)
        assertTrue(EnginePlan.loadsBoth(p.config))
        assertEquals(lid, p.config.lidPath)
    }

    @Test fun noLanguageIdModelLeavesThePlanAsBefore() {
        for (lang in listOf("auto", "en", "he")) {
            assertEquals(
                lang,
                EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = lang, hebrew = he),
                EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = lang, hebrew = he, lidPath = null),
            )
        }
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "auto", hebrew = he) as EnginePlan.Plan.Native
        assertFalse(JSONObject(CoreJson.encode(p.config)).has("lid_path"))
    }

    @Test fun languageIdNeedsTwoModels() {
        val noHebrew = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "auto", hebrew = null, lidPath = lid) as EnginePlan.Plan.Native
        assertEquals(ModelConfig(asrPath = "/m/base.bin", threads = 4), noHebrew.config)
        val hebrewOnly = EnginePlan.plan(Settings(), null, null, exists, 4, language = "he", hebrew = he, lidPath = lid) as EnginePlan.Plan.Native
        assertNull(hebrewOnly.config.lidPath)
    }

    @Test fun theWantedLanguageIsDecodedFromPollAndCommit() {
        assertEquals("he", RouterJson.decodePoll("""{"partial":null,"language_wanted":"he"}""").languageWanted)
        assertNull(RouterJson.decodePoll("""{"partial":"hi"}""").languageWanted)
        assertEquals("en", CoreJson.decodeCommit("""{"status":"RAW_TRANSCRIPT","language_wanted":"en"}""").languageWanted)
        assertNull(CoreJson.decodeCommit("""{"status":"RAW_TRANSCRIPT"}""").languageWanted)
    }

    @Test fun theLanguageIdFileIsPinnedAndFoundBySize() {
        assertEquals("ggml-tiny-q5_1.bin", HebrewModels.LID.name)
        assertEquals(32_152_673L, HebrewModels.LID.size)
        val d = kotlin.io.path.createTempDirectory("lid").toFile()
        try {
            assertNull(HebrewModels.lidPath(d))
            File(d, HebrewModels.LID.name).writeBytes(ByteArray(10))
            assertNull("a truncated file is not used", HebrewModels.lidPath(d))
        } finally { d.deleteRecursively() }
    }

    @Test fun anImportMayCarryTheLanguageIdFile() {
        val ok = HebrewModels.Facts(HebrewModels.LID.size, HebrewModels.LID.sha256)
        assertEquals(HebrewModels.Extras(lid = true, model = false), HebrewModels.extras(mapOf(HebrewModels.LID.name to ok)))
        assertEquals(HebrewModels.Extras(lid = false, model = true), HebrewModels.extras(mapOf("tokens.txt" to HebrewModels.Facts(3))))
        val bad = HebrewModels.extras(mapOf(HebrewModels.LID.name to HebrewModels.Facts(5)))
        assertFalse(bad.lid)
        assertTrue(bad.bad!!.contains("5 bytes"))
        assertTrue(HebrewModels.extras(mapOf(HebrewModels.LID.name to HebrewModels.Facts(HebrewModels.LID.size, "00"))).bad!!.contains("SHA-256"))
    }
}
