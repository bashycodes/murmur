package dev.murmur.calls

import dev.murmur.core.MeetingJson
import dev.murmur.core.MeetingSegment
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

/** D46 transcripts on disk and their wording. Synthetic text only. */
class CallsStoreTest {
    private val dir: File = Files.createTempDirectory("calls").toFile()
    private val store = CallsStore(dir)

    @After fun cleanup() { dir.deleteRecursively() }

    private fun t(id: String, created: Long, vararg text: String) = CallTranscript(
        id, created, created - 1000, "rec-$id.m4a", 65_000, "en",
        text.mapIndexed { i, s -> MeetingSegment(i * 10_000L, i * 10_000L + 9000, s) },
    )

    @Test fun savesListsNewestFirstLoadsAndDeletes() {
        store.save(t("a", 1, "first call"))
        store.save(t("b", 3, "third call"))
        store.save(t("c", 2, "second call"))
        assertEquals(listOf("b", "c", "a"), store.list().map { it.id })
        assertEquals(t("c", 2, "second call"), store.load("c"))
        assertTrue(store.delete("c"))
        assertNull(store.load("c"))
        assertEquals(listOf("b", "a"), store.list().map { it.id })
        assertFalse(File(dir, "c.tmp").exists())
    }

    @Test fun idsCannotLeaveTheFolderAndBrokenFilesAreSkippedNotDeleted() {
        File(dir.parentFile, "outside.json").let { f ->
            f.writeText("{}")
            assertFalse(store.delete("../outside"))
            assertTrue(f.exists()); f.delete()
        }
        assertNull(store.load("../x"))
        File(dir, "zz.json").writeText("not json")
        store.save(t("ok", 1, "fine"))
        assertEquals(listOf("ok"), store.list().map { it.id })
        assertTrue(File(dir, "zz.json").exists())
    }

    @Test fun roundTripsSegmentFields() {
        val x = CallTranscript("r", 5, 4, "שיחה.m4a", 1000, "he", listOf(
            MeetingSegment(0, 900, "שלום", "he", "zipformer-he"),
            MeetingSegment(900, 1000, "", error = "engine failed"),
        ))
        assertEquals(x, CallsStore.decode(CallsStore.encode(x)))
    }

    @Test fun newIdsAreValidAndOrdered() {
        val a = CallsStore.newId(1_700_000_000_000); val b = CallsStore.newId(1_700_000_100_000)
        assertTrue(CallsStore.isValidId(a) && CallsStore.isValidId(b))
        assertTrue(a < b)
    }

    @Test fun wording() {
        assertEquals("first line", CallsView.firstLine(t("x", 1, "", "  first line\nmore ", "later")))
        assertEquals(CallsView.NO_SPEECH, CallsView.firstLine(t("x", 1)))
        assertEquals("0:00", CallsView.clock(0)); assertEquals("4:05", CallsView.clock(245_400)); assertEquals("1:02:03", CallsView.clock(3_723_000))
        val withError = t("x", 1, "hello", "there").let { it.copy(segments = it.segments + MeetingSegment(20_000, 25_000, "", error = "timeout")) }
        assertEquals("[0:00] hello\n[0:10] there\n[0:20] (not transcribed: timeout)", CallsView.text(withError))
        assertEquals("Hebrew", CallsView.languageName("he")); assertEquals("English", CallsView.languageName("en"))
        assertEquals("Transcribing a.m4a · 42 %", CallsView.progressLabel("a.m4a", 0.425f, false))
        assertTrue(CallsView.progressLabel("a.m4a", null, true).startsWith("Loading"))
    }

    @Test fun meetingJsonFromTheNativeSide() {
        val s = MeetingJson.decodeSegments("""[{"start_ms":0,"end_ms":8000,"text":"hi","language":"en","engine":"whisper"},{"start_ms":8000,"end_ms":9000,"text":"","error":"x"}]""")
        assertEquals(listOf(MeetingSegment(0, 8000, "hi", "en", "whisper"), MeetingSegment(8000, 9000, "", null, null, "x")), s)
        val p = MeetingJson.decodeProgress("""{"done_ms":5000,"received_ms":6000,"total_ms":10000,"segments":1}""")
        assertEquals(0.5f, p.fraction!!, 1e-6f)
        assertNull(MeetingJson.decodeProgress("{}").fraction)
    }
}
