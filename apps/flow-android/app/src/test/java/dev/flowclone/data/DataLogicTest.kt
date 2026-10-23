package dev.flowclone.data

import dev.flowclone.core.CleanupLevel
import dev.flowclone.core.DictEntry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class RetentionTest {
    private val day = 86_400_000L

    @Test fun keepForeverHasNoCutoff() = assertNull(Retention.KEEP.cutoff(now = 10 * day))

    @Test fun oneDayCutoff() = assertEquals(9 * day, Retention.DAY.cutoff(now = 10 * day))

    @Test fun neverStoresNothing() {
        assertTrue(!Retention.NEVER.stores)
        assertTrue(Retention.KEEP.stores)
    }

    @Test fun parsesStoredNameWithFallback() {
        assertEquals(Retention.DAY, Retention.fromKey("day"))
        assertEquals(Retention.KEEP, Retention.fromKey("garbage"))
    }
}

class DictionaryMergeTest {
    @Test fun importReplacesSameWordCaseInsensitivelyAndKeepsOthers() {
        val existing = listOf(DictEntry("Wispr"), DictEntry("sig", "old", true))
        val incoming = listOf(DictEntry("wispr", "Wispr Flow"), DictEntry("new"))
        val merged = DictionaryMerge.merge(existing, incoming)
        assertEquals(listOf(DictEntry("sig", "old", true), DictEntry("wispr", "Wispr Flow"), DictEntry("new")), merged)
    }

    @Test fun duplicateWordsInImportKeepLast() {
        val merged = DictionaryMerge.merge(emptyList(), listOf(DictEntry("a", "1"), DictEntry("A", "2")))
        assertEquals(listOf(DictEntry("A", "2")), merged)
    }
}

class SettingsTest {
    @Test fun sessionConfigFromSettings() {
        val s = Settings(language = "de", cleanup = CleanupLevel.NONE)
        val dict = listOf(DictEntry("x"))
        val c = s.toSessionConfig(dict)
        assertEquals(listOf("de"), c.languages)
        assertEquals(dict, c.dictionary)
        assertEquals(CleanupLevel.NONE, c.cleanupLevel)
    }

    @Test fun autoLanguageSendsEmptyList() = assertTrue(Settings(language = "auto").toSessionConfig(emptyList()).languages.isEmpty())

    @Test fun capMinutesBecomeSessionCap() = assertEquals(120_000L, Settings(capMinutes = 2).sessionCap().capMs)
}

class FieldContextBuilderTest {
    private val text = "a".repeat(600) + "SEL" + "b".repeat(700)

    @Test fun splitsAroundSelectionAndTruncatesTo500() {
        val c = FieldContextBuilder.build("com.Slack", inputType = 0x1, surrounding = text, selStart = 600, selEnd = 603)
        assertEquals(500, c.before.length)
        assertEquals("SEL", c.selected)
        assertEquals(500, c.after.length)
        assertEquals("work_chat", c.appCategory)
        assertEquals("com.Slack", c.appId)
    }

    @Test fun excludedFieldGetsBlankText() {
        val c = FieldContextBuilder.build("com.foo", inputType = 0x11 /* URI */, surrounding = "secret", selStart = 6, selEnd = 6)
        assertEquals("", c.before)
        assertEquals("com.foo", c.appId)
    }

    @Test fun bankingAppGetsBlankText() {
        val c = FieldContextBuilder.build("com.chase.sig.android", inputType = 0x1, surrounding = "x", selStart = 1, selEnd = 1)
        assertEquals("", c.before)
    }

    @Test fun unknownContextIsEmpty() {
        val c = FieldContextBuilder.build("com.foo", inputType = 0x1, surrounding = null, selStart = -1, selEnd = -1)
        assertEquals("", c.before)
        assertEquals("", c.after)
    }

    @Test fun reversedOutOfRangeSelectionIsNormalized() {
        val c = FieldContextBuilder.build("com.foo", inputType = 0x1, surrounding = "hello", selStart = 9, selEnd = 2)
        assertEquals("he", c.before)
        assertEquals("llo", c.selected)
        assertEquals("", c.after)
    }

    @Test fun handsFreeEndpointPref() {
        assertEquals("D22: 2000 -> 1500", 1500, Settings().handsFreeEndpointMs())
        assertEquals(null, Settings(endpointSilenceMs = 0).handsFreeEndpointMs())
        assertEquals(300, Settings(endpointSilenceMs = 10).handsFreeEndpointMs())
        assertEquals(0, Settings.parseEndpointMs(""))
        assertEquals(0, Settings.parseEndpointMs("0"))
        assertEquals(10_000, Settings.parseEndpointMs("99999"))
        assertEquals(1500, Settings.parseEndpointMs(" 1500 "))
        assertEquals("toSessionConfig leaves it for the controller (hands-free only)", null, Settings().toSessionConfig(emptyList()).endpointSilenceMs)
    }

    /** Review S17: the SQL the history store runs so deleted rows do not linger on disk. */
    @Test fun historyHygieneStatements() {
        assertEquals("PRAGMA secure_delete=ON", HistoryHygiene.ON_OPEN)
        assertEquals("PRAGMA wal_checkpoint(TRUNCATE)", HistoryHygiene.AFTER_DELETE)
        assertEquals("VACUUM", HistoryHygiene.AFTER_CLEAR)
        val src = java.io.File("src/main/java/dev/flowclone/data/Stores.kt").readText()
        assertTrue("every delete path scrubs", Regex("""fun (delete|clear)\(.*scrub\(""").findAll(src).count() == 2)
        assertTrue(src.contains("override fun onConfigure"))
    }

    // ---- D22 (docs/proposals/streaming-asr-shell-api.md §1-2, §5) ----

    @Test fun decodeWhileSpeakingDefaultsOnAndSends800() {
        assertTrue(Settings().decodeWhileSpeaking)
        assertEquals(800, Settings().toSessionConfig(emptyList()).segmentPauseMs)
        assertEquals("off = field omitted (today's behaviour)", null, Settings(decodeWhileSpeaking = false).toSessionConfig(emptyList()).segmentPauseMs)
    }

    @Test fun endpointDefaultMigratesOnlyTheOldDefault() {
        assertEquals("never saved", 1500, Settings.migrateEndpointMs(stored = null, migrated = false))
        assertEquals("still at the old default", 1500, Settings.migrateEndpointMs(stored = 2000, migrated = false))
        assertEquals("user value kept", 3000, Settings.migrateEndpointMs(stored = 3000, migrated = false))
        assertEquals("user turned it off", 0, Settings.migrateEndpointMs(stored = 0, migrated = false))
        assertEquals("2000 chosen after the migration is kept", 2000, Settings.migrateEndpointMs(stored = 2000, migrated = true))
    }
}

/** 2026-10-23: English dictations came out as Bulgarian with `auto` on 2 s clips (findings). */
class LanguageDefaultTest {
    private val multi = LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-base-q5_1.bin")
    private val english = LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-base.en-q5_1.bin")
    private val parakeet = LoadedAsr(dev.flowclone.core.AsrEngine.PARAKEET, "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8")

    @Test fun unsetFollowsThePhoneLanguageOnAMultilingualWhisper() {
        assertEquals("en", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "en", multi))
        assertEquals("de", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "de", multi))
    }

    @Test fun unsetIsAutoWithoutAMultilingualWhisper() {
        assertEquals("English-only whisper", "auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "en", english))
        assertEquals("Parakeet", "auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "en", parakeet))
        assertEquals("no local engine (server / cloud only)", "auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "en", null))
    }

    @Test fun unsetIsAutoWhenWhisperDoesNotKnowThePhoneLanguage() {
        assertEquals("auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "fil", multi))
        assertEquals("auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, null, multi))
        assertEquals("auto", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "", multi))
    }

    @Test fun legacyJavaCodesAreNormalised() {
        assertEquals("he", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "iw", multi))
        assertEquals("id", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "in", multi))
        assertEquals("yi", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "ji", multi))
        assertEquals("whisper's Javanese code", "jw", LanguageDefault.effective(Settings.LANGUAGE_UNSET, "jv", multi))
    }

    @Test fun explicitChoicesAreKept() {
        assertEquals("auto", LanguageDefault.effective("auto", "en", multi))
        assertEquals("fr", LanguageDefault.effective("fr", "en", multi))
        assertEquals("fr", LanguageDefault.effective("fr", "en", english))
    }

    @Test fun multilingualDetectionFromTheFileName() {
        assertTrue(LanguageDefault.isMultilingual(multi))
        assertTrue(LanguageDefault.isMultilingual(LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-large-v3-turbo-q5_0.bin")))
        assertTrue(LanguageDefault.isMultilingual(LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-small.bin")))
        assertTrue(!LanguageDefault.isMultilingual(english))
        assertTrue(!LanguageDefault.isMultilingual(LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-small.en.bin")))
        assertTrue(!LanguageDefault.isMultilingual(LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-tiny.en-q8_0.bin")))
        assertTrue("distil-whisper is English-only", !LanguageDefault.isMultilingual(LoadedAsr(dev.flowclone.core.AsrEngine.WHISPER, "ggml-distil-large-v3.bin")))
        assertTrue(!LanguageDefault.isMultilingual(parakeet))
    }

    @Test fun storedAutoFromBeforeTheChangeBecomesUnsetOnce() {
        // Settings are saved as a whole and "auto" was the first, default entry: a stored "auto"
        // from before is almost always the untouched default (as the D22 endpoint migration).
        assertEquals(Settings.LANGUAGE_UNSET, Settings.migrateLanguage(stored = "auto", migrated = false))
        assertEquals(Settings.LANGUAGE_UNSET, Settings.migrateLanguage(stored = null, migrated = false))
        assertEquals(Settings.LANGUAGE_UNSET, Settings.migrateLanguage(stored = null, migrated = true))
        assertEquals("auto chosen after the change is kept", "auto", Settings.migrateLanguage(stored = "auto", migrated = true))
        assertEquals("en", Settings.migrateLanguage(stored = "en", migrated = false))
    }

    @Test fun sessionConfigUsesTheEffectiveLanguage() {
        assertEquals(listOf("de"), Settings().toSessionConfig(emptyList(), language = "de").languages)
        assertTrue("unset, unresolved → auto", Settings().toSessionConfig(emptyList()).languages.isEmpty())
        assertEquals(listOf("fr"), Settings(language = "fr").toSessionConfig(emptyList()).languages)
    }

    @Test fun label() {
        assertEquals("English (phone language)", LanguageDefault.label(Settings.LANGUAGE_UNSET, "en"))
        assertEquals("Auto-detect (phone language not used: model is English-only or unknown)", LanguageDefault.label(Settings.LANGUAGE_UNSET, "auto"))
        assertEquals("Auto-detect", LanguageDefault.label("auto", "auto"))
        assertEquals("German", LanguageDefault.label("de", "de"))
    }
}
