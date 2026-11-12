package dev.murmur.ui

import android.graphics.Rect
import android.view.View
import android.widget.ScrollView

/**
 * Keeps the focused field of a [DevUi.screen] above the keyboard. The screen pads its
 * ScrollView by the IME inset with `clipToPadding = false`, and ScrollView's own
 * focus scrolling ignores that padding, so a field low on the page (the Server token)
 * stayed under the keyboard.
 */
object ScrollReveal {
    /**
     * How far to scroll (positive = down) so `[top, bottom]` fits in the visible window
     * `[visibleTop, visibleBottom]`, all in the scroll view's content coordinates. A rect taller
     * than the window is aligned to its top; one already inside does not move.
     */
    fun delta(top: Int, bottom: Int, visibleTop: Int, visibleBottom: Int): Int = when {
        bottom - top > visibleBottom - visibleTop -> top - visibleTop
        bottom > visibleBottom -> bottom - visibleBottom
        top < visibleTop -> top - visibleTop
        else -> 0
    }

    /** Scrolls [scroll] so its focused descendant, plus [belowPx] (its help / error line), is visible. */
    fun focused(scroll: ScrollView, belowPx: Int) {
        val d = focusedDelta(scroll, belowPx)
        if (d != 0) scroll.smoothScrollBy(0, d)
    }

    /**
     * The [delta] [focused] scrolls by. `offsetDescendantRectToMyCoords` gives the field in
     * content coordinates (scrollY not subtracted), so the visible window is shifted by
     * scrollY too, as ScrollView.scrollToChild does.
     */
    fun focusedDelta(scroll: ScrollView, belowPx: Int): Int {
        val f = scroll.findFocus() ?: return 0
        if (f === scroll) return 0
        val r = Rect(0, 0, f.width, f.height)
        scroll.offsetDescendantRectToMyCoords(f, r)
        val y = scroll.scrollY
        return delta(r.top, r.bottom + belowPx, y + scroll.paddingTop, y + scroll.height - scroll.paddingBottom)
    }

    /** Reveals on each focus change and after each layout whose bottom padding (the keyboard) changed. */
    fun install(scroll: ScrollView, belowPx: Int) {
        val reveal = Runnable { focused(scroll, belowPx) }
        var lastBottom = scroll.paddingBottom
        scroll.viewTreeObserver.addOnGlobalFocusChangeListener { _, now -> if (now != null) scroll.post(reveal) }
        scroll.addOnLayoutChangeListener { v: View, _, _, _, _, _, _, _, _ ->
            if (v.paddingBottom != lastBottom) { lastBottom = v.paddingBottom; scroll.post(reveal) }
        }
    }
}
