package dev.flowclone.bubble

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.drawable.Drawable
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import dev.flowclone.R
import dev.flowclone.session.DictationState
import dev.flowclone.session.DictationState.ErrorKind

/**
 * Canvas-drawn bubble: circle (Ready), recording row X·waveform·✓ (tap mode),
 * mic+waveform (hold mode), paste/copy/dismiss (FailedToPaste), alert (other errors).
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
    /** Spec dp → px at the current "Bubble size" (07 §5.4 values × [BubbleUi.scale]). */
    private fun dp(v: Float) = v * d * ui.scale

    var ui = BubbleUi(); private set
    private var processingSince = 0L
    private val smoother = Waveform.Smoother()
    /** The buttons as last drawn, for hit-testing a touch. */
    private var slots = emptyList<BubbleGeometry.Slot>()
    private val mic: Drawable = context.getDrawable(R.drawable.ic_flow)!!

    private val bg = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.parseColor("#FF1C1B1F") }
    private val accent = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.parseColor("#FF6750A4") }
    private val danger = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.parseColor("#FFB3261E") }
    private val bar = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE }
    private val glyph = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE; textAlign = Paint.Align.CENTER; textSize = 22f * d; isFakeBoldText = true
    }

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
        val pill = RectF(0f, 0f, width.toFloat(), h)
        canvas.drawRoundRect(pill, r, r, bg)
        when (s) {
            DictationState.Idle, DictationState.Ready -> if (ui.repair) {
                val pulse = 0.6f + 0.4f * kotlin.math.abs(((SystemClock.uptimeMillis() % 2000) / 1000f) - 1f)
                danger.alpha = (255 * pulse).toInt()
                circleButton(canvas, danger, BubbleButton.MAIN) { cx, cy -> drawGlyph(canvas, "!", cx, cy) }
                danger.alpha = 255
                postInvalidateOnAnimation()
            } else if (ui.minimized && !ui.showCopy) {
                circleButton(canvas, accent, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy, 9f) }
            } else {
                circleButton(canvas, accent, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy) }
                if (ui.showCopy) circleButton(canvas, bg, BubbleButton.COPY) { cx, cy -> drawGlyph(canvas, "⧉", cx, cy) }
            }
            is DictationState.Recording -> {
                val level = smoother.step(ui.level)
                if (s.hold) {
                    val pulse = 0.6f + 0.4f * kotlin.math.abs(((SystemClock.uptimeMillis() % 2000) / 1000f) - 1f)
                    accent.alpha = (255 * pulse).toInt()
                    circleButton(canvas, accent, BubbleButton.MAIN) { cx, cy -> drawMic(canvas, cx, cy) }
                    accent.alpha = 255
                    drawWave(canvas, dp(56f), width - dp(56f), level, 1f)
                } else {
                    circleButton(canvas, danger, BubbleButton.CANCEL) { cx, cy -> drawGlyph(canvas, "✕", cx, cy) }
                    drawWave(canvas, dp(62f), width - dp(62f), level, 1f)
                    circleButton(canvas, accent, BubbleButton.DONE) { cx, cy -> drawGlyph(canvas, "✓", cx, cy) }
                }
                postInvalidateOnAnimation()
            }
            is DictationState.Processing -> {
                if (showCancel) circleButton(canvas, danger, BubbleButton.CANCEL) { cx, cy -> drawGlyph(canvas, "✕", cx, cy) }
                drawWave(canvas, dp(62f), width - dp(62f), 0.6f, 0.33f)
                postInvalidateOnAnimation()
            }
            is DictationState.Error -> when (s.kind) {
                is ErrorKind.FailedToPaste -> {
                    // Paste here · Copy (2026-10-24: never lose a dictation) · Dismiss.
                    circleButton(canvas, accent, BubbleButton.PASTE) { cx, cy -> drawGlyph(canvas, "✓", cx, cy) }
                    circleButton(canvas, bg, BubbleButton.COPY) { cx, cy -> drawGlyph(canvas, "⧉", cx, cy) }
                    circleButton(canvas, danger, BubbleButton.DISMISS) { cx, cy -> drawGlyph(canvas, "✕", cx, cy) }
                }
                else -> circleButton(canvas, danger, BubbleButton.DISMISS) { cx, cy -> drawGlyph(canvas, "!", cx, cy) }
            }
        }
    }

    /** Draws [b] in its [slots] square (absent = not shown in this state). */
    private inline fun circleButton(c: Canvas, paint: Paint, b: BubbleButton, content: (Float, Float) -> Unit) {
        val slot = slots.firstOrNull { it.button == b } ?: return
        val h = height.toFloat()
        val inset = if (h < dp(40f)) dp(2f) else dp(4f)
        val rect = RectF(slot.left, 0f, slot.right, h)
        c.drawCircle(rect.centerX(), rect.centerY(), h / 2 - inset, paint)
        content(rect.centerX(), rect.centerY())
    }

    private fun drawMic(c: Canvas, cx: Float, cy: Float, halfDp: Float = 14f) {
        val half = dp(halfDp).toInt()
        mic.setBounds((cx - half).toInt(), (cy - half).toInt(), (cx + half).toInt(), (cy + half).toInt())
        mic.draw(c)
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
