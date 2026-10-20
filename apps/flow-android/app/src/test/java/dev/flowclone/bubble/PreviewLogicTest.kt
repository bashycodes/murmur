package dev.flowclone.bubble

import dev.flowclone.session.DictationHint
import dev.flowclone.session.DictationState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PreviewLogicTest {
    private val rec = DictationState.Recording(false, 0)

    @Test fun partialWhileRecordingDimWhileProcessing() {
        assertEquals(PreviewContent.Line("hello there", false), PreviewContent.line(rec, " hello  there ", null, true))
        assertEquals(PreviewContent.Line("hello", true), PreviewContent.line(DictationState.Processing(false, 0), "hello", null, true))
        assertNull(PreviewContent.line(DictationState.Ready, "hello", null, true))
        assertNull(PreviewContent.line(rec, "hello", null, enabled = false))
    }

    @Test fun hintsBeatPartials() {
        assertEquals(PreviewContent.CANT_HEAR, PreviewContent.line(rec, "x", DictationHint.NoSpeechYet, true)!!.text)
        assertEquals(PreviewContent.NO_SPEECH, PreviewContent.line(DictationState.Ready, null, DictationHint.NoSpeechDetected, false)!!.text)
    }

    @Test fun tailKeepsNewestWords() {
        val long = (1..100).joinToString(" ") { "word$it" }
        val t = PreviewContent.tail(long, 40)
        assertTrue(t.startsWith("…word"))
        assertTrue(t.endsWith("word100"))
        assertTrue(t.length <= 40)
    }

    @Test fun placementAboveRightDocked() {
        val p = PreviewPlacement.place(bubbleX = 900, bubbleY = 1000, bubbleW = 160, bubbleH = 168, previewH = 100, screenW = 1080, gapPx = 24, topMarginPx = 100)
        assertEquals(PreviewPlacement.Place(true, 20, 876), p)
    }

    @Test fun placementBelowWhenNoRoomLeftDocked() {
        val p = PreviewPlacement.place(bubbleX = 60, bubbleY = 150, bubbleW = 168, bubbleH = 168, previewH = 100, screenW = 1080, gapPx = 24, topMarginPx = 100)
        assertEquals(PreviewPlacement.Place(false, 60, 342), p)
    }
}
