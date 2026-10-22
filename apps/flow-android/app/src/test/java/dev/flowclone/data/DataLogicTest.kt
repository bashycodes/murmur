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
        assertEquals(2000, Settings().handsFreeEndpointMs())
        assertEquals(null, Settings(endpointSilenceMs = 0).handsFreeEndpointMs())
        assertEquals(300, Settings(endpointSilenceMs = 10).handsFreeEndpointMs())
        assertEquals(0, Settings.parseEndpointMs(""))
        assertEquals(0, Settings.parseEndpointMs("0"))
        assertEquals(10_000, Settings.parseEndpointMs("99999"))
        assertEquals(1500, Settings.parseEndpointMs(" 1500 "))
        assertEquals("toSessionConfig leaves it for the controller (hands-free only)", null, Settings().toSessionConfig(emptyList()).endpointSilenceMs)
    }
}
