package dev.murmur.session

import dev.murmur.debug.InsertOutcome
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 2026-10-24: the tap-to-copy notification for a dictation that did not go in. */
class NotInsertedNoticeTest {
    private val transcript = "meet me at the station at noon"

    @Test fun namesWhyAndHowMuchButNeverTheText() {
        for (o in listOf(InsertOutcome.TARGET_CHANGED, InsertOutcome.TARGET_PROTECTED, InsertOutcome.PASTE_FAILED)) {
            val t = NotInsertedNotice.text(o, transcript)
            assertTrue(t, t.contains("7 words") && t.contains("Tap to copy"))
            assertFalse("lock-screen safe: $t", t.contains("station"))
        }
        assertTrue(NotInsertedNotice.text(InsertOutcome.TARGET_CHANGED, transcript).contains("switched fields"))
        assertTrue(NotInsertedNotice.text(InsertOutcome.TARGET_PROTECTED, transcript).contains("protected"))
        assertTrue(NotInsertedNotice.text(InsertOutcome.PASTE_FAILED, "one").contains("1 word)"))
    }

    @Test fun wordCount() {
        assertEquals(0, NotInsertedNotice.words("   "))
        assertEquals(3, NotInsertedNotice.words(" a  b\nc "))
    }
}
