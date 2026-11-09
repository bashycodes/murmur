package dev.flowclone.insert

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Review S16: a transcript never stays on the clipboard. */
@OptIn(ExperimentalCoroutinesApi::class)
class ClipboardGuardTest {
    private class FakeClip : ClipboardGuard.Port {
        var text: String? = "user's own clip"
        var marker: String? = null
        var sensitive = false
        var readable = true
        var foreignChangeAt = -1L
        var t = 0L
        override fun current() = if (!readable || text == null) null else ClipboardGuard.Clip(marker, text)
        override fun setSensitive(text: String, marker: String) { this.text = text; this.marker = marker; sensitive = true }
        override fun restore(raw: Any) { text = raw as String; marker = null; sensitive = false }
        override fun clear() { text = null; marker = null; sensitive = false }
        override fun changedSince(since: Long) = foreignChangeAt >= since
        override fun now() = t
        fun userCopies(s: String) { text = s; marker = null; sensitive = false; foreignChangeAt = ++t }
    }

    private val clip = FakeClip()
    private fun TestScope.guard() = ClipboardGuard(clip, backgroundScope, holdMs = 60_000)

    @Test fun copiedSensitiveThenTheUsersClipComesBackAfterAMinute() = runTest {
        val g = guard()
        g.copy("my transcript")
        assertEquals("my transcript", clip.text)
        assertTrue(clip.sensitive)
        advanceTimeBy(59_000); runCurrent()
        assertEquals("my transcript", clip.text)
        advanceTimeBy(2_000); runCurrent()
        assertEquals("user's own clip", clip.text)
        assertFalse(g.holding)
    }

    @Test fun clearedWhenThereWasNothingToRestore() = runTest {
        clip.text = null
        val g = guard()
        g.copy("t")
        advanceTimeBy(61_000); runCurrent()
        assertNull(clip.text)
    }

    @Test fun aLaterInsertTakesItOffAtOnce() = runTest {
        val g = guard()
        g.copy("t")
        g.onInserted()
        assertEquals("user's own clip", clip.text)
    }

    @Test fun neverOverwritesWhatTheUserCopiedMeanwhile() = runTest {
        val g = guard()
        g.copy("t")
        clip.userCopies("new user clip")
        advanceTimeBy(61_000); runCurrent()
        assertEquals("new user clip", clip.text)
    }

    @Test fun unreadableClipboardUsesTheChangeListener() = runTest {
        val g = guard()
        g.copy("t")
        clip.readable = false
        advanceTimeBy(61_000); runCurrent()
        assertEquals("no foreign change: ours, restored", "user's own clip", clip.text)
        g.copy("t2"); clip.readable = false
        clip.userCopies("mine")
        advanceTimeBy(61_000); runCurrent()
        assertEquals("mine", clip.text)
    }

    @Test fun secondCopyKeepsTheOriginalUserClipAndRestartsTheTimer() = runTest {
        val g = guard()
        g.copy("a")
        advanceTimeBy(30_000)
        g.copy("b")
        advanceTimeBy(40_000); runCurrent()
        assertEquals("b", clip.text)
        advanceTimeBy(21_000); runCurrent()
        assertEquals("user's own clip", clip.text)
    }
}
