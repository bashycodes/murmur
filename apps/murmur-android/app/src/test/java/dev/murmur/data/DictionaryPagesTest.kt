package dev.murmur.data

import dev.murmur.core.DictEntry
import dev.murmur.data.DictionaryDraft.Field
import dev.murmur.data.DictionaryPages.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Dictionary & Snippets tabs (rows, labels, empty states) and the edit form (Save when changed + valid). */
class DictionaryPagesTest {
    private val tailscale = DictEntry("tailscail", "Tailscale", false)
    private val kubectl = DictEntry("kubectl", null, false)
    private val sig = DictEntry("my signature", "Best,\nAlex", true)
    private val all = listOf(tailscale, kubectl, sig)

    @Test fun listPagesShowOnlyTheirKindSortedAndFiltered() {
        assertEquals(listOf(kubectl, tailscale), DictionaryPages.entries(all, Kind.WORDS, ""))
        assertEquals(listOf(sig), DictionaryPages.entries(all, Kind.SNIPPETS, ""))
        assertEquals(listOf(tailscale), DictionaryPages.entries(all, Kind.WORDS, "tai"))
        assertEquals(listOf(sig), DictionaryPages.entries(all, Kind.SNIPPETS, "alex"))
    }

    @Test fun tabRowsShowHeardArrowWordAndSnippetText() {
        assertEquals(DictionaryPages.Row("tailscail", "Tailscale", "", "tailscail, replaced with Tailscale"), DictionaryPages.row(tailscale))
        assertEquals(DictionaryPages.Row(null, "kubectl", "", "kubectl"), DictionaryPages.row(kubectl))
        assertEquals(DictionaryPages.Row(null, "my signature", "Best, Alex", "my signature, inserts Best, Alex"), DictionaryPages.row(sig))
        val long = DictionaryPages.row(DictEntry("x", "a".repeat(200), true)).detail
        assertEquals(81, long.length)
        assertTrue(long.endsWith("…"))
    }

    @Test fun labels() {
        assertEquals("Search dictionary", DictionaryPages.searchLabel(Kind.WORDS))
        assertEquals("Search snippets", DictionaryPages.searchLabel(Kind.SNIPPETS))
        assertEquals("Add a word", DictionaryPages.addLabel(Kind.WORDS))
        assertEquals("Add a snippet", DictionaryPages.addLabel(Kind.SNIPPETS))
        assertEquals("Add word", DictionaryPages.editTitle(true, false))
        assertEquals("Edit snippet", DictionaryPages.editTitle(false, true))
    }

    @Test fun emptyStates() {
        assertEquals("No words yet", DictionaryPages.empty(Kind.WORDS, "", 0).title)
        assertTrue(DictionaryPages.empty(Kind.SNIPPETS, "", 0).hint.startsWith("Tap + to add one."))
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
        val d = DictionaryDraft.new(Kind.WORDS).copy(word = "  Kubernetes ")
        assertTrue(d.dirty); assertTrue(d.canSave(all))
        assertEquals(all + DictEntry("Kubernetes", null, false), d.save(all))
    }

    @Test fun duplicateWordIsAnInlineWordError() {
        val d = DictionaryDraft.new(Kind.WORDS).copy(word = "TAILSCAIL")
        assertEquals("\"TAILSCAIL\" is already in the dictionary.", d.visibleErrors(all)[Field.WORD])
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
        val d = DictionaryDraft.of(tailscale)
        assertFalse(d.dirty)
        assertFalse("unchanged", d.canSave(all))
        // Whitespace-only changes normalise away: still clean.
        assertFalse(d.copy(word = " tailscail ").dirty)
        val renamed = d.copy(word = "tailskale", replacement = "Tailscale")
        assertTrue(renamed.canSave(all))
        assertEquals(listOf(DictEntry("tailskale", "Tailscale", false), kubectl, sig), renamed.save(all))
        // Keeping its own word is not a clash.
        assertTrue(d.copy(replacement = "Tailscale VPN").canSave(all))
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
            DictionaryDraft.of(sig).copy(word = "tailscail"),
        )
        for (c in cases) {
            val old = DictionaryEdit.validate(all, c.original?.word, c.entry(), wantSnippet = c.snippet)
            assertEquals(c.toString(), old == null, c.errors(all).isEmpty())
        }
    }
}
