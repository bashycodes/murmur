package dev.murmur.bubble

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OwnScreensTest {
    private val home = Any()
    private val settings = Any()
    private val noteEditor = Any()
    private val page = OwnScreens.Screen(dictation = false, split = false)
    private val splitPage = OwnScreens.Screen(dictation = false, split = true)
    private val editor = OwnScreens.Screen(dictation = true, split = false)

    private fun OwnScreens.hides() = front.value.hidesBubble

    @Test fun anyOwnScreenStartedHidesTheBubble() {
        val o = OwnScreens()
        assertFalse(o.hides())
        o.started(settings, page)
        assertTrue(o.hides())
        o.stopped(settings, recreating = false)
        assertFalse(o.hides()) // left the app: the bubble may come back
    }

    @Test fun movingBetweenOwnScreensNeverDropsOut() {
        val o = OwnScreens()
        o.started(home, page)
        o.started(settings, page) // the next page starts before the last stops
        o.stopped(home, recreating = false)
        assertTrue(o.hides())
    }

    @Test fun aDictationScreenKeepsTheBubbleAndOpensTheGate() {
        val o = OwnScreens()
        o.started(home, page)
        o.started(noteEditor, editor)
        o.stopped(home, recreating = false)
        assertFalse(o.hides())
        assertTrue(o.front.value.dictationScreen)
        o.started(settings, page)
        o.stopped(noteEditor, recreating = false)
        assertTrue(o.hides())
        assertFalse(o.front.value.dictationScreen)
    }

    @Test fun aSplitScreenHalfDoesNotHideTheBubble() {
        val o = OwnScreens()
        o.started(settings, splitPage)
        assertFalse(o.hides())
        o.started(noteEditor, OwnScreens.Screen(dictation = true, split = true))
        assertTrue(o.front.value.dictationScreen) // the gate still opens for a note editor in split screen
    }

    @Test fun enteringOrLeavingSplitScreenInPlaceIsReRead() {
        val o = OwnScreens()
        o.started(settings, page)
        o.refresh(settings, splitPage) // full screen → split: the other half gets its bubble
        assertFalse(o.hides())
        o.refresh(settings, page) // split → maximised: no bubble over Settings
        assertTrue(o.hides())
    }

    @Test fun refreshAfterStopIsIgnored() {
        val o = OwnScreens()
        o.started(settings, page)
        o.stopped(settings, recreating = false)
        o.refresh(settings, page) // a late config change of a stopped screen
        assertFalse(o.hides())
    }

    @Test fun rotationDoesNotFlashTheBubble() {
        val o = OwnScreens()
        val old = Any(); val recreated = Any()
        val seen = mutableListOf<Boolean>()
        o.started(old, page); seen += o.hides()
        o.stopped(old, recreating = true); seen += o.hides()
        o.started(recreated, page); seen += o.hides()
        assertTrue(seen.toString(), seen.all { it })
        o.stopped(recreated, recreating = false)
        assertFalse(o.hides())
    }
}
