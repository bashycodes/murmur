package dev.flowclone.learn

import dev.flowclone.core.DictEntry
import dev.flowclone.core.LearnCandidate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SuggestionsTest {
    private fun cand(heard: String, word: String) = LearnCandidate(original = heard, corrected = word)

    @Test fun addSkipsKnownAndDismissedAndCountsRepeats() {
        val r = Suggestions().add(listOf(cand("whisper", "Wispr"), cand("cooper netties", "Kubernetes")), "com.slack", 1, listOf(DictEntry("kubernetes")))
        assertEquals(listOf("Wispr"), r.added.map { it.word })
        assertEquals(1, r.next.pending.size)
        val again = r.next.add(listOf(cand("whisper", "wispr")), "x", 2, emptyList())
        assertTrue("a repeat is not new", again.added.isEmpty())
        assertEquals(2, again.next.pending.single().count)
        val gone = again.next.dismiss("WISPR")
        assertTrue(gone.pending.isEmpty())
        assertTrue("dismissed words are never suggested again", gone.add(listOf(cand("whisper", "Wispr")), "x", 3, emptyList()).added.isEmpty())
        assertTrue(gone.add(listOf(cand("whisper", "Wispr")), "x", 3, emptyList()).next.pending.isEmpty())
    }

    @Test fun replacementTargetsCountAsKnown() {
        val r = Suggestions().add(listOf(cand("flo clone", "Flow Clone")), "a", 1, listOf(DictEntry("flow clone app", "Flow Clone")))
        assertTrue(r.added.isEmpty())
    }

    @Test fun acceptReturnsAWordOrAReplacementAndRemovesIt() {
        val s = Suggestions().add(listOf(cand("flo clone", "Flow Clone"), cand("get hub", "GitHub")), "a", 1, emptyList()).next
        val (s1, e1) = s.accept("flow clone")
        assertEquals(DictEntry("Flow Clone"), e1)
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
        val s = Suggestions().add(listOf(cand("flo clone", "Flow Clone")), "com.notes", 42, emptyList()).next.dismiss("nope")
        assertEquals(s, Suggestions.decode(Suggestions.encode(s)))
        assertEquals(Suggestions(), Suggestions.decode(null))
        assertEquals(Suggestions(), Suggestions.decode("not json"))
    }

    @Test fun subtitleSaysWhatWasHeard() {
        assertEquals("Heard “flo clone”", Suggestions.subtitle(Suggestion("Flow Clone", "flo clone", "a", 1)))
        assertEquals("Heard “x” · fixed 3 times", Suggestions.subtitle(Suggestion("X", "x", "a", 1, count = 3)))
    }

    @Test fun floCloneEditBecomesAPendingSuggestion() {
        // The watch saw "flo clone" corrected; the core's learn_edit (faked here) returns the candidate.
        var seen: Pair<String, String>? = null
        val r = EditLearning.learn(
            "I use flo clone daily.", Outcome(EndReason.STABLE, "I use Flow Clone daily. Bye"), Suggestions(), emptyList(), "com.notes", 7,
        ) { p, e -> seen = p to e; listOf(cand("flo clone", "Flow Clone")) }!!
        assertEquals("text typed after the dictation is cut off", "I use flo clone daily." to "I use Flow Clone daily.", seen)
        assertEquals(listOf(Suggestion("Flow Clone", "flo clone", "com.notes", 7)), r.next.pending)
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
