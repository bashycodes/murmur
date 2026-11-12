package dev.murmur.ui

import android.app.Activity
import android.view.View
import android.view.View.MeasureSpec
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/** [ScrollReveal.focusedDelta] on a real ScrollView, including one already scrolled down. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ScrollRevealFocusTest {
    // ScrollView 1000 px tall, keyboard pads the bottom 600: 360 px visible from 40 below the top.
    // Content starts at the 40 px top padding: 900 px above the field, so the field is at
    // 940..1040 in content coordinates, then 2000 px below it.
    private fun page(scrollY: Int): ScrollView {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val field = EditText(activity).apply { isFocusableInTouchMode = true }
        val column = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            addView(View(activity), LinearLayout.LayoutParams(1000, 900))
            addView(field, LinearLayout.LayoutParams(1000, 100))
            addView(View(activity), LinearLayout.LayoutParams(1000, 2000))
        }
        val scroll = ScrollView(activity).apply {
            clipToPadding = false
            setPadding(0, 40, 0, 600)
            addView(column)
        }
        activity.setContentView(scroll)
        scroll.measure(
            MeasureSpec.makeMeasureSpec(1000, MeasureSpec.EXACTLY),
            MeasureSpec.makeMeasureSpec(1000, MeasureSpec.EXACTLY),
        )
        scroll.layout(0, 0, 1000, 1000)
        assertTrue(field.requestFocus()) // ScrollView's own focus scroll runs here, so scroll after it
        scroll.scrollTo(0, scrollY)
        assertEquals(scrollY, scroll.scrollY)
        return scroll
    }

    @Test fun unscrolledPageScrollsFieldAboveKeyboard() =
        assertEquals(640, ScrollReveal.focusedDelta(page(0), 0)) // window 40..400

    @Test fun scrolledPageScrollsOnlyTheRemainder() =
        assertEquals(140, ScrollReveal.focusedDelta(page(500), 0)) // window 540..900

    @Test fun scrolledPageCountsTheHelpLine() =
        assertEquals(188, ScrollReveal.focusedDelta(page(500), 48))

    @Test fun alreadyVisibleFieldDoesNotMove() =
        assertEquals(0, ScrollReveal.focusedDelta(page(700), 0)) // window 740..1100
}
