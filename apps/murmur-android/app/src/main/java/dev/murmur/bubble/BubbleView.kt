package dev.murmur.bubble

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.drawable.Drawable
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import dev.murmur.R
import dev.murmur.session.DictationState
import dev.murmur.session.DictationState.ErrorKind

/**
 * Canvas-drawn bubble: circle (Ready), minimised dot or small circle, recording row X·waveform·✓ (tap mode),
 * mic+waveform (hold mode), paste/copy/dismiss (FailedToPaste), pulsing … (model still loading), alert (other errors).
 * Gestures go through [BubbleGesture]; the host decides what each action means.
 */
@SuppressLint("ViewConstructor")
class BubbleView(context: Context, private val listener: Listener) : View(context) {
    interface Listener {
        fun onTap(button: BubbleButton)
        fun onHoldStart()
        fun onHoldEnd()
        fun onDragStart()
        fun onDragBy(dx: Float, dy: Float)
        fun onDragEnd()
    }

    private val d = resources.displayMetrics.density
    /** Design dp → px at the current "Bubble size" (base values × [BubbleUi.scale]). */
    private fun dp(v: Float) = v * d * ui.scale

    var ui = BubbleUi(); private set
    private var processingSince = 0L
    private val smoother = Waveform.Smoother()
    /** The buttons as last drawn, for hit-testing a touch. */
    private var slots = emptyList<BubbleGeometry.Slot>()
    private val mic: Drawable = context.getDrawable(R.drawable.ic_murmur)!!.mutate().apply { setTint(BubbleColors.onSurface) }

    // Design: dark vast-950 pill, light sand-50 glyphs and bars, red only for errors (BubbleColors).
    private val pill = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = BubbleColors.surface
        setShadowLayer(BubbleColors.SHADOW_RADIUS_DP * d, 0f, BubbleColors.SHADOW_DY_DP * d, BubbleColors.shadow)
    }
    private val raised = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.raised }
    private val primary = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.onSurface }
    private val danger = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.error }
    private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.onSurface; style = Paint.Style.STROKE }
    private val bar = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.onSurface }
    private val glyph = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = BubbleColors.onSurface; textAlign = Paint.Align.CENTER; textSize = 22f * d; isFakeBoldText = true
    }
    /** ✕ / ✓ as the design's 24-unit stroked icons (lw-ico: round caps and joins). */
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = BubbleColors.onSurface; style = Paint.Style.STROKE
        strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND
    }
    private val iconPath = Path()

    private val gesture = BubbleGesture(ViewConfiguration.get(context).scaledTouchSlop.toFloat()) {
        when (ui.state) {
            is DictationState.Recording, is DictationState.Processing, is DictationState.Error -> true
            else -> false
        }
    }
    private val longPress = Runnable { gesture.longPressTimeout().forEach(::dispatch) }

    fun update(newUi: BubbleUi) {
        if (newUi.state is DictationState.Processing && ui.state !is DictationState.Processing) processingSince = SystemClock.uptimeMillis()
        val resize = BubbleGeometry.widthDp(newUi) != BubbleGeometry.widthDp(ui) || BubbleGeometry.heightDp(newUi) != BubbleGeometry.heightDp(ui)
        ui = newUi
        glyph.textSize = dp(22f)
        if (resize) requestLayout()
        invalidate()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        // The geometry sizes are already scaled: density only (same px as the overlay's window maths).
        setMeasuredDimension((BubbleGeometry.widthDp(ui) * d).toInt(), (BubbleGeometry.heightDp(ui) * d).toInt())
    }

    override fun onDraw(canvas: Canvas) {
        val h = height.toFloat(); val r = h / 2
        val s = ui.state
        val showCancel = s !is DictationState.Processing || !s.hold || SystemClock.uptimeMillis() - processingSince > 3000
        // Button positions come from the geometry, so drawing and hit-testing never disagree.
        slots = BubbleGeometry.slots(ui, width.toFloat(), h, ::dp, processingCancel = showCancel)
        if (BubbleGeometry.isDot(ui)) {
            // "Shrink to a dot": a dark dot with a light ring (reads on light and dark apps), centred
            // in the 36dp window, which stays the touch target.
            val rr = BubbleGeometry.drawnSizeDp(ui) * d / 2
            canvas.drawCircle(width / 2f, h / 2, rr, pill)
            ring.strokeWidth = 1.5f * d * ui.scale
            canvas.drawCircle(width / 2f, h / 2, rr - ring.strokeWidth / 2, ring)
            return
        }
        canvas.drawRoundRect(RectF(0f, 0f, width.toFloat(), h), r, r, pill)
        when (s) {
            DictationState.Idle, DictationState.Ready -> if (ui.repair) {
                val pulse = 0.6f + 0.4f * kotlin.math.abs(((SystemClock.uptimeMillis() % 2000) / 1000f) - 1f)
                danger.alpha = (255 * pulse).toInt()
                circleButton(canvas, danger, BubbleButton.MAIN) { cx, cy -> drawGlyph(canvas, "!", cx, cy) }
                danger.alpha = 255
                postInvalidateOnAnimation()
            } else if (BubbleGeometry.isMinimisedLook(ui)) {
                circleButton(canvas, null, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy, 9f) }
            } else {
                circleButton(canvas, raised, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy) }
                if (ui.showCommand) circleButton(canvas, null, BubbleButton.COMMAND) { cx, cy -> drawGlyph(canvas, "✎", cx, cy) }
                if (ui.showCopy) circleButton(canvas, null, BubbleButton.COPY) { cx, cy -> drawGlyph(canvas, "⧉", cx, cy) }
            }
            is DictationState.Recording -> {
                val level = smoother.step(ui.level)
                if (s.hold) {
                    val pulse = 0.6f + 0.4f * kotlin.math.abs(((SystemClock.uptimeMillis() % 2000) / 1000f) - 1f)
                    raised.alpha = (255 * pulse).toInt()
                    circleButton(canvas, raised, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy) }
                    raised.alpha = 255
                    drawWave(canvas, dp(56f), width - dp(56f), level, 1f)
                } else {
                    circleButton(canvas, null, BubbleButton.CANCEL) { cx, cy -> drawCross(canvas, cx, cy, onSurface = true) }
                    drawWave(canvas, dp(62f), width - dp(62f), level, 1f)
                    circleButton(canvas, null, BubbleButton.DONE) { cx, cy -> drawCheck(canvas, cx, cy, onSurface = true) }
                }
                postInvalidateOnAnimation()
            }
            is DictationState.Processing -> {
                if (showCancel) circleButton(canvas, null, BubbleButton.CANCEL) { cx, cy -> drawCross(canvas, cx, cy, onSurface = true) }
                drawWave(canvas, dp(62f), width - dp(62f), 0.6f, 0.33f)
                postInvalidateOnAnimation()
            }
            is DictationState.Error -> when (s.kind) {
                is ErrorKind.FailedToPaste -> {
                    // Paste here (the primary action: light disc, dark ✓) · Copy (2026-10-24: never lose a dictation) · Dismiss.
                    circleButton(canvas, primary, BubbleButton.PASTE) { cx, cy -> drawCheck(canvas, cx, cy, onSurface = false) }
                    circleButton(canvas, null, BubbleButton.COPY) { cx, cy -> drawGlyph(canvas, "⧉", cx, cy) }
                    circleButton(canvas, null, BubbleButton.DISMISS) { cx, cy -> drawCross(canvas, cx, cy, onSurface = true) }
                }
                is ErrorKind.EngineLoading -> {
                    // Still loading (not an error): a pulsing neutral disc with "…"; a tap checks again.
                    val pulse = 0.6f + 0.4f * kotlin.math.abs(((SystemClock.uptimeMillis() % 2000) / 1000f) - 1f)
                    raised.alpha = (255 * pulse).toInt()
                    circleButton(canvas, raised, BubbleButton.DISMISS) { cx, cy -> drawGlyph(canvas, "…", cx, cy) }
                    raised.alpha = 255
                    postInvalidateOnAnimation()
                }
                else -> circleButton(canvas, danger, BubbleButton.DISMISS) { cx, cy -> drawGlyph(canvas, "!", cx, cy) }
            }
        }
    }

    /** Draws [b] in its [slots] square (absent = not shown in this state); a null [paint] = no disc, glyph on the pill. */
    private inline fun circleButton(c: Canvas, paint: Paint?, b: BubbleButton, content: (Float, Float) -> Unit) {
        val slot = slots.firstOrNull { it.button == b } ?: return
        val h = height.toFloat()
        val inset = if (h < dp(40f)) dp(2f) else dp(4f)
        val rect = RectF(slot.left, 0f, slot.right, h)
        if (paint != null) c.drawCircle(rect.centerX(), rect.centerY(), h / 2 - inset, paint)
        content(rect.centerX(), rect.centerY())
    }

    private fun drawMic(c: Canvas, cx: Float, cy: Float, halfDp: Float = 14f) {
        val half = dp(halfDp).toInt()
        mic.setBounds((cx - half).toInt(), (cy - half).toInt(), (cx + half).toInt(), (cy + half).toInt())
        mic.draw(c)
    }

    /** ✕ from the design: M6 6l12 12M18 6L6 18 in a 22dp box. */
    private fun drawCross(c: Canvas, cx: Float, cy: Float, onSurface: Boolean) =
        drawIcon(c, cx, cy, 22f, onSurface, CROSS)

    /** ✓ from the design: M5 12.5l4.5 4.5L19 7 in a 24dp box. */
    private fun drawCheck(c: Canvas, cx: Float, cy: Float, onSurface: Boolean) =
        drawIcon(c, cx, cy, 24f, onSurface, CHECK)

    /**
     * Strokes [lines] (polylines of x,y pairs in the icon's 24-unit viewBox, 1.5-unit lw-ico stroke
     * thickened to 2 for the small dark pill) scaled into a [boxDp] square centred on ([cx], [cy]):
     * light on the pill, dark on a light disc.
     */
    private fun drawIcon(c: Canvas, cx: Float, cy: Float, boxDp: Float, onSurface: Boolean, lines: List<FloatArray>) {
        val unit = dp(boxDp) / 24f
        val left = cx - dp(boxDp) / 2; val top = cy - dp(boxDp) / 2
        iconPath.reset()
        for (pl in lines) {
            iconPath.moveTo(left + pl[0] * unit, top + pl[1] * unit)
            for (i in 2 until pl.size step 2) iconPath.lineTo(left + pl[i] * unit, top + pl[i + 1] * unit)
        }
        stroke.color = if (onSurface) BubbleColors.onSurface else BubbleColors.surface
        stroke.strokeWidth = 2f * unit
        c.drawPath(iconPath, stroke)
    }

    private companion object {
        val CROSS = listOf(floatArrayOf(6f, 6f, 18f, 18f), floatArrayOf(18f, 6f, 6f, 18f))
        val CHECK = listOf(floatArrayOf(5f, 12.5f, 9.5f, 17f, 19f, 7f))
    }

    private fun drawGlyph(c: Canvas, g: String, cx: Float, cy: Float) {
        c.drawText(g, cx, cy - (glyph.descent() + glyph.ascent()) / 2, glyph)
    }

    private fun drawWave(c: Canvas, left: Float, right: Float, level: Float, alpha: Float) {
        val heights = Waveform.barHeightsDp(SystemClock.uptimeMillis(), level)
        val total = heights.size * Waveform.BAR_WIDTH_DP + (heights.size - 1) * Waveform.BAR_GAP_DP
        var x = (left + right) / 2 - dp(total) / 2
        val cy = height / 2f
        bar.alpha = (255 * alpha).toInt()
        for (hDp in heights) {
            val hh = dp(hDp) / 2
            c.drawRoundRect(RectF(x, cy - hh, x + dp(Waveform.BAR_WIDTH_DP), cy + hh), dp(1.5f), dp(1.5f), bar)
            x += dp(Waveform.BAR_WIDTH_DP + Waveform.BAR_GAP_DP)
        }
    }

    private fun hit(x: Float, y: Float): BubbleButton = BubbleGeometry.hit(slots, height.toFloat(), x, y)

    private var lastDownX = 0f; private var lastDownY = 0f

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                lastDownX = e.x; lastDownY = e.y
                gesture.down(e.rawX, e.rawY)
                postDelayed(longPress, ViewConfiguration.getLongPressTimeout().toLong())
            }
            MotionEvent.ACTION_MOVE -> gesture.move(e.rawX, e.rawY).forEach(::dispatch)
            MotionEvent.ACTION_UP -> { removeCallbacks(longPress); gesture.up(e.rawX, e.rawY).forEach(::dispatch) }
            MotionEvent.ACTION_CANCEL -> { removeCallbacks(longPress); gesture.cancel(); listener.onDragEnd() }
        }
        return true
    }

    private fun dispatch(a: BubbleGesture.Action) {
        when (a) {
            BubbleGesture.Action.Tap -> listener.onTap(hit(lastDownX, lastDownY))
            is BubbleGesture.Action.BlockedTap -> listener.onTap(hit(lastDownX, lastDownY))
            BubbleGesture.Action.HoldStart -> listener.onHoldStart()
            BubbleGesture.Action.HoldEnd -> listener.onHoldEnd()
            BubbleGesture.Action.DragStart -> { removeCallbacks(longPress); listener.onDragStart() }
            is BubbleGesture.Action.DragBy -> listener.onDragBy(a.dx, a.dy)
            BubbleGesture.Action.DragEnd -> listener.onDragEnd()
        }
    }
}
