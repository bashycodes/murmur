package dev.flowclone.data

import dev.flowclone.core.AsrEngine
import dev.flowclone.core.CoreException
import dev.flowclone.core.CoreJson
import dev.flowclone.core.FakeFlowCore
import dev.flowclone.core.LanguageModel
import dev.flowclone.core.ModelConfig
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D54 "Keep both languages loaded": the load-set decision and the plan it gives. */
class KeepBothLanguagesTest {
    private val exists = { _: String -> true }
    private val he = LanguageModel(AsrEngine.SHERPA_ONLINE, "/f/hebrew/zipformer-he")

    @Test fun offByDefault() {
        assertFalse(Settings().keepBothLanguages)
    }

    private fun set(lang: String?, hebrew: Boolean, default: Boolean, keepBoth: Boolean) = LoadSet.of(lang, hebrew, keepBoth) { default }

    @Test fun loadSetIsBothOnlyWithTheSettingAndTwoModels() {
        for (lang in listOf("he", "en", "auto", null)) {
            assertEquals(lang.toString(), LoadSet.Both, set(lang, hebrew = true, default = true, keepBoth = true))
        }
        // Off: the one-model route as before (D33 decision 4).
        assertEquals(LoadSet.Language("he"), set("he", hebrew = true, default = true, keepBoth = false))
        assertEquals(LoadSet.Default, set("en", hebrew = true, default = true, keepBoth = false))
        // Only one language model installed: no effect.
        assertEquals(LoadSet.Language("he"), set("he", hebrew = true, default = false, keepBoth = true))
        assertEquals(LoadSet.Default, set("he", hebrew = false, default = true, keepBoth = true))
    }

    @Test fun theDefaultModelCheckRunsOnlyWithTheSettingOn() {
        LoadSet.of("he", hebrewInstalled = true, keepBoth = false) { throw AssertionError("file system touched while off") }
    }

    @Test fun aKeyboardSwitchKeepsTheSameLoadSetSoNothingReloads() {
        val before = set("en", hebrew = true, default = true, keepBoth = true)
        val after = set("he", hebrew = true, default = true, keepBoth = true)
        assertEquals(before, after)
        // Toggling the setting changes it: the next dictation reloads once.
        assertTrue(after != set("he", hebrew = true, default = true, keepBoth = false))
    }

    @Test fun onLoadsWhisperAndHebrewTogetherWithWhisperAloneAsFallback() {
        for (lang in listOf("he", "en")) {
            val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = lang, hebrew = he, keepBoth = true) as EnginePlan.Plan.Native
            assertNull("no load_language: the core loads every language model", p.config.loadLanguage)
            assertEquals(mapOf("he" to he), p.config.languages)
            assertEquals("/m/base.bin", p.config.asrPath)
            assertTrue(EnginePlan.loadsBoth(p.config))
            assertEquals(ModelConfig(asrPath = "/m/base.bin", threads = 4), p.whisperFallback)
        }
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "en", hebrew = he, keepBoth = true) as EnginePlan.Plan.Native
        assertFalse(JSONObject(CoreJson.encode(p.config)).has("load_language"))
        assertEquals("base.bin + Hebrew zipformer-he", EnginePlan.load(p) { FakeFlowCore("x") }.description)
    }

    @Test fun onWithParakeetKeepsParakeetAsTheDefault() {
        val p = EnginePlan.plan(
            Settings(asrEngine = AsrEngine.PARAKEET), "/m/base.bin", null, exists, 4, parakeetPath = "/pk", isModelDir = { true },
            language = "he", hebrew = he, keepBoth = true,
        ) as EnginePlan.Plan.Native
        assertEquals(AsrEngine.PARAKEET, p.config.asrEngine)
        assertEquals(mapOf("he" to he), p.config.languages)
        assertNull(p.config.loadLanguage)
        assertEquals(AsrEngine.PARAKEET, EnginePlan.selectedEngine(Settings(asrEngine = AsrEngine.PARAKEET), p))
        assertEquals("Parakeet pk + Hebrew zipformer-he", EnginePlan.load(p) { FakeFlowCore("x") }.description)
    }

    @Test fun bothFailingTogetherFallsBackToWhisperAndSaysWhy() {
        val p = EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "he", hebrew = he, keepBoth = true) as EnginePlan.Plan.Native
        val r = EnginePlan.load(p) { cfg -> if (cfg.languages.isNotEmpty()) throw CoreException("model load failed: oom") else FakeFlowCore("x") }
        assertEquals(LoadedAsr(AsrEngine.WHISPER, "base.bin"), r.asr)
        assertTrue(r.description, r.description.contains("(Both languages failed: model load failed: oom)"))
        // EngineMismatch reads the same label back.
        val m = EngineMismatch.message(AsrEngine.PARAKEET, p, r.asr, r.description)
        assertEquals("selected parakeet, loaded whisper base.bin: model load failed: oom", m)
    }

    @Test fun offOrOneModelLeavesThePlanUnchanged() {
        fun plan(lang: String, keepBoth: Boolean, asr: String? = "/m/base.bin") =
            EnginePlan.plan(Settings(), asr, null, exists, 4, language = lang, hebrew = he, keepBoth = keepBoth)
        assertEquals(ModelConfig(asrPath = "/m/base.bin", threads = 4), (plan("en", keepBoth = false) as EnginePlan.Plan.Native).config)
        assertEquals("he", (plan("he", keepBoth = false) as EnginePlan.Plan.Native).config.loadLanguage)
        // Only the Hebrew model: on behaves as off.
        assertEquals(plan("he", keepBoth = false, asr = null), plan("he", keepBoth = true, asr = null))
        // No Hebrew model: on behaves as off.
        assertEquals(
            EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "en", hebrew = null),
            EnginePlan.plan(Settings(), "/m/base.bin", null, exists, 4, language = "en", hebrew = null, keepBoth = true),
        )
    }

    @Test fun defaultInstalledMatchesWhatThePlanCanLoad() {
        assertTrue(EnginePlan.defaultInstalled(Settings(), "/m/base.bin", exists, null) { false })
        assertFalse(EnginePlan.defaultInstalled(Settings(), null, exists, "/pk") { true })
        assertTrue(EnginePlan.defaultInstalled(Settings(asrEngine = AsrEngine.PARAKEET), null, exists, "/pk") { true })
        assertFalse(EnginePlan.defaultInstalled(Settings(), "/m/gone.bin", { false }, null) { false })
    }

    @Test fun summarySaysWhenTheToggleDoesNothing() {
        assertTrue(SettingsSummary.keepBothLanguages(hebrewInstalled = true, defaultInstalled = true).contains("never waits"))
        for ((h, d) in listOf(true to false, false to true, false to false)) {
            assertTrue(SettingsSummary.keepBothLanguages(h, d).startsWith("No effect"))
        }
    }
}
