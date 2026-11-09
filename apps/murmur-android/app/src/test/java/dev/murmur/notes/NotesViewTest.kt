package dev.murmur.notes

import org.junit.Assert.assertEquals
import org.junit.Test

/** What the Notes list shows: titles from the first line, content search. */
class NotesViewTest {
    private fun note(id: String, text: String, pinned: Boolean = false) = Note(id, text, pinned, 0, 0)

    @Test fun titleIsTheFirstNonBlankLineTrimmed() {
        assertEquals("Buy milk", NotesView.title(note("a", "\n   Buy milk  \nand bread")))
    }

    @Test fun blankNoteHasAPlaceholderTitle() {
        assertEquals(NotesView.EMPTY_TITLE, NotesView.title(note("a", " \n\t")))
    }

    @Test fun longFirstLineIsCut() {
        val t = NotesView.title(note("a", "word ".repeat(40)))
        assertEquals(NotesView.TITLE_MAX + 1, t.length)
        assertEquals('…', t.last())
    }

    @Test fun scratchpadIsAlwaysCalledQuickNote() {
        assertEquals(NotesView.QUICK_NOTE, NotesView.title(note(DocumentStore.SCRATCHPAD, "whatever")))
    }

    @Test fun previewIsTheTextAfterTheTitleLine() {
        assertEquals("and bread and eggs", NotesView.preview(note("a", "Buy milk\nand bread\n\nand eggs")))
        assertEquals("", NotesView.preview(note("a", "only a title")))
        // The Quick note's title is fixed, so its preview starts at its first line.
        assertEquals("first line second", NotesView.preview(note(DocumentStore.SCRATCHPAD, "first line\nsecond")))
    }

    @Test fun searchMatchesContentIgnoringCaseAndKeepsOrder() {
        val notes = listOf(note("a", "Plumber on Tuesday"), note("b", "Groceries\nmilk"), note("c", "call the PLUMBER"))
        assertEquals(listOf("a", "c"), NotesView.filter(notes, " plumber ").map { it.id })
        assertEquals(listOf("b"), NotesView.filter(notes, "MILK").map { it.id })
        assertEquals(notes, NotesView.filter(notes, "  "))
        // "Quick note" is the Scratchpad's title, so searching it finds the Scratchpad.
        assertEquals(listOf("scratchpad"), NotesView.filter(notes + note("scratchpad", "x"), "quick").map { it.id })
    }
}
