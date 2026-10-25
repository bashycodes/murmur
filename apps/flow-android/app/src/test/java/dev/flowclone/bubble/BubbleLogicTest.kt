package dev.flowclone.bubble

import dev.flowclone.bubble.BubbleGesture.Action
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BubbleGestureTest {
    private var blocked = false
    private val g = BubbleGesture(touchSlopPx = 10f) { blocked }

    @Test fun quickReleaseIsTap() {
        g.down(0f, 0f)
        g.move(3f, 3f)
        assertEquals(listOf(Action.Tap), g.up(3f, 3f))
    }

    @Test fun longPressStartsHoldAndReleaseEndsIt() {
        g.down(0f, 0f)
        assertEquals(listOf(Action.HoldStart), g.longPressTimeout())
        assertEquals(listOf(Action.HoldEnd), g.up(0f, 0f))
    }

    @Test fun holdThenDragStillSubmitsOnRelease() {
        g.down(0f, 0f)
        g.longPressTimeout()
        assertEquals(listOf(Action.DragStart, Action.DragBy(20f, 0f)), g.move(20f, 0f))
        assertEquals(listOf(Action.HoldEnd, Action.DragEnd), g.up(20f, 0f))
    }

    @Test fun movePastSlopBeforeTimeoutIsDragNotTapOrHold() {
        g.down(0f, 0f)
        assertEquals(listOf(Action.DragStart, Action.DragBy(15f, 0f)), g.move(15f, 0f))
        assertEquals(listOf(Action.DragBy(5f, 5f)), g.move(20f, 5f))
        assertTrue("timeout after drag is ignored", g.longPressTimeout().isEmpty())
        assertEquals(listOf(Action.DragEnd), g.up(20f, 5f))
    }

    @Test fun blockedStateOnlyDragsAndReportsTapLocation() {
        blocked = true
        g.down(5f, 6f)
        assertTrue(g.longPressTimeout().isEmpty())
        assertEquals(listOf(Action.BlockedTap(5f, 6f)), g.up(5f, 6f))
    }

    @Test fun blockedStateCanStillBeDragged() {
        blocked = true
        g.down(0f, 0f)
        assertEquals(listOf(Action.DragStart, Action.DragBy(0f, 30f)), g.move(0f, 30f))
        assertEquals(listOf(Action.DragEnd), g.up(0f, 30f))
    }

    @Test fun upWithoutDownIsIgnored() = assertTrue(g.up(0f, 0f).isEmpty())
}

class BubblePositionerTest {
    // 1080x2400 px at density 2.75 (≈ 393dp wide)
    private val store = object : BubblePositioner.Store {
        var saved: BubblePositioner.Saved? = null
        override fun load() = saved
        override fun save(s: BubblePositioner.Saved) { saved = s }
    }
    private val p = BubblePositioner(screenW = 1080, screenH = 2400, density = 2.75f, store = store)
    private val w = (56 * 2.75f).toInt()

    @Test fun defaultsToRightAnchorAndYIndex35() {
        val pos = p.position(w, w)
        assertEquals(1080 - w - (20 * 2.75f).toInt(), pos.x)
        assertEquals(p.yForIndex(35, w), pos.y)
    }

    @Test fun releaseOnLeftHalfSnapsLeft() {
        val pos = p.snap(x = 100, y = 1000, bubbleW = w, bubbleH = w)
        assertEquals((20 * 2.75f).toInt(), pos.x)
        assertEquals(BubblePositioner.Side.LEFT, store.saved!!.side)
    }

    @Test fun releaseOnRightHalfSnapsRight() {
        val pos = p.snap(x = 900, y = 1000, bubbleW = w, bubbleH = w)
        assertEquals(1080 - w - (20 * 2.75f).toInt(), pos.x)
    }

    @Test fun ySnapsToNearestOf100AnchorsWithinMargins() {
        val top = p.snap(x = 900, y = -500, bubbleW = w, bubbleH = w)
        assertEquals((56 * 2.75f).toInt(), top.y)
        assertEquals(0, store.saved!!.yIndex)
        val bottom = p.snap(x = 900, y = 99_999, bubbleW = w, bubbleH = w)
        assertEquals(99, store.saved!!.yIndex)
        assertEquals(2400 - (56 * 2.75f).toInt() - w, bottom.y)
    }

    @Test fun positionIsRememberedAcrossInstances() {
        p.snap(x = 10, y = 300, bubbleW = w, bubbleH = w)
        val again = BubblePositioner(1080, 2400, 2.75f, store).position(w, w)
        assertEquals((20 * 2.75f).toInt(), again.x)
    }

    @Test fun rightDockedBubbleGrowsLeftwards() {
        val small = p.position(w, w)
        val wide = p.position(230 * 3, w)
        assertEquals(small.x + w - 230 * 3, wide.x)
    }

    @Test fun marginsScaleWithBubbleSize() {
        val big = BubblePositioner(1080, 2400, 2.75f, store, scale = 1.5f)
        val bw = (84 * 2.75f).toInt()
        assertEquals((30 * 2.75f).toInt(), big.xFor(BubblePositioner.Side.LEFT, bw))
        assertEquals(1080 - bw - (30 * 2.75f).toInt(), big.xFor(BubblePositioner.Side.RIGHT, bw))
        assertEquals((84 * 2.75f).toInt(), big.yForIndex(0, bw))
        assertEquals(2400 - (84 * 2.75f).toInt() - bw, big.yForIndex(99, bw))
        val small = BubblePositioner(1080, 2400, 2.75f, store, scale = 0.75f)
        assertEquals((15 * 2.75f).toInt(), small.xFor(BubblePositioner.Side.LEFT, bw))
    }

    @Test fun snappedBubbleAt150PercentIsFullyOnScreenInEveryState() {
        // A small 360dp phone at density 3: the 345dp recording row is wider than screen − 30dp.
        val sw = 1080; val sh = 1920; val d = 3f
        val big = BubblePositioner(sw, sh, d, store, scale = 1.5f)
        for (wDp in listOf(36f, 56f, 120f, 160f, 176f, 230f)) for (hDp in listOf(36f, 56f)) {
            val bw = (wDp * 1.5f * d).toInt(); val bh = (hDp * 1.5f * d).toInt()
            for ((x, y) in listOf(-500 to -500, 99_999 to 99_999, 10 to 900, 900 to 10)) {
                val pos = big.snap(x, y, bw, bh)
                assertTrue("$wDp×$hDp from $x,$y → $pos", pos.x >= 0 && pos.x + bw <= sw && pos.y >= 0 && pos.y + bh <= sh)
            }
        }
    }

    @Test fun savedAnchorSurvivesASizeChange() {
        p.snap(x = 10, y = 1200, bubbleW = w, bubbleH = w)
        val saved = store.saved!!
        val big = BubblePositioner(1080, 2400, 2.75f, store, scale = 1.5f)
        val bw = (84 * 2.75f).toInt()
        val pos = big.position(bw, bw)
        assertEquals(big.xFor(BubblePositioner.Side.LEFT, bw), pos.x)
        assertEquals(big.yForIndex(saved.yIndex, bw), pos.y)
        assertEquals(saved, store.saved)
    }

    @Test fun clampKeepsDragOnScreen() {
        val c = p.clampDrag(x = -50, y = 5000, bubbleW = w, bubbleH = w)
        assertEquals(0, c.x)
        assertEquals(2400 - w, c.y)
    }
}

class WaveformTest {
    @Test fun tenBarsWithSymmetricScale() = assertEquals(10, Waveform.SCALE.size)

    @Test fun silentLevelGivesMinimumHeights() {
        val h = Waveform.barHeightsDp(tMs = 123, level = 0f)
        assertTrue(h.all { it == 3f })
    }

    @Test fun fullLevelCentreBarsReachBaseHeightAtPeakKeyframe() {
        // kf(0.4) = 1 → bar 4 (scale 1) at t where (t + 0.1·4) mod 1 = 0.4 → t = 0
        val h = Waveform.barHeightsDp(tMs = 0, level = 1f)
        assertEquals(32f, h[4], 0.01f)
    }

    @Test fun keyframesInterpolateLinearly() {
        assertEquals(0.57f, Waveform.keyframe(0f), 1e-4f)
        assertEquals(0.655f, Waveform.keyframe(0.1f), 1e-3f)
        assertEquals(1f, Waveform.keyframe(0.4f), 1e-4f)
        assertEquals(0.57f, Waveform.keyframe(1f), 1e-4f)
    }

    @Test fun smoothingMoves15PercentPerFrame() {
        val s = Waveform.Smoother()
        assertEquals(0.15f, s.step(1f), 1e-5f)
        assertEquals(0.2775f, s.step(1f), 1e-4f)
    }
}
