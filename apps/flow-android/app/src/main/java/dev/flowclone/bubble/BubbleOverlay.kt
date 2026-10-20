package dev.flowclone.bubble

import android.animation.ValueAnimator
import android.content.Context
import android.graphics.PixelFormat
import android.hardware.input.InputManager
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.ViewGroup
import android.view.WindowManager
import android.view.animation.OvershootInterpolator
import android.widget.FrameLayout
import dev.flowclone.session.DictationState

/**
 * The `TYPE_APPLICATION_OVERLAY` bubble window (07 §5.1). Never focusable (taking focus
 * would end the target field's IME session). Idle = attached but invisible and
 * click-through, so showing it again is instant.
 */
class BubbleOverlay(
    private val context: Context,
    private val positionStore: BubblePositioner.Store,
    listener: BubbleView.Listener,
) {
    private val wm = context.getSystemService(WindowManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val forwarding = object : BubbleView.Listener by listener {
        override fun onDragStart() { snapAnim?.cancel(); listener.onDragStart() }
        override fun onDragBy(dx: Float, dy: Float) { moveBy(dx, dy); listener.onDragBy(dx, dy) }
        override fun onDragEnd() { snapToEdge(); listener.onDragEnd() }
    }
    val view = BubbleView(context, forwarding)
    private val root = FrameLayout(context).apply {
        descendantFocusability = ViewGroup.FOCUS_BLOCK_DESCENDANTS
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

    private fun positioner(): BubblePositioner {
        val m = context.resources.displayMetrics
        return BubblePositioner(m.widthPixels, m.heightPixels, m.density, positionStore)
    }

    private fun bubbleW() = (view.widthDpFor(view.ui) * context.resources.displayMetrics.density).toInt()
    private fun bubbleH() = (56 * context.resources.displayMetrics.density).toInt()

    fun attach() {
        if (attached) return
        val p = positioner().position(bubbleW(), bubbleH())
        params.x = p.x; params.y = p.y
        applyVisibility(false)
        runCatching { wm.addView(root, params) }.onSuccess { attached = true }
        startWatchdog()
    }

    fun detach() {
        main.removeCallbacksAndMessages(null)
        if (attached) runCatching { wm.removeView(root) }
        attached = false
    }

    fun render(ui: BubbleUi) {
        val oldW = bubbleW()
        view.update(ui)
        val show = ui.state != DictationState.Idle
        val dictating = ui.state is DictationState.Recording || ui.state is DictationState.Processing
        val keepOn = if (dictating) WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON else 0
        params.flags = (params.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON.inv()) or keepOn
        // Docked right, the bubble grows leftwards.
        val newW = bubbleW()
        if (newW != oldW && params.x + oldW / 2 > context.resources.displayMetrics.widthPixels / 2) params.x -= newW - oldW
        applyVisibility(show)
    }

    private fun applyVisibility(show: Boolean) {
        visible = show
        params.alpha = if (show) maxAlpha else 0f
        params.flags = if (show) params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
        else params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
        update()
    }

    private fun update() { if (attached) runCatching { wm.updateViewLayout(root, params) } }

    private fun moveBy(dx: Float, dy: Float) {
        val c = positioner().clampDrag((params.x + dx).toInt(), (params.y + dy).toInt(), bubbleW(), bubbleH())
        params.x = c.x; params.y = c.y
        update()
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

    /** 07 §5.1 watchdog: every 15 s; after 3 ticks "should be visible but isn't attached", re-add. */
    private var missing = 0
    private fun startWatchdog() {
        main.postDelayed(object : Runnable {
            override fun run() {
                if (visible && !root.isAttachedToWindow) {
                    if (++missing >= 3) { missing = 0; attached = false; runCatching { wm.addView(root, params) }.onSuccess { attached = true } }
                } else missing = 0
                main.postDelayed(this, 15_000)
            }
        }, 15_000)
    }
}
