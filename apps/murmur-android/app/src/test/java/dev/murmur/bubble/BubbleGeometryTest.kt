package dev.murmur.bubble

import dev.murmur.bubble.BubbleButton.CANCEL
import dev.murmur.bubble.BubbleButton.COPY
import dev.murmur.bubble.BubbleButton.DISMISS
import dev.murmur.bubble.BubbleButton.DONE
import dev.murmur.bubble.BubbleButton.MAIN
import dev.murmur.bubble.BubbleButton.PASTE
import dev.murmur.session.DictationState
import dev.murmur.session.DictationState.ErrorKind
import org.junit.Assert.assertEquals
import org.junit.Test

class BubbleGeometryTest {
    private val idle = BubbleUi(DictationState.Idle)
    private val ready = BubbleUi(DictationState.Ready)
    private val readyCopy = BubbleUi(DictationState.Ready, showCopy = true)
    private val minimised = BubbleUi(DictationState.Ready, minimized = true)
    private val repair = BubbleUi(DictationState.Ready, repair = true, minimized = true)
    private val recTap = BubbleUi(DictationState.Recording(hold = false, startedAt = 0))
    private val recHold = BubbleUi(DictationState.Recording(hold = true, startedAt = 0))
    private val procTap = BubbleUi(DictationState.Processing(hold = false, since = 0))
    private val procHold = BubbleUi(DictationState.Processing(hold = true, since = 0))
    private val failedPaste = BubbleUi(DictationState.Error(ErrorKind.FailedToPaste("hi")))
    private val otherError = BubbleUi(DictationState.Error(ErrorKind.MicUnavailable))
    private val answer = BubbleUi(DictationState.Error(ErrorKind.CommandAnswer("It means hello.")))

    /** Width × height in dp at 100 % (the values the bubble had before "Bubble size"). */
    private val today = listOf(
        idle to (56f to 56f), ready to (56f to 56f), readyCopy to (120f to 56f), minimised to (36f to 36f),
        repair to (56f to 56f), recTap to (230f to 56f), recHold to (160f to 56f), procTap to (230f to 56f),
        procHold to (160f to 56f), failedPaste to (176f to 56f), otherError to (56f to 56f),
    )

    private fun size(u: BubbleUi) = BubbleGeometry.widthDp(u) to BubbleGeometry.heightDp(u)

    @Test fun everyStateAt100PercentIsTodaysSize() {
        for ((u, wh) in today) assertEquals(u.toString(), wh, size(u))
    }

    @Test fun everyStateScalesBothWays() {
        val at150 = listOf(84f to 84f, 84f to 84f, 180f to 84f, 54f to 54f, 84f to 84f, 345f to 84f, 240f to 84f, 345f to 84f, 240f to 84f, 264f to 84f, 84f to 84f)
        val at75 = listOf(42f to 42f, 42f to 42f, 90f to 42f, 27f to 27f, 42f to 42f, 172.5f to 42f, 120f to 42f, 172.5f to 42f, 120f to 42f, 132f to 42f, 42f to 42f)
        for ((i, e) in today.withIndex()) {
            assertEquals("150% ${e.first}", at150[i], size(e.first.copy(scale = 1.5f)))
            assertEquals("75% ${e.first}", at75[i], size(e.first.copy(scale = 0.75f)))
        }
    }

    // ---- "Shrink to a dot": the minimised look is drawn 16dp, the window (touch target) stays 36dp ----

    private fun drawn(u: BubbleUi) = BubbleGeometry.drawnSizeDp(u)

    @Test fun minimisedIsDrawnAsA16dpDotWhenShrinkToDotIsOn() {
        assertEquals(16f, drawn(minimised))
        assertEquals(24f, drawn(minimised.copy(scale = 1.5f)))
        assertEquals(12f, drawn(minimised.copy(scale = 0.75f)))
    }

    @Test fun minimisedIsDrawnAsTodays36dpCircleWhenShrinkToDotIsOff() {
        val circle = minimised.copy(dot = false)
        assertEquals(36f, drawn(circle))
        assertEquals(54f, drawn(circle.copy(scale = 1.5f)))
        assertEquals(27f, drawn(circle.copy(scale = 0.75f)))
    }

    @Test fun theDotKeepsThe36dpMinimisedTouchTarget() {
        for (scale in listOf(0.75f, 1f, 1.5f)) {
            val dot = minimised.copy(scale = scale)
            assertEquals("$scale", 36f * scale to 36f * scale, size(dot))
            assertEquals("$scale: same window as the circle", size(dot.copy(dot = false)), size(dot))
        }
    }

    @Test fun onlyTheMinimisedLookBecomesADot() {
        for (u in listOf(ready, readyCopy, repair, recTap, failedPaste, readyCopy.copy(minimized = true))) {
            assertEquals(u.toString(), BubbleGeometry.heightDp(u), drawn(u))
        }
    }

    // ---- hit-testing: density 1 so px == dp × scale ----

    private fun hit(u: BubbleUi, x: Float, y: Float = BubbleGeometry.heightDp(u) / 2, processingCancel: Boolean = true): BubbleButton {
        val w = BubbleGeometry.widthDp(u); val h = BubbleGeometry.heightDp(u)
        return BubbleGeometry.hit(BubbleGeometry.slots(u, w, h, { it * u.scale }, processingCancel), h, x, y)
    }

    @Test fun at100PercentButtonsSitWhereTheyAlwaysDid() {
        assertEquals(MAIN, hit(ready, 28f))
        assertEquals(MAIN, hit(readyCopy, 55f))
        assertEquals(COPY, hit(readyCopy, 64f))
        assertEquals(COPY, hit(readyCopy, 119f))
        assertEquals(CANCEL, hit(recTap, 55f))
        assertEquals(MAIN, hit(recTap, 115f)) // waveform
        assertEquals(DONE, hit(recTap, 174f))
        assertEquals(PASTE, hit(failedPaste, 30f))
        assertEquals(COPY, hit(failedPaste, 90f))
        assertEquals(DISMISS, hit(failedPaste, 150f))
        assertEquals(DISMISS, hit(otherError, 28f))
    }

    @Test fun aCommandsAnswerOffersCopyAndDismissButNoPaste() {
        assertEquals(116f to 56f, size(answer))
        assertEquals(COPY, hit(answer, 30f))
        assertEquals(DISMISS, hit(answer, 90f))
        val buttons = BubbleGeometry.slots(answer, 116f, 56f, { it }).map { it.button }
        assertEquals(listOf(COPY, DISMISS), buttons)
    }

    @Test fun at150PercentButtonsGrowWithTheBubble() {
        val s = 1.5f
        assertEquals(COPY, hit(readyCopy.copy(scale = s), 100f)) // 64·1.5 = 96 .. 180
        assertEquals(MAIN, hit(readyCopy.copy(scale = s), 80f))
        assertEquals(CANCEL, hit(recTap.copy(scale = s), 80f)) // ✕ is 84 wide
        assertEquals(MAIN, hit(recTap.copy(scale = s), 172f))
        assertEquals(DONE, hit(recTap.copy(scale = s), 262f)) // 345 − 84 = 261
        assertEquals(DONE, hit(recTap.copy(scale = s), 344f))
        assertEquals(PASTE, hit(failedPaste.copy(scale = s), 83f))
        assertEquals(COPY, hit(failedPaste.copy(scale = s), 91f)) // 60·1.5 = 90
        assertEquals(DISMISS, hit(failedPaste.copy(scale = s), 263f)) // 120·1.5 = 180 .. 264
        assertEquals(CANCEL, hit(procTap.copy(scale = s), 10f, y = 83f))
    }

    @Test fun at75PercentButtonsShrinkWithTheBubble() {
        val s = 0.75f
        assertEquals(MAIN, hit(readyCopy.copy(scale = s), 47f))
        assertEquals(COPY, hit(readyCopy.copy(scale = s), 48f)) // 64·0.75 = 48
        assertEquals(CANCEL, hit(recTap.copy(scale = s), 41f))
        assertEquals(MAIN, hit(recTap.copy(scale = s), 50f))
        assertEquals(DONE, hit(recTap.copy(scale = s), 131f)) // 172.5 − 42 = 130.5
        assertEquals(PASTE, hit(failedPaste.copy(scale = s), 41f)) // ✓ is 42 wide
        assertEquals(COPY, hit(failedPaste.copy(scale = s), 46f)) // 60·0.75 = 45
        assertEquals(DISMISS, hit(failedPaste.copy(scale = s), 91f)) // 120·0.75 = 90
    }

    @Test fun heldProcessingHasNoCancelUntilItShows() {
        assertEquals(MAIN, hit(procHold.copy(scale = 1.5f), 10f, processingCancel = false))
        assertEquals(CANCEL, hit(procHold.copy(scale = 1.5f), 10f, processingCancel = true))
    }

    // ---- Command Mode button (✎): right of the mic, before copy ----

    private val readyCommand = BubbleUi(DictationState.Ready, showCommand = true)

    @Test fun theCommandButtonSitsRightOfTheMicAndBeforeCopy() {
        assertEquals(120f to 56f, size(readyCommand))
        assertEquals(MAIN, hit(readyCommand, 28f))
        assertEquals(BubbleButton.COMMAND, hit(readyCommand, 64f))
        assertEquals(BubbleButton.COMMAND, hit(readyCommand, 119f))
        val both = readyCommand.copy(showCopy = true)
        assertEquals(184f to 56f, size(both))
        assertEquals(BubbleButton.COMMAND, hit(both, 100f))
        assertEquals(COPY, hit(both, 150f))
        assertEquals(BubbleButton.COMMAND, hit(readyCommand.copy(scale = 1.5f), 100f))
    }

    @Test fun theCommandButtonKeepsTheBubbleFullSizeAndLeavesRecordingAlone() {
        assertEquals(false, BubbleGeometry.isMinimisedLook(readyCommand.copy(minimized = true)))
        assertEquals(120f to 56f, size(readyCommand.copy(minimized = true)))
        val rec = BubbleUi(DictationState.Recording(hold = false, startedAt = 0, command = true), showCommand = true)
        assertEquals(230f to 56f, size(rec))
        assertEquals(CANCEL, hit(rec, 28f))
        assertEquals(DONE, hit(rec, 200f))
    }
}
