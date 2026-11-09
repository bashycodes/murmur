package dev.murmur.insert

import kotlinx.coroutines.async
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InsertionCoordinatorTest {
    private val env = FakeEnv()
    private val clips = mutableListOf<String>()
    private val clipboard = object : ClipboardPort {
        override fun copySensitive(text: String) { clips += text }
    }

    @Test fun successDoesNotTouchClipboard() = runTest {
        val c = InsertionCoordinator(V3Inserter(env) { currentTime }, clipboard) { currentTime }
        assertTrue(c.insertTextAndVerify("hi").succeeded)
        assertTrue(clips.isEmpty())
    }

    @Test fun failureCopiesToSensitiveClipboard() = runTest {
        env.field.text = "existing"; env.field.commitSilentlyDropped = true
        val c = InsertionCoordinator(V3Inserter(env) { currentTime }, clipboard) { currentTime }
        val r = c.insertTextAndVerify("hello")
        assertFalse(r.succeeded)
        assertEquals(listOf("hello"), clips)
    }

    /** S16: text meant for a password / sensitive field never reaches the clipboard. */
    @Test fun excludedFieldIsNotCopied() = runTest {
        env.editorType = 0x81 // TEXT_PASSWORD
        val c = InsertionCoordinator(V3Inserter(env) { currentTime }, clipboard) { currentTime }
        val r = c.insertTextAndVerify("secret")
        assertEquals(InsertResult.FailureReason.EXCLUDED_FIELD, r.reason)
        assertTrue(clips.isEmpty())
    }

    @Test fun insertionsAreSerializedAndQueueTimeCountsAgainstDeadline() = runTest {
        // First insert burns its full verify window (never lands, non-empty field).
        env.field.text = "x"; env.field.commitSilentlyDropped = true
        val c = InsertionCoordinator(V3Inserter(env) { currentTime }, clipboard) { currentTime }
        val a = async { c.insertTextAndVerify("one") }
        val b = async { c.insertTextAndVerify("two") }
        a.await(); b.await()
        assertEquals("serialized: one commit each", listOf("one", "two"), env.field.commits)
        assertTrue(currentTime <= 2 * 3500)
    }

    @Test fun pendingCountVisibleDuringInsert() = runTest {
        val c = InsertionCoordinator(V3Inserter(env) { currentTime }, clipboard) { currentTime }
        assertEquals(0, c.pendingInsertions)
        val d = async { c.insertTextAndVerify("x") }
        testScheduler.runCurrent()
        assertEquals(1, c.pendingInsertions)
        d.await()
        assertEquals(0, c.pendingInsertions)
    }
}
