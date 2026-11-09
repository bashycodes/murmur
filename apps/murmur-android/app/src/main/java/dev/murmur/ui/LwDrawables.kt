package dev.murmur.ui

import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ColorFilter
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.graphics.drawable.StateListDrawable

/** Drawables of the design's controls, built from [Tokens] (no XML so the colours follow the palette). */
internal object LwDrawables {
    private val CHECKED = intArrayOf(android.R.attr.state_checked)

    /** `.a-switch` track: 52x32 dp pill; off = light track + 2 dp border, on = ink. */
    fun switchTrack(t: Tokens, density: Float): Drawable {
        fun pill(fill: Int, border: Int) = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = 16 * density
            setColor(fill); setStroke((2 * density).toInt().coerceAtLeast(1), border)
            setSize((52 * density).toInt(), (32 * density).toInt())
        }
        return StateListDrawable().apply {
            addState(CHECKED, pill(t.trackOn, t.trackOn))
            addState(intArrayOf(), pill(t.trackOff, t.trackOffBorder))
        }
    }

    /** `.a-switch::before` thumb: a 16 dp dot when off, a 22 dp dot when on. */
    fun switchThumb(t: Tokens, density: Float): Drawable = SwitchThumb(t, density)

    /**
     * Android's Switch makes the track 2 x the thumb's width, so the thumb is 26 dp wide (track 52 dp)
     * and draws its dot where the design puts it: centre 16 dp from the track's left edge when off,
     * 11 dp into the thumb (37 dp) when on.
     */
    private class SwitchThumb(private val t: Tokens, private val d: Float) : Drawable() {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private var on = false

        override fun getIntrinsicWidth() = (26 * d).toInt()
        override fun getIntrinsicHeight() = (32 * d).toInt()
        override fun isStateful() = true
        override fun onStateChange(state: IntArray): Boolean {
            val now = android.R.attr.state_checked in state
            if (now == on) return false
            on = now; invalidateSelf(); return true
        }

        override fun draw(canvas: Canvas) {
            val b = bounds
            paint.color = if (on) t.thumbOn else t.thumbOff
            val cx = b.left + (if (on) 11 else 16) * d
            canvas.drawCircle(cx, b.exactCenterY(), (if (on) 11 else 8) * d, paint)
        }

        override fun setAlpha(alpha: Int) { paint.alpha = alpha }
        override fun setColorFilter(colorFilter: ColorFilter?) { paint.colorFilter = colorFilter }
        @Deprecated("Deprecated in Java")
        override fun getOpacity() = PixelFormat.TRANSLUCENT
    }

    fun rounded(color: Int, radiusPx: Float, strokePx: Int = 0, strokeColor: Int = Color.TRANSPARENT) = GradientDrawable().apply {
        setColor(color); cornerRadius = radiusPx
        if (strokePx > 0) setStroke(strokePx, strokeColor)
    }

    fun circle(color: Int) = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(color) }

    /** Ripple in [ink] at 13 % over [content], masked to [content]'s shape (or the bounds). */
    fun ripple(ink: Int, content: Drawable?, mask: Drawable? = null) =
        RippleDrawable(ColorStateList.valueOf((ink and 0x00FFFFFF) or 0x22000000), content, mask ?: content ?: GradientDrawable().apply { setColor(Color.WHITE) })
}
