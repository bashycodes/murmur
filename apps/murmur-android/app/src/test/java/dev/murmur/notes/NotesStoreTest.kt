package dev.murmur.notes

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

/** Notes: `<id>.txt` files + `index.json` (pinned, created, updated), tolerant of either side missing. */
class NotesStoreTest {
    @get:Rule val tmp = TemporaryFolder()

    private val dir get() = File(tmp.root, "notes")
    private var now = 1_000L
    private fun store() = NotesStore(dir) { now }

    @Test fun savedNotesSurviveANewStoreWithTimes() {
        val s = store()
        s.save("n1", "Buy milk\nand bread")
        now = 2_000
        s.save("n2", "Call mum")
        now = 3_000
        s.save("n1", "Buy milk\nand eggs")
        // A fresh instance over the same directory = the app killed and reopened.
        val notes = store().list().associateBy { it.id }
        assertEquals("Buy milk\nand eggs", notes.getValue("n1").text)
        assertEquals(1_000L, notes.getValue("n1").createdAt)
        assertEquals(3_000L, notes.getValue("n1").updatedAt)
        assertEquals(2_000L, notes.getValue("n2").createdAt)
        assertEquals("Call mum", store().load("n2"))
    }

    @Test fun listIsPinnedFirstThenMostRecentlyUpdated() {
        val s = store()
        s.save("a", "a"); now = 2_000
        s.save("b", "b"); now = 3_000
        s.save("c", "c")
        s.setPinned("a", true)
        assertEquals(listOf("a", "c", "b"), store().list().map { it.id })
        s.setPinned("a", false)
        assertEquals(listOf("c", "b", "a"), store().list().map { it.id })
    }

    @Test fun pinningDoesNotChangeTheUpdatedTime() {
        val s = store()
        s.save("a", "a")
        now = 5_000
        s.setPinned("a", true)
        assertEquals(1_000L, store().list().single().updatedAt)
    }

    @Test fun existingScratchpadWithoutIndexIsListedPinnedAndKept() {
        // The Scratchpad release wrote only files/notes/scratchpad.txt, no index.
        dir.mkdirs()
        val f = File(dir, "scratchpad.txt").apply { writeText("old scratchpad text"); setLastModified(42_000) }
        val n = store().list().single()
        assertEquals(DocumentStore.SCRATCHPAD, n.id)
        assertEquals("old scratchpad text", n.text)
        assertTrue(n.pinned)
        assertEquals(42_000L, n.updatedAt)
        assertEquals("old scratchpad text", f.readText())
    }

    @Test fun scratchpadCanBeUnpinned() {
        dir.mkdirs(); File(dir, "scratchpad.txt").writeText("x")
        store().setPinned(DocumentStore.SCRATCHPAD, false)
        assertFalse(store().list().single().pinned)
    }

    @Test fun fileWithoutIndexEntryIsListedFromItsModifiedTime() {
        dir.mkdirs()
        File(dir, "orphan.txt").apply { writeText("found"); setLastModified(7_000) }
        val n = store().list().single()
        assertEquals("orphan", n.id)
        assertFalse(n.pinned)
        assertEquals(7_000L, n.createdAt)
        assertEquals(7_000L, n.updatedAt)
    }

    @Test fun staleIndexEntryWithoutFileIsIgnoredAndDroppedOnNextWrite() {
        val s = store()
        s.save("gone", "x")
        File(dir, "gone.txt").delete()
        assertTrue(store().list().isEmpty())
        store().save("kept", "y")
        assertFalse(File(dir, "index.json").readText().contains("gone"))
    }

    @Test fun brokenIndexIsTreatedAsEmpty() {
        dir.mkdirs()
        File(dir, "a.txt").writeText("text")
        File(dir, "index.json").writeText("{not json")
        assertEquals(listOf("a"), store().list().map { it.id })
        store().setPinned("a", true)
        assertTrue(store().list().single().pinned)
    }

    @Test fun filesWithUnsafeNamesAndOtherFilesAreNotListed() {
        dir.mkdirs()
        File(dir, "Upper.txt").writeText("x")
        File(dir, ".hidden.txt").writeText("x")
        File(dir, "a.txt.tmp").writeText("x")
        File(dir, "notes.md").writeText("x")
        File(dir, "ok.txt").writeText("x")
        assertEquals(listOf("ok"), store().list().map { it.id })
    }

    @Test fun indexEntriesWithUnsafeIdsAreIgnored() {
        dir.mkdirs()
        File(dir, "index.json").writeText("""{"version":1,"notes":{"../x":{"pinned":true,"created":1,"updated":1}}}""")
        assertTrue(store().list().isEmpty())
    }

    @Test fun deleteRemovesFileAndEntry() {
        val s = store()
        s.save("a", "a"); s.setPinned("a", true)
        s.save("b", "b")
        s.delete("a")
        assertFalse(File(dir, "a.txt").exists())
        assertNull(s.load("a"))
        assertFalse(File(dir, "index.json").readText().contains("\"a\""))
        assertEquals(listOf("b"), store().list().map { it.id })
    }

    @Test fun unsafeIdsAreRefusedEverywhere() {
        val s = store()
        for (bad in listOf("", "../x", "a/b", "A", ".hidden", "index.json", "x".repeat(65))) {
            assertTrue(bad, runCatching { s.save(bad, "t") }.isFailure)
            assertTrue(bad, runCatching { s.load(bad) }.isFailure)
            assertTrue(bad, runCatching { s.delete(bad) }.isFailure)
            assertTrue(bad, runCatching { s.setPinned(bad, true) }.isFailure)
        }
        assertFalse(File(tmp.root, "x.txt").exists())
    }

    @Test fun newIdsAreValidAndUnused() {
        val s = store()
        val a = s.newId()
        assertTrue(a, DocumentStore.isValidId(a))
        s.save(a, "x")
        val b = s.newId()
        assertTrue(b, DocumentStore.isValidId(b))
        assertNotEquals(a, b)
        assertNotEquals(DocumentStore.SCRATCHPAD, b)
    }

    @Test fun newIdIsNotWrittenUntilSaved() {
        val s = store()
        s.newId()
        assertTrue(s.list().isEmpty())
    }

    @Test fun replaceAllSwapsEveryNoteAndKeepsTimesAndPins() {
        val s = store()
        s.save("old", "gone after restore")
        s.save("scratchpad", "old quick note")
        s.replaceAll(listOf(Note("scratchpad", "restored", false, 10, 20), Note("n1", "body", true, 30, 40)))
        val notes = store().list()
        assertEquals(listOf("n1", "scratchpad"), notes.map { it.id })
        assertEquals(Note("n1", "body", true, 30, 40), notes[0])
        assertEquals(Note("scratchpad", "restored", false, 10, 20), notes[1])
        assertFalse(File(tmp.root, "notes.restore").exists())
        assertFalse(File(tmp.root, "notes.old").exists())
    }

    @Test fun replaceAllWithABadIdChangesNothing() {
        val s = store()
        s.save("keep", "still here")
        runCatching { s.replaceAll(listOf(Note("../x", "evil", false, 1, 1))) }
        assertEquals(listOf("keep"), store().list().map { it.id })
    }
}
