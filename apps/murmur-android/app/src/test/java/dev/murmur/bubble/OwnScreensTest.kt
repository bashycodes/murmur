package dev.murmur.bubble

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OwnScreensTest {
    private val home = Any()
    private val settings = Any()
    private val noteEditor = Any()

    @Test fun anyOwnScreenStartedIsInFront() {
        val o = OwnScreens()
        assertFalse(o.inFront.value)
        o.started(settings, surface = false, splitScreen = false)
        assertTrue(o.inFront.value)
        o.stopped(settings)
        assertFalse(o.inFront.value) // left the app: the bubble may come back
    }

    @Test fun movingBetweenOwnScreensNeverDropsOut() {
        val o = OwnScreens()
        o.started(home, surface = false, splitScreen = false)
        o.started(settings, surface = false, splitScreen = false) // the next page starts before the last stops
        o.stopped(home)
        assertTrue(o.inFront.value)
    }

    @Test fun aDictationSurfaceKeepsTheBubble() {
        val o = OwnScreens()
        o.started(home, surface = false, splitScreen = false)
        o.started(noteEditor, surface = true, splitScreen = false)
        o.stopped(home)
        assertFalse(o.inFront.value)
        o.started(settings, surface = false, splitScreen = false)
        o.stopped(noteEditor)
        assertTrue(o.inFront.value)
    }

    @Test fun aSplitScreenHalfDoesNotCount() {
        val o = OwnScreens()
        o.started(settings, surface = false, splitScreen = true)
        assertFalse(o.inFront.value)
        o.stopped(settings)
        assertFalse(o.inFront.value)
    }
}
