package dev.murmur.data

import dev.murmur.core.CommitResult
import dev.murmur.core.CommitStatus
import dev.murmur.core.RouteInfo
import dev.murmur.core.Timings
import dev.murmur.debug.InsertOutcome
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** D35 "Retry with current settings": which recordings stay, the menu, versions, Insights. */
class RetryLogicTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun row(id: Long = 1, status: String = "FORMATTED", text: String = "Hello Alex.") =
        HistoryRow(id, 1_000, "com.example.notes", status, "hello alex", text, true, 2_000, route = "local", appCategory = "personal_chat")

    @Test fun onlyTheNewestRowsKeepTheirRecording() {
        assertEquals(setOf(9L, 8L), RetryLogic.keepIds(listOf(9, 8, 7, 3), 2))
        assertEquals(emptySet<Long>(), RetryLogic.keepIds(listOf(9, 8), 0))
        assertEquals(20, Settings().retryAudioKeep)
        assertEquals(RetryLogic.MAX_KEEP, RetryLogic.keepOf(10_000))
        assertEquals(0, RetryLogic.keepOf(-3))
    }

    @Test fun protectedFieldsAppsAndCommandsKeepNoRecording() {
        fun keeps(
            keep: Int = 20, retention: Retention = Retention.KEEP, command: Boolean = false,
            outcome: String? = InsertOutcome.INSERTED, field: Boolean = false, app: Boolean = false,
        ) = RetryLogic.keepsRecording(keep, retention, command, outcome, field, app)
        assertTrue(keeps())
        assertTrue("a failed transcription can be retried", keeps(outcome = InsertOutcome.ERROR))
        assertFalse("setting off", keeps(keep = 0))
        assertFalse("history not saved", keeps(retention = Retention.NEVER))
        assertFalse("command mode", keeps(command = true))
        assertFalse("password field", keeps(field = true))
        assertFalse("banking / denylisted app", keeps(app = true))
        assertFalse("protected target", keeps(outcome = InsertOutcome.TARGET_PROTECTED))
    }

    @Test fun menuOffersRetryOnlyWhileTheRecordingIsKept() {
        val (details, retry, delete) = RetryLogic.MenuItem.entries
        assertEquals(listOf(details, retry, delete), RetryLogic.menu(row(), hasAudio = true))
        assertEquals(listOf(details, delete), RetryLogic.menu(row(), hasAudio = false))
        assertEquals(listOf(details, delete), RetryLogic.menu(row(status = HistoryRow.COMMAND), hasAudio = true))
        assertEquals("Retry with current settings", retry.label)
    }

    @Test fun aRetryKeepsWhenAndWhereButTakesTheNewText() {
        val r = CommitResult(
            CommitStatus.RAW_TRANSCRIPT, asrText = "hello alix", finalText = "hello alix", plainText = "hello alix",
            timings = Timings(audioMs = 1_990, asrMs = 120, totalMs = 150), route = RouteInfo("remote"),
        )
        val old = row()
        val new = RetryLogic.retried(old, r)
        assertEquals("hello alix", new.finalText)
        assertEquals("RAW_TRANSCRIPT", new.status)
        assertEquals("remote", new.route)
        assertEquals(120L, new.asrMs)
        assertEquals(old.createdAt, new.createdAt)
        assertEquals(old.appId, new.appId)
        assertEquals(old.inserted, new.inserted)
        assertEquals("the recording length stays", 2_000L, new.audioMs)
        assertEquals(HistoryVersion(0, 1_000, "FORMATTED", "hello alex", "Hello Alex.", "local"), RetryLogic.original(old))
    }

    @Test fun failuresSayWhy() {
        assertEquals(RetryLogic.EMPTY, RetryLogic.failure(CommitResult(CommitStatus.EMPTY)))
        assertEquals("server down", RetryLogic.failure(CommitResult(CommitStatus.ERROR, error = "server down")))
        assertEquals("Transcription failed", RetryLogic.failure(CommitResult(CommitStatus.ERROR)))
        assertNull(RetryLogic.failure(CommitResult(CommitStatus.FORMATTED, finalText = "Hi.")))
    }

    @Test fun pillsAppearOnceRetried() {
        val v0 = HistoryVersion(0, 1, "FORMATTED", "a", "A.")
        val v1 = HistoryVersion(1, 2, "FORMATTED", "b", "")
        assertEquals(emptyList<String>(), RetryLogic.pills(listOf(v0)))
        assertEquals(listOf("0", "1"), RetryLogic.pills(listOf(v0, v1)))
        assertEquals("b", v1.text)
        assertEquals("Off", RetryLogic.keepLabel(0))
        assertEquals("Newest 20 dictations", RetryLogic.keepLabel(20))
    }

    @Test fun insightsMoveToTheNewestVersion() {
        val day = 20_000L
        val a = Insights.Aggregates().add(day, 4, 2_000, "app")
        val b = a.retried(day, 4, 6, 2_000, "app")
        val d = b.days.values.single()
        assertEquals(6L, d.words)
        assertEquals(1L, d.count)
        assertEquals(mapOf("app" to 6L), d.apps)
        val gone = b.retried(day, 6, 0, 2_000, "app").days.values.single()
        assertEquals(0L, gone.words); assertEquals(0L, gone.count); assertEquals(0L, gone.ms)
        val added = Insights.Aggregates().retried(day, 0, 3, 1_000, "app").days.values.single()
        assertEquals(3L, added.words); assertEquals(1L, added.count)
        assertEquals("a reset day stays reset", Insights.Aggregates(), Insights.Aggregates().retried(day, 4, 6, 0, "app"))
        assertEquals(0L, RetryLogic.countedWords("ERROR", "words here"))
        assertEquals(2L, RetryLogic.countedWords("FORMATTED", "words here"))
    }

    @Test fun recordingsAreFilesThatGoWhenAsked() {
        val store = RetryAudioStore(tmp.newFolder("retry_audio"))
        val pcm = ShortArray(3_200) { (it % 100).toShort() }
        store.save(7, pcm); store.save(8, pcm); store.save(9, ShortArray(0))
        assertTrue(store.has(7))
        assertFalse("nothing to keep", store.has(9))
        assertTrue(pcm.contentEquals(store.load(7)))
        assertEquals(setOf(7L, 8L), store.ids())
        store.retain(setOf(8))
        assertNull(store.load(7))
        assertEquals(setOf(8L), store.ids())
        store.clear()
        assertEquals(emptySet<Long>(), store.ids())
    }
}
