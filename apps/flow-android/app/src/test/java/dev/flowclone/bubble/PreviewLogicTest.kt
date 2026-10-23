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

    @Test fun engineBadgePrefixesTheLineWhileRecording() {
        assertEquals("[S] hello", PreviewContent.line(rec, "hello", null, true, "S")!!.text)
        assertEquals("[L•]", PreviewContent.line(rec, null, null, true, "L•")!!.text)
        assertEquals(null, PreviewContent.line(DictationState.Ready, null, null, true, "S"))
        assertEquals("hello", PreviewContent.line(rec, "hello", null, true, null)!!.text)
    }

    @Test fun liveRouteLabels() {
        assertEquals("via phone", RouteLabel.of(dev.flowclone.core.SessionRoute("local")))
        assertEquals("via server", RouteLabel.of(dev.flowclone.core.SessionRoute("remote", server = "100.64.0.10:7313")))
        assertEquals(
            "via OpenRouter (openai/gpt-transcribe)",
            RouteLabel.of(dev.flowclone.core.SessionRoute("cloud", provider = "openrouter-1", model = "openai/gpt-transcribe")) { if (it == "openrouter-1") "OpenRouter" else null },
        )
        assertEquals("via phone · server unreachable", RouteLabel.of(dev.flowclone.core.SessionRoute("local", fallback = "disconnected")))
        assertEquals(null, RouteLabel.of(null))
    }

    @Test fun routeLabelLeadsTheLineWhileRecordingAndProcessing() {
        assertEquals(PreviewContent.Line("via server", dim = true), PreviewContent.line(rec, null, null, true, badge = "S", route = "via server"))
        assertEquals(PreviewContent.Line("via server · ask not", dim = false), PreviewContent.line(rec, "ask not", null, true, route = "via server"))
        assertEquals(
            PreviewContent.Line("via phone · ask not", dim = true),
            PreviewContent.line(DictationState.Processing(false, 0), "ask not", null, true, route = "via phone"),
        )
        assertEquals("idle: no route", null, PreviewContent.line(DictationState.Ready, null, null, true, route = "via phone"))
    }

    @Test fun loadingPhoneModelLabel() {
        assertEquals("Loading phone model… · server unreachable", RouteLabel.of(dev.flowclone.core.SessionRoute("local", fallback = "connect_failed", loading = true)))
        assertEquals("Loading phone model… · paused after failures", RouteLabel.of(dev.flowclone.core.SessionRoute("local", fallback = "circuit_open", loading = true)))
    }
}
