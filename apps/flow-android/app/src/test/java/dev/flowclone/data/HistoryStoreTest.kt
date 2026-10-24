package dev.flowclone.data

import android.database.sqlite.SQLiteDatabase
import dev.flowclone.FlowApp
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** history.db v3 (insert outcome + route) and the upgrade from a v2 file saved by 0.3.0. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], application = FlowApp::class)
class HistoryStoreTest {
    private val ctx get() = RuntimeEnvironment.getApplication()

    @After fun cleanup() { ctx.deleteDatabase(NAME) }

    @Test fun outcomeAndRouteRoundTrip() {
        val s = HistoryStore(ctx)
        s.add(HistoryRow(0, 1L, "a", "RAW_TRANSCRIPT", "x", "x", false, 100, insertOutcome = "target_changed", route = "local"), Retention.KEEP)
        val r = s.recent().single()
        assertEquals("target_changed", r.insertOutcome)
        assertEquals("local", r.route)
        s.close()
    }

    @Test fun v2FileUpgradesWithUnknownOutcome() {
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
            db.execSQL("INSERT INTO history(created_at, app_id, status, asr_text, final_text, inserted, audio_ms, asr_ms) VALUES (5, 'b', 'FORMATTED', 'old', 'Old.', 0, 900, 300)")
            db.version = 2
        }
        val s = HistoryStore(ctx)
        val old = s.recent().single()
        assertEquals("Old.", old.finalText)
        assertEquals(300L, old.asrMs)
        assertNull(old.insertOutcome)
        assertNull(old.route)
        s.add(HistoryRow(0, 6L, "b", "RAW_TRANSCRIPT", "new", "new", true, 100, insertOutcome = "inserted", route = "remote"), Retention.KEEP)
        assertEquals("remote", s.recent().first().route)
        s.close()
    }

    private companion object { const val NAME = "history.db" }
}
