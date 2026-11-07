package dev.flowclone.data

import dev.flowclone.core.DictEntry
import dev.flowclone.core.ParakeetDecoding
import dev.flowclone.core.ParakeetDecoding.AUTO
import dev.flowclone.core.ParakeetDecoding.BEAM
import dev.flowclone.core.ParakeetDecoding.GREEDY
import org.junit.Assert.assertEquals
import org.junit.Test

/** D68: the phone's Parakeet search follows the dictionary unless Developer settings override it. */
class ParakeetDecodingTest {
    private fun pick(override: ParakeetDecoding?, vararg d: DictEntry) = EnginePlan.parakeetDecoding(override, d.toList())

    @Test fun anEmptyDictionaryKeepsTodaysGreedyPath() {
        assertEquals(GREEDY, pick(null))
        assertEquals("snippets give no hotwords", GREEDY, pick(null, DictEntry("addr", "1 Example Road", isSnippet = true)))
        assertEquals("a blank word gives no hotwords", GREEDY, pick(null, DictEntry("  ")))
    }

    @Test fun aDictionaryWordSwitchesToBeamSearch() {
        assertEquals(BEAM, pick(null, DictEntry("Zorblax")))
        assertEquals("a replacement's target is the hotword", BEAM, pick(null, DictEntry("zorb lax", "Zorblax")))
    }

    @Test fun aDeveloperOverrideWins() {
        assertEquals(GREEDY, pick(GREEDY, DictEntry("Zorblax")))
        assertEquals(AUTO, pick(AUTO))
        assertEquals(BEAM, pick(BEAM))
    }

    @Test fun hebrewStaysGreedyUnlessOverridden() {
        assertEquals(GREEDY, HebrewModels.decoding(null))
        assertEquals(AUTO, HebrewModels.decoding(AUTO))
    }
}
