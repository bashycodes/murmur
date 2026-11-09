package dev.flowclone.learn

import dev.flowclone.core.CoreJson
import dev.flowclone.core.DictEntry
import dev.flowclone.core.LearnCandidate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** D68 name learning from a History correction (made-up names). */
class HistoryCorrectionTest {
    private val calls = mutableListOf<Pair<String, String>>()

    /** Stands in for the core's `learn_edit`: the candidates it would return. */
    private fun core(vararg c: LearnCandidate): (String, String) -> List<LearnCandidate> = { p, e -> calls += p to e; c.toList() }

    @Test fun aNameIsOfferedAndEveryCandidateIsSuggested() {
        val l = HistoryCorrection.learn(
            "ask  gina about the cubernetes job", "ask Jenna about the kubernetes job", Suggestions(), emptyList(), "com.chat", 5,
            core(LearnCandidate("gina", "Jenna", looksLikeName = true), LearnCandidate("cubernetes", "kubernetes")),
        )
        assertEquals("the core sees whitespace-normalised text", listOf("ask gina about the cubernetes job" to "ask Jenna about the kubernetes job"), calls)
        assertEquals(listOf("Jenna"), l.names.map { it.word })
        assertEquals("gina", l.names.single().heard)
        assertEquals(listOf("kubernetes", "Jenna"), l.next.pending.map { it.word })
    }

    @Test fun aNameAlreadyInTheDictionaryOrDismissedIsNotOffered() {
        val dict = listOf(DictEntry("Jenna"))
        val known = HistoryCorrection.learn("ask gina", "ask Jenna", Suggestions(), dict, "a", 1, core(LearnCandidate("gina", "Jenna", looksLikeName = true)))
        assertTrue(known.names.isEmpty())
        val dismissed = Suggestions().dismiss("jenna")
        val d = HistoryCorrection.learn("ask gina", "ask Jenna", dismissed, emptyList(), "a", 1, core(LearnCandidate("gina", "Jenna", looksLikeName = true)))
        assertTrue(d.names.isEmpty())
    }

    @Test fun aRepeatedNameIsOfferedAgainWithItsCount() {
        val first = HistoryCorrection.learn("ask gina", "ask Jenna", Suggestions(), emptyList(), "a", 1, core(LearnCandidate("gina", "Jenna", looksLikeName = true))).next
        val again = HistoryCorrection.learn("tell gena", "tell Jenna", first, emptyList(), "a", 2, core(LearnCandidate("gena", "Jenna", looksLikeName = true)))
        assertEquals(2, again.names.single().count)
    }

    @Test fun noChangeAsksTheCoreNothing() {
        val cur = Suggestions()
        val l = HistoryCorrection.learn("same  text", "same text", cur, emptyList(), "a", 1, core(LearnCandidate("x", "Y", looksLikeName = true)))
        assertSame(cur, l.next)
        assertTrue(l.names.isEmpty())
        assertTrue(calls.isEmpty())
        assertTrue(HistoryCorrection.learn("text", "  ", cur, emptyList(), "a", 1, core()).names.isEmpty())
    }

    @Test fun theCoreJsonCarriesTheNameFlag() {
        val c = CoreJson.decodeCandidates("""[{"original":"gina","corrected":"Jenna","looks_like_name":true},{"original":"a","corrected":"b"}]""")
        assertTrue(c[0].looksLikeName)
        assertFalse("an older core sends no flag", c[1].looksLikeName)
    }
}
