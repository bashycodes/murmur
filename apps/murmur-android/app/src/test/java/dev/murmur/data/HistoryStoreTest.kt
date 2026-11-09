package dev.murmur.data

import android.database.sqlite.SQLiteDatabase
import dev.murmur.MurmurApp
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
@Config(sdk = [34], application = MurmurApp::class)
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

    /** Insights backfill: every saved dictation (not just the 500 the list shows), errors left out. */
    @Test fun statsRowsCoverAllRowsOldestFirstWithoutErrors() {
        val s = HistoryStore(ctx)
        for (i in 1..502) s.add(HistoryRow(0, i.toLong(), "a", "RAW_TRANSCRIPT", "x", "w$i", true, 10), Retention.KEEP)
        s.add(HistoryRow(0, 600, "a", "ERROR", "", "", false, 10), Retention.KEEP)
        val rows = s.statsRows()
        assertEquals(502, rows.size)
        assertEquals(InsightsStore.Row(1, "a", "w1", 10), rows.first())
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

    /** Command Mode rows: instruction, original selection and result round-trip; Insights never counts them. */
    @Test fun commandRowsRoundTripAndStayOutOfInsights() {
        val s = HistoryStore(ctx)
        val rec = dev.murmur.command.CommandRecord("make it shorter", "A very long sentence.", "Short.", "com.example.notes", "inserted")
        s.add(HistoryRow.ofCommand(rec, 7L), Retention.KEEP)
        s.add(HistoryRow.ofCommand(rec.copy(original = null, result = null, outcome = "command_failed"), 8L), Retention.KEEP)
        val (failed, ok) = s.recent()
        assertEquals(true, ok.isCommand)
        assertEquals("make it shorter", ok.asrText)
        assertEquals("A very long sentence.", ok.commandOriginal)
        assertEquals("Short.", ok.finalText)
        assertEquals(true, ok.inserted)
        assertEquals("Command", HistoryView.label(ok))
        assertNull(failed.commandOriginal)
        assertEquals(false, failed.inserted)
        assertEquals("Command failed", HistoryView.label(failed))
        assertEquals("(no result)", HistoryView.text(failed, raw = false))
        assertEquals("make it shorter", HistoryView.text(failed, raw = true))
        assertEquals(emptyList<InsightsStore.Row>(), s.statsRows())
        s.close()
    }

    @Test fun v3FileUpgradesToV4() {
        ctx.deleteDatabase(NAME)
        val f = ctx.getDatabasePath(NAME).also { it.parentFile?.mkdirs() }
        SQLiteDatabase.openOrCreateDatabase(f, null).use { db ->
            db.execSQL(
                """CREATE TABLE history(id INTEGER PRIMARY KEY AUTOINCREMENT, created_at INTEGER NOT NULL,
                   app_id TEXT, status TEXT NOT NULL, asr_text TEXT NOT NULL, final_text TEXT NOT NULL,
                   inserted INTEGER NOT NULL, audio_ms INTEGER NOT NULL)""",
            )
            db.execSQL("ALTER TABLE history ADD COLUMN fallback_reason TEXT")
            for (c in listOf("asr_ms", "llm_ms", "total_ms")) db.execSQL("ALTER TABLE history ADD COLUMN $c INTEGER NOT NULL DEFAULT 0")
            for (c in listOf("insert_outcome", "route")) db.execSQL("ALTER TABLE history ADD COLUMN $c TEXT")
            db.execSQL("INSERT INTO history(created_at, app_id, status, asr_text, final_text, inserted, audio_ms, route) VALUES (5, 'b', 'FORMATTED', 'old', 'Old.', 1, 900, 'local')")
            db.version = 3
        }
        val s = HistoryStore(ctx)
        val old = s.recent().single()
        assertEquals("local", old.route)
        assertNull(old.commandOriginal)
        assertEquals(false, old.isCommand)
        s.close()
    }

    private companion object { const val NAME = "history.db" }
}
