package dev.murmur.learn

import dev.murmur.core.DictEntry
import dev.murmur.core.LearnCandidate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SuggestionsTest {
    private fun cand(heard: String, word: String) = LearnCandidate(original = heard, corrected = word)

    @Test fun addSkipsKnownAndDismissedAndCountsRepeats() {
        val r = Suggestions().add(listOf(cand("tailskale", "Tailscale"), cand("cooper netties", "Kubernetes")), "com.slack", 1, listOf(DictEntry("kubernetes")))
        assertEquals(listOf("Tailscale"), r.added.map { it.word })
        assertEquals(1, r.next.pending.size)
        val again = r.next.add(listOf(cand("tailskale", "tailscale")), "x", 2, emptyList())
        assertTrue("a repeat is not new", again.added.isEmpty())
        assertEquals(2, again.next.pending.single().count)
        val gone = again.next.dismiss("TAILSCALE")
        assertTrue(gone.pending.isEmpty())
        assertTrue("dismissed words are never suggested again", gone.add(listOf(cand("tailskale", "Tailscale")), "x", 3, emptyList()).added.isEmpty())
        assertTrue(gone.add(listOf(cand("tailskale", "Tailscale")), "x", 3, emptyList()).next.pending.isEmpty())
    }

    @Test fun replacementTargetsCountAsKnown() {
        val r = Suggestions().add(listOf(cand("mer mur", "Murmur")), "a", 1, listOf(DictEntry("mer mur app", "Murmur")))
        assertTrue(r.added.isEmpty())
    }

    @Test fun acceptReturnsAWordOrAReplacementAndRemovesIt() {
        val s = Suggestions().add(listOf(cand("mer mur", "Murmur"), cand("get hub", "GitHub")), "a", 1, emptyList()).next
        val (s1, e1) = s.accept("murmur")
        assertEquals(DictEntry("Murmur"), e1)
        val (s2, e2) = s1.accept("GitHub", asReplacement = true)
        assertEquals(DictEntry("get hub", "GitHub"), e2)
        assertTrue(s2.pending.isEmpty())
        assertNull("already accepted", s2.accept("GitHub").second)
        assertFalse("accepting is not dismissing", "github" in s2.dismissed)
    }

    @Test fun pendingIsCappedNewestFirstAndPruned() {
        var s = Suggestions()
        for (i in 0 until Suggestions.MAX_PENDING + 5) s = s.add(listOf(cand("x", "w$i")), "a", i.toLong(), emptyList()).next
        assertEquals(Suggestions.MAX_PENDING, s.pending.size)
        assertEquals("w${Suggestions.MAX_PENDING + 4}", s.pending.first().word)
        assertEquals(Suggestions.MAX_PENDING - 1, s.pruneKnown(listOf(DictEntry(s.pending.first().word))).pending.size)
    }

    @Test fun jsonRoundTripsAndToleratesJunk() {
        val s = Suggestions().add(listOf(cand("mer mur", "Murmur")), "com.notes", 42, emptyList()).next.dismiss("nope")
        assertEquals(s, Suggestions.decode(Suggestions.encode(s)))
        assertEquals(Suggestions(), Suggestions.decode(null))
        assertEquals(Suggestions(), Suggestions.decode("not json"))
    }

    @Test fun subtitleSaysWhatWasHeard() {
        assertEquals("Heard “mer mur”", Suggestions.subtitle(Suggestion("Murmur", "mer mur", "a", 1)))
        assertEquals("Heard “x” · fixed 3 times", Suggestions.subtitle(Suggestion("X", "x", "a", 1, count = 3)))
    }

    @Test fun brandNameEditBecomesAPendingSuggestion() {
        // The watch saw "mer mur" corrected; the core's learn_edit (faked here) returns the candidate.
        var seen: Pair<String, String>? = null
        val r = EditLearning.learn(
            "I use mer mur daily.", Outcome(EndReason.STABLE, "I use Murmur daily. Bye"), Suggestions(), emptyList(), "com.notes", 7,
        ) { p, e -> seen = p to e; listOf(cand("mer mur", "Murmur")) }!!
        assertEquals("text typed after the dictation is cut off", "I use mer mur daily." to "I use Murmur daily.", seen)
        assertEquals(listOf(Suggestion("Murmur", "mer mur", "com.notes", 7)), r.next.pending)
    }

    @Test fun nothingIsLearnedWithoutACountedEditOrCandidates() {
        val none: (String, String) -> List<LearnCandidate> = { _, _ -> error("not called") }
        assertNull(EditLearning.learn("a b", Outcome(EndReason.TIMEOUT, null), Suggestions(), emptyList(), "a", 1, none))
        assertNull(EditLearning.learn("a b", Outcome(EndReason.TIMEOUT, "a c"), Suggestions(), emptyList(), "a", 1) { _, _ -> emptyList() })
        assertNull("a core failure learns nothing", EditLearning.learn("a b", Outcome(EndReason.TIMEOUT, "a c"), Suggestions(), emptyList(), "a", 1) { _, _ -> error("boom") })
    }

    @Test fun watchOnlyWhenEnabledAndSomethingWentIn() {
        assertTrue(EditLearning.shouldWatch(true, "hello", sentKey = false, hasContext = true))
        assertFalse("setting off = no re-reads", EditLearning.shouldWatch(false, "hello", false, true))
        assertFalse("Enter sent the message", EditLearning.shouldWatch(true, "hello", true, true))
        assertFalse(EditLearning.shouldWatch(true, "  ", false, true))
        assertFalse(EditLearning.shouldWatch(true, "hello", false, false))
    }
}
