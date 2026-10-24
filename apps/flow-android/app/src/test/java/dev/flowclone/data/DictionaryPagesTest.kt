package dev.flowclone.data

import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryDraft.Field
import dev.flowclone.data.DictionaryPages.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Dictionary & snippets hub, list pages and the edit form (Save when changed + valid). */
class DictionaryPagesTest {
    private val wispr = DictEntry("wisper", "Wispr", false)
    private val kubectl = DictEntry("kubectl", null, false)
    private val sig = DictEntry("my signature", "Best,\nAlex", true)
    private val all = listOf(wispr, kubectl, sig)

    @Test fun hubSubtitlesCountEachKind() {
        assertEquals("2 words · 1 replacement", DictionaryPages.hubSubtitle(all, Kind.WORDS))
        assertEquals("1 snippet", DictionaryPages.hubSubtitle(all, Kind.SNIPPETS))
        assertEquals("None yet", DictionaryPages.hubSubtitle(emptyList(), Kind.WORDS))
        assertEquals("None yet", DictionaryPages.hubSubtitle(emptyList(), Kind.SNIPPETS))
        assertEquals("1 word", DictionaryPages.hubSubtitle(listOf(kubectl), Kind.WORDS))
        assertEquals("3 snippets", DictionaryPages.hubSubtitle(List(3) { DictEntry("s$it", "x", true) }, Kind.SNIPPETS))
    }

    @Test fun mainScreenSubtitle() {
        assertEquals("2 words · 1 snippet", DictionaryPages.mainSubtitle(all))
        assertEquals("No words or snippets yet", DictionaryPages.mainSubtitle(emptyList()))
        assertEquals("0 words · 1 snippet", DictionaryPages.mainSubtitle(listOf(sig)))
    }

    @Test fun listPagesShowOnlyTheirKindSortedAndFiltered() {
        assertEquals(listOf(kubectl, wispr), DictionaryPages.entries(all, Kind.WORDS, ""))
        assertEquals(listOf(sig), DictionaryPages.entries(all, Kind.SNIPPETS, ""))
        assertEquals(listOf(wispr), DictionaryPages.entries(all, Kind.WORDS, "wis"))
        assertEquals(listOf(sig), DictionaryPages.entries(all, Kind.SNIPPETS, "alex"))
    }

    @Test fun rowSubtitles() {
        assertEquals("→ Wispr", DictionaryPages.rowSubtitle(wispr))
        assertEquals("Spelling hint for the speech engine", DictionaryPages.rowSubtitle(kubectl))
        assertEquals("Best, Alex", DictionaryPages.rowSubtitle(sig))
        val long = DictEntry("x", "a".repeat(200), true)
        assertEquals(81, DictionaryPages.rowSubtitle(long).length)
        assertTrue(DictionaryPages.rowSubtitle(long).endsWith("…"))
    }

    @Test fun searchOnlyForLongLists() {
        assertFalse(DictionaryPages.showSearch(DictionaryPages.SEARCH_FROM - 1))
        assertTrue(DictionaryPages.showSearch(DictionaryPages.SEARCH_FROM))
    }

    @Test fun emptyStates() {
        assertEquals("No words yet", DictionaryPages.empty(Kind.WORDS, "", 0).title)
        assertTrue(DictionaryPages.empty(Kind.SNIPPETS, "", 0).hint.contains("Add"))
        assertEquals("Nothing matches “zz”", DictionaryPages.empty(Kind.WORDS, " zz ", 4).title)
    }

    // ---- edit form ----

    @Test fun newFormIsCleanAndShowsNoErrorsUntilTyped() {
        val d = DictionaryDraft.new(Kind.WORDS)
        assertFalse(d.dirty)
        assertFalse(d.canSave(all))
        assertEquals(emptyMap<Field, String>(), d.visibleErrors(all))
        // The error exists; it is just not shown on a pristine form.
        assertEquals("Enter a word or trigger phrase.", d.errors(all)[Field.WORD])
    }

    @Test fun validNewWordCanBeSavedAndAppends() {
        val d = DictionaryDraft.new(Kind.WORDS).copy(word = "  Tailscale ")
        assertTrue(d.dirty); assertTrue(d.canSave(all))
        assertEquals(all + DictEntry("Tailscale", null, false), d.save(all))
    }

    @Test fun duplicateWordIsAnInlineWordError() {
        val d = DictionaryDraft.new(Kind.WORDS).copy(word = "WISPER")
        assertEquals("\"WISPER\" is already in the dictionary.", d.visibleErrors(all)[Field.WORD])
        assertFalse(d.canSave(all))
    }

    @Test fun snippetNeedsItsExpansionAndBothErrorsShowAtOnce() {
        val d = DictionaryDraft.new(Kind.SNIPPETS).copy(word = "x".repeat(61))
        val e = d.visibleErrors(all)
        assertEquals("Keep it to 60 characters or fewer.", e[Field.WORD])
        assertEquals("A snippet needs the text it expands to.", e[Field.REPLACEMENT])
        val tooLong = DictionaryDraft.new(Kind.SNIPPETS).copy(word = "sig2", replacement = "y".repeat(DictionaryEdit.MAX_EXPANSION + 1))
        assertEquals("Expansion is limited to 2000 characters.", tooLong.visibleErrors(all)[Field.REPLACEMENT])
        assertNull(tooLong.visibleErrors(all)[Field.WORD])
    }

    @Test fun editingAnEntryIsCleanUntilChangedAndReplacesInPlace() {
        val d = DictionaryDraft.of(wispr)
        assertFalse(d.dirty)
        assertFalse("unchanged", d.canSave(all))
        // Whitespace-only changes normalise away: still clean.
        assertFalse(d.copy(word = " wisper ").dirty)
        val renamed = d.copy(word = "whisper", replacement = "Wispr")
        assertTrue(renamed.canSave(all))
        assertEquals(listOf(DictEntry("whisper", "Wispr", false), kubectl, sig), renamed.save(all))
        // Keeping its own word is not a clash.
        assertTrue(d.copy(replacement = "Wispr Flow").canSave(all))
    }

    @Test fun switchingKindIsAChangeAndKeepsValidation() {
        val d = DictionaryDraft.of(kubectl).copy(snippet = true)
        assertTrue(d.dirty)
        assertEquals("A snippet needs the text it expands to.", d.visibleErrors(all)[Field.REPLACEMENT])
        val ok = d.copy(replacement = "kubectl get pods")
        assertEquals(DictEntry("kubectl", "kubectl get pods", true), ok.save(all)[1])
    }

    @Test fun saveRuleMatchesTheOldValidator() {
        // Whatever the form allows, DictionaryEdit.validate (the old single-page editor) allows too.
        val cases = listOf(
            DictionaryDraft.new(Kind.WORDS).copy(word = "a"),
            DictionaryDraft.new(Kind.WORDS).copy(word = ""),
            DictionaryDraft.new(Kind.SNIPPETS).copy(word = "b", replacement = " "),
            DictionaryDraft.new(Kind.SNIPPETS).copy(word = "b", replacement = "text"),
            DictionaryDraft.of(sig).copy(word = "wisper"),
        )
        for (c in cases) {
            val old = DictionaryEdit.validate(all, c.original?.word, c.entry(), wantSnippet = c.snippet)
            assertEquals(c.toString(), old == null, c.errors(all).isEmpty())
        }
    }
}
