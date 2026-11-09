package dev.flowclone.notes

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

/** Scratchpad persistence: one UTF-8 file per document id, replaced atomically. */
class DocumentStoreTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun store(dir: File = File(tmp.root, "notes")) = FileDocumentStore(dir)

    @Test fun missingDocumentLoadsAsNull() {
        assertNull(store().load(DocumentStore.SCRATCHPAD))
    }

    @Test fun savedTextSurvivesANewStoreInstance() {
        val text = "Shopping:\n- milk\n- café ☕\n"
        store().save(DocumentStore.SCRATCHPAD, text)
        // A fresh instance over the same directory = the app killed and reopened.
        assertEquals(text, store().load(DocumentStore.SCRATCHPAD))
    }

    @Test fun saveReplacesAndLeavesNoTempFile() {
        val dir = File(tmp.root, "notes")
        val s = store(dir)
        s.save(DocumentStore.SCRATCHPAD, "first version that is longer")
        s.save(DocumentStore.SCRATCHPAD, "second")
        assertEquals("second", s.load(DocumentStore.SCRATCHPAD))
        assertEquals(listOf("scratchpad.txt"), dir.list()!!.sorted())
    }

    @Test fun emptyTextIsKeptAsEmptyNotMissing() {
        val s = store()
        s.save(DocumentStore.SCRATCHPAD, "x")
        s.save(DocumentStore.SCRATCHPAD, "")
        assertEquals("", s.load(DocumentStore.SCRATCHPAD))
    }

    @Test fun documentsAreSeparateById() {
        val s = store()
        s.save("scratchpad", "a")
        s.save("note-2", "b")
        assertEquals("a", s.load("scratchpad"))
        assertEquals("b", s.load("note-2"))
    }

    @Test fun idsThatCouldEscapeTheDirectoryAreRefused() {
        val s = store()
        for (bad in listOf("", "../x", "a/b", "A", ".hidden", "x".repeat(65))) {
            assertFalse(bad, DocumentStore.isValidId(bad))
            assertTrue(bad, runCatching { s.save(bad, "t") }.isFailure)
            assertTrue(bad, runCatching { s.load(bad) }.isFailure)
        }
        assertFalse(File(tmp.root, "x.txt").exists())
    }
}
