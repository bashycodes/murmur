package dev.murmur.ui

import org.junit.Assert.assertEquals
import org.junit.Test

/** The focused field vs. the window left above the keyboard (scroll-view coordinates). */
class ScrollRevealTest {
    // Window 0..1000 tall, the keyboard pads the bottom 600 px: visible 40..400.
    private fun d(top: Int, bottom: Int) = ScrollReveal.delta(top, bottom, visibleTop = 40, visibleBottom = 400)

    @Test fun fieldUnderTheKeyboardScrollsUpJustEnough() = assertEquals(300, d(600, 700))

    @Test fun visibleFieldDoesNotMove() = assertEquals(0, d(100, 300))

    @Test fun fieldAboveTheWindowScrollsBack() = assertEquals(-60, d(-20, 80))

    @Test fun tallFieldAlignsItsTop() = assertEquals(460, d(500, 1200))
}
