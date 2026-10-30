package dev.flowclone.data

import android.database.sqlite.SQLiteDatabase
import dev.flowclone.FlowApp
import dev.flowclone.core.CommitResult
import dev.flowclone.core.CommitStatus
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.io.File

/** history.db v5 (D35): recordings for Retry, versions of retried rows, and the upgrade from v4. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class RetryStoreTest {
    private val ctx get() = RuntimeEnvironment.getApplication()
    private val dir get() = File(ctx.filesDir, "retry_audio")
    private val pcm = ShortArray(1_600) { 1_000 }

    @After fun cleanup() { ctx.deleteDatabase(NAME); dir.deleteRecursively() }

    private fun row(at: Long, text: String = "Note $at.") =
        HistoryRow(0, at, "com.example.notes", "FORMATTED", text.lowercase(), text, true, 1_000, appCategory = "document")

    @Test fun onlyTheNewestRowsKeepTheirRecording() {
        val s = HistoryStore(ctx)
        val ids = (1L..4L).map { s.add(row(it), Retention.KEEP)!! }
        for (id in ids) s.recordAudio(id, pcm, keep = 2)
        assertEquals(ids.takeLast(2).toSet(), s.withAudio(ids))
        s.pruneAudio(1)
        assertEquals(setOf(ids.last()), s.withAudio(ids))
        s.pruneAudio(0)
        assertEquals(emptySet<Long>(), s.withAudio(ids))
        assertEquals("document", s.row(ids.first())!!.appCategory)
        s.close()
    }

    @Test fun recordingsAndVersionsGoWithTheirRow() {
        val s = HistoryStore(ctx)
        val a = s.add(row(1), Retention.KEEP)!!
        val b = s.add(row(2), Retention.KEEP)!!
        s.recordAudio(a, pcm, 20); s.recordAudio(b, pcm, 20)
        s.applyRetry(a, CommitResult(CommitStatus.FORMATTED, finalText = "Retried."), now = 5)
        s.delete(a)
        assertFalse(s.hasAudio(a))
        assertTrue(s.versions(a).isEmpty())
        assertTrue(s.hasAudio(b))
        s.clear()
        assertFalse(s.hasAudio(b))
        assertTrue(dir.listFiles().orEmpty().isEmpty())
        s.close()
    }

    @Test fun expiredRowsTakeTheirRecordingAlong() {
        val s = HistoryStore(ctx)
        val old = s.add(row(1), Retention.KEEP)!!
        s.recordAudio(old, pcm, 20)
        s.purge(Retention.DAY, now = 2 * 86_400_000L)
        assertFalse(s.hasAudio(old))
        s.close()
    }

    @Test fun retriesAddVersionsAndTheRowShowsTheNewest() {
        val s = HistoryStore(ctx)
        val id = s.add(row(1, "Hello Alex."), Retention.KEEP)!!
        val (before, n1) = s.applyRetry(id, CommitResult(CommitStatus.RAW_TRANSCRIPT, asrText = "hello alix", finalText = "hello alix"), now = 10)!!
        assertEquals("Hello Alex.", before.finalText)
        assertEquals(1, n1)
        val (_, n2) = s.applyRetry(id, CommitResult(CommitStatus.FORMATTED, asrText = "hello alex", finalText = "Hello, Alex!"), now = 20)!!
        assertEquals(2, n2)
        val vs = s.versions(id)
        assertEquals(listOf(0, 1, 2), vs.map { it.n })
        assertEquals(listOf("Hello Alex.", "hello alix", "Hello, Alex!"), vs.map { it.text })
        val now = s.row(id)!!
        assertEquals("Hello, Alex!", now.finalText)
        assertEquals("when it was dictated stays", 1L, now.createdAt)
        assertEquals(mapOf(id to vs), s.versionsFor(listOf(id, 999)))
        assertNull(s.applyRetry(999, CommitResult(CommitStatus.FORMATTED, finalText = "x")))
        s.close()
    }

    @Test fun aRestoreForgetsRecordingsAndVersions() {
        val s = HistoryStore(ctx)
        val id = s.add(row(1), Retention.KEEP)!!
        s.recordAudio(id, pcm, 20)
        s.applyRetry(id, CommitResult(CommitStatus.FORMATTED, finalText = "Retried."))
        s.replaceAll(listOf(row(3)), Retention.KEEP)
        val restored = s.recent().single()
        assertFalse(s.hasAudio(restored.id))
        assertTrue(s.versions(restored.id).isEmpty())
        s.close()
    }

    @Test fun v4FileUpgradesToV5() {
        ctx.deleteDatabase(NAME)
        val f = ctx.getDatabasePath(NAME).also { it.parentFile?.mkdirs() }
        SQLiteDatabase.openOrCreateDatabase(f, null).use { db ->
            db.execSQL(
                """CREATE TABLE history(id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL,
                   app_id TEXT, status TEXT NOT NULL, asr_text TEXT NOT NULL, final_text TEXT NOT NULL,
                   inserted INTEGER NOT NULL, audio_ms INTEGER NOT NULL)""",
            )
            db.execSQL("CREATE INDEX history_created ON history(created_at)")
            db.execSQL("ALTER TABLE history ADD COLUMN fallback_reason TEXT")
            for (c in listOf("asr_ms", "llm_ms", "total_ms")) db.execSQL("ALTER TABLE history ADD COLUMN $c INTEGER NOT NULL DEFAULT 0")
            for (c in listOf("insert_outcome", "route")) db.execSQL("ALTER TABLE history ADD COLUMN $c TEXT")
            db.execSQL("ALTER TABLE history ADD COLUMN command_original TEXT")
            db.execSQL("INSERT INTO history(created_at, app_id, status, asr_text, final_text, inserted, audio_ms) VALUES (5, 'b', 'FORMATTED', 'old', 'Old.', 1, 900)")
            db.version = 4
        }
        val s = HistoryStore(ctx)
        val old = s.recent().single()
        assertNull(old.appCategory)
        assertTrue(s.versions(old.id).isEmpty())
        s.applyRetry(old.id, CommitResult(CommitStatus.FORMATTED, finalText = "New."))
        assertEquals(listOf("Old.", "New."), s.versions(old.id).map { it.text })
        s.close()
    }

    private companion object { const val NAME = "history.db" }
}
