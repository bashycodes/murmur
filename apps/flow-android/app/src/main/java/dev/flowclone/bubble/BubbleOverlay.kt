package dev.flowclone.bubble

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.hardware.input.InputManager
import android.os.Handler
import android.os.Looper
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.view.animation.OvershootInterpolator
import android.widget.FrameLayout
import android.widget.TextView
import dev.flowclone.session.DictationState

/**
 * The `TYPE_APPLICATION_OVERLAY` bubble window. Never focusable (taking focus
 * would end the target field's IME session). Idle = attached but invisible and
 * click-through, so showing it again is instant.
 *
 * Also owns two click-through satellite windows: the live-preview line and the bottom
 * snooze target shown while the Ready bubble is dragged.
 */
class BubbleOverlay(
    private val context: Context,
    private val positionStore: BubblePositioner.Store,
    listener: BubbleView.Listener,
    /** Called when a drag from Ready ends on the snooze target; true = snoozed. */
    private val onSnoozeDrop: () -> Boolean,
) {
    private val wm = context.getSystemService(WindowManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val density = context.resources.displayMetrics.density
    private val forwarding = object : BubbleView.Listener by listener {
        override fun onDragStart() { snapAnim?.cancel(); dragStarted(); listener.onDragStart() }
        override fun onDragBy(dx: Float, dy: Float) { moveBy(dx, dy); listener.onDragBy(dx, dy) }
        override fun onDragEnd() { dragEnded(); listener.onDragEnd() }
    }
    val view = BubbleView(context, forwarding)
    /** Transparent margin round the bubble so its drop shadow is not clipped by the window edge. */
    private val shadowPad = (BubbleColors.SHADOW_PAD_DP * density).toInt()
    private val root = FrameLayout(context).apply {
        descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
        setPadding(shadowPad, shadowPad, shadowPad, shadowPad)
        clipChildren = false; clipToPadding = false
        addView(view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT))
    }
    private var attached = false
    private var snapAnim: ValueAnimator? = null
    private var visible = false

    private val maxAlpha = minOf(0.8f, context.getSystemService(InputManager::class.java)?.maximumObscuringOpacityForTouch ?: 0.8f)

    private val params = WindowManager.LayoutParams(
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.WRAP_CONTENT,
        WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS or WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED,
        PixelFormat.TRANSLUCENT,
    ).apply {
        gravity = Gravity.TOP or Gravity.START
        windowAnimations = 0
        title = "FlowCloneBubble"
    }

    /**
     * [params] holds the bubble's own top-left (all the positioning maths uses it); the window
     * itself sits [shadowPad] up-left of that so the padded root puts the bubble exactly there.
     */
    private val windowParams = WindowManager.LayoutParams()
    private fun win(): WindowManager.LayoutParams = windowParams.apply {
        copyFrom(params); x = params.x - shadowPad; y = params.y - shadowPad
    }

    private fun positioner(): BubblePositioner {
        val m = context.resources.displayMetrics
        return BubblePositioner(m.widthPixels, m.heightPixels, m.density, positionStore, view.ui.scale)
    }

    private fun bubbleW() = (BubbleGeometry.widthDp(view.ui) * density).toInt()
    private fun bubbleH() = (BubbleGeometry.heightDp(view.ui) * density).toInt()

    fun attach() {
        if (attached) return
        val p = positioner().position(bubbleW(), bubbleH())
        params.x = p.x; params.y = p.y
        applyVisibility(false)
        runCatching { wm.addView(root, win()) }.onSuccess { attached = true }
        startWatchdog()
    }

    /** After unlock / config change: re-add a window the system dropped. */
    fun ensureAttached() {
        if (attached && root.isAttachedToWindow) return
        attached = false
        runCatching { wm.addView(root, win()) }.onSuccess { attached = true }
    }

    fun detach() {
        main.removeCallbacksAndMessages(null)
        if (attached) runCatching { wm.removeView(root) }
        attached = false
        preview.hide()
        target.hide()
    }

    fun render(ui: BubbleUi) {
        val oldW = bubbleW()
        val oldH = bubbleH()
        val rescaled = ui.scale != view.ui.scale
        view.update(ui)
        val dictating = ui.state is DictationState.Recording || ui.state is DictationState.Processing
        val keepOn = if (dictating) WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON else 0
        params.flags = (params.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON.inv()) or keepOn
        if (rescaled) {
            // "Bubble size" changed: back to the saved anchor with the margins at the new size.
            snapAnim?.cancel()
            val p = positioner().position(bubbleW(), bubbleH())
            params.x = p.x; params.y = p.y
        } else {
            // Docked right, the bubble grows leftwards; keep it vertically centred on its anchor.
            val newW = bubbleW()
            if (newW != oldW && params.x + oldW / 2 > context.resources.displayMetrics.widthPixels / 2) params.x -= newW - oldW
            val newH = bubbleH()
            if (newH != oldH) params.y -= (newH - oldH) / 2
            // A wide row at a large size can be wider than the gap to the far edge.
            if (newW != oldW || newH != oldH) positioner().clampDrag(params.x, params.y, newW, newH).let { params.x = it.x; params.y = it.y }
        }
        applyVisibility(ui.visible)
        if (!ui.visible) preview.hide()
    }

    /** Live preview / hint line (null hides it). */
    fun renderPreview(line: PreviewContent.Line?) {
        if (line == null || !visible) { preview.hide(); return }
        preview.show(line)
        placePreview()
    }

    private fun applyVisibility(show: Boolean) {
        visible = show
        params.alpha = if (show) maxAlpha else 0f
        params.flags = if (show) params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        else params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        update()
    }

    private fun update() {
        if (attached) runCatching { wm.updateViewLayout(root, win()) }
        if (preview.shown) placePreview()
    }

    private fun moveBy(dx: Float, dy: Float) {
        val c = positioner().clampDrag((params.x + dx).toInt(), (params.y + dy).toInt(), bubbleW(), bubbleH())
        params.x = c.x; params.y = c.y
        update()
        if (target.shown) target.setNear(snoozeGeometry().isNear(centerX(), centerY()))
    }

    private fun centerX() = params.x + bubbleW() / 2f
    private fun centerY() = params.y + bubbleH() / 2f

    private fun snoozeGeometry(): SnoozeTarget {
        val m = context.resources.displayMetrics
        return SnoozeTarget(m.widthPixels, m.heightPixels, m.density)
    }

    private var dragFromReady = false
    private val showTarget = Runnable { if (dragFromReady) target.show(snoozeGeometry()) }

    private fun dragStarted() {
        dragFromReady = view.ui.state == DictationState.Ready && !view.ui.repair
        if (dragFromReady) main.postDelayed(showTarget, SnoozeTarget.SHOW_AFTER_DRAG_MS)
    }

    private fun dragEnded() {
        main.removeCallbacks(showTarget)
        val wasShown = target.shown
        target.hide()
        val onTarget = dragFromReady && wasShown && snoozeGeometry().isNear(centerX(), centerY())
        dragFromReady = false
        if (onTarget && onSnoozeDrop()) {
            // Snoozed: go back to the saved anchor so it reappears where it was.
            val p = positioner().position(bubbleW(), bubbleH())
            params.x = p.x; params.y = p.y
            update()
            return
        }
        snapToEdge()
    }

    private fun snapToEdge() {
        val target = positioner().snap(params.x, params.y, bubbleW(), bubbleH())
        val sx = params.x; val sy = params.y
        snapAnim?.cancel()
        snapAnim = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 300
            interpolator = OvershootInterpolator(0.8f)
            addUpdateListener {
                val f = it.animatedValue as Float
                params.x = (sx + (target.x - sx) * f).toInt(); params.y = (sy + (target.y - sy) * f).toInt()
                update()
            }
            start()
        }
    }

    private fun placePreview() {
        val m = context.resources.displayMetrics
        val p = PreviewPlacement.place(
            params.x, params.y, bubbleW(), bubbleH(), preview.measuredHeight(m.widthPixels),
            m.widthPixels, (10 * density).toInt(), (24 * density).toInt(),
        )
        preview.moveTo(p)
    }

    /** Watchdog: every 15 s; after 3 ticks "should be visible but isn't attached", re-add. */
    private var missing = 0
    private fun startWatchdog() {
        main.postDelayed(object : Runnable {
            override fun run() {
                if (visible && !root.isAttachedToWindow) {
                    if (++missing >= 3) { missing = 0; attached = false; runCatching { wm.addView(root, win()) }.onSuccess { attached = true } }
                } else missing = 0
                main.postDelayed(this, 15_000)
            }
        }, 15_000)
    }

    private fun satelliteParams(w: Int, h: Int, gravity: Int, title: String) = WindowManager.LayoutParams(
        w, h, WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED,
        PixelFormat.TRANSLUCENT,
    ).apply { this.gravity = gravity; windowAnimations = 0; this.title = title; alpha = maxAlpha }

    private val preview = PreviewWindow()
    private val target = TargetWindow()

    private inner class PreviewWindow {
        var shown = false; private set
        private val text = TextView(context).apply {
            // Design tooltip: vast-950 rounded rect (12dp), sand-50 14sp/20sp text, 8×12dp padding.
            setTextColor(BubbleColors.onSurface); textSize = 14f; maxLines = 3; ellipsize = TextUtils.TruncateAt.START
            setLineHeight((20 * density).toInt())
            maxWidth = (280 * density).toInt()
            setPadding((12 * density).toInt(), (8 * density).toInt(), (12 * density).toInt(), (8 * density).toInt())
            background = GradientDrawable().apply { cornerRadius = 12 * density; setColor(BubbleColors.surface) }
        }
        private val lp = satelliteParams(
            WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.START, "FlowClonePreview",
        )

        fun show(line: PreviewContent.Line) {
            text.text = line.text
            text.alpha = if (line.dim) 0.6f else 1f
            if (!shown) { runCatching { wm.addView(text, lp) }.onSuccess { shown = true } }
        }

        fun measuredHeight(screenW: Int): Int {
            text.measure(View.MeasureSpec.makeMeasureSpec(screenW, View.MeasureSpec.AT_MOST), View.MeasureSpec.UNSPECIFIED)
            return text.measuredHeight
        }

        fun moveTo(p: PreviewPlacement.Place) {
            lp.gravity = Gravity.TOP or (if (p.alignRight) Gravity.END else Gravity.START)
            lp.x = p.x; lp.y = p.y
            if (shown) runCatching { wm.updateViewLayout(text, lp) }
        }

        fun hide() { if (shown) runCatching { wm.removeView(text) }; shown = false }
    }

    private inner class TargetWindow {
        var shown = false; private set
        private var near = false
        private val circle = object : View(context) {
            private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.surface; alpha = 0xCC }
            private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.onSurface; style = Paint.Style.STROKE; strokeWidth = 2 * density }
            private val glyph = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = BubbleColors.onSurface; textAlign = Paint.Align.CENTER; textSize = 18 * density }
            override fun onDraw(c: Canvas) {
                val r = (if (near) SnoozeTarget.CIRCLE_NEAR_DP else SnoozeTarget.CIRCLE_DP) * density / 2
                c.drawCircle(width / 2f, height / 2f, r, fill)
                c.drawCircle(width / 2f, height / 2f, r, ring)
                c.drawText("zZ", width / 2f, height / 2f - (glyph.descent() + glyph.ascent()) / 2, glyph)
            }
        }
        private val lp = satelliteParams(WindowManager.LayoutParams.MATCH_PARENT, 0, Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL, "FlowCloneSnooze")

        fun show(g: SnoozeTarget) {
            if (shown) return
            lp.height = g.windowHeightPx()
            near = false
            runCatching { wm.addView(circle, lp) }.onSuccess { shown = true }
        }

        fun setNear(n: Boolean) { if (n != near) { near = n; circle.invalidate() } }

        fun hide() { if (shown) runCatching { wm.removeView(circle) }; shown = false }
    }
}
