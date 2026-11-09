package dev.murmur.probe

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.drawable.GradientDrawable
import android.provider.Settings
import android.view.Gravity
import android.view.MotionEvent
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The overlay test (PQ-a): a small probe-owned `TYPE_APPLICATION_OVERLAY` panel, the same window
 * type as the dictation bubble, so a run can start while another app is the top activity. It
 * answers "does GenAI speech work when our app is not on top?" (expect BACKGROUND_USE_BLOCKED if
 * not). Never focusable, so the app underneath keeps its keyboard. Drag it by its text line.
 *   G mic / G clip: GenAI (mode from the probe screen); P mic / P clip: the platform recognizer.
 * A mic button turns into "■" while recording; tap it again to stop. × hides the panel.
 */
object ProbeOverlay {
    private var root: LinearLayout? = null
    private var status: TextView? = null

    val shown get() = root != null

    fun canShow(ctx: Context) = Settings.canDrawOverlays(ctx)

    @SuppressLint("ClickableViewAccessibility", "SetTextI18n")
    fun show(ctx: Context) {
        if (root != null) return
        val app = ctx.applicationContext
        val wm = app.getSystemService(WindowManager::class.java)
        val dp = app.resources.displayMetrics.density
        val lp = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply { gravity = Gravity.TOP or Gravity.END; x = (8 * dp).toInt(); y = (160 * dp).toInt() }

        val panel = LinearLayout(app).apply {
            orientation = LinearLayout.VERTICAL
            setPadding((8 * dp).toInt(), (6 * dp).toInt(), (8 * dp).toInt(), (6 * dp).toInt())
            background = GradientDrawable().apply { setColor(0xE6202124.toInt()); cornerRadius = 14 * dp }
        }
        val msg = TextView(app).apply {
            setTextColor(Color.WHITE); textSize = 12f; maxWidth = (240 * dp).toInt(); maxLines = 4
            text = "English probe · drag here"
        }
        panel.addView(msg)
        // Drag by the text line.
        var downX = 0f; var downY = 0f; var startX = 0; var startY = 0
        msg.setOnTouchListener { _, e ->
            when (e.actionMasked) {
                MotionEvent.ACTION_DOWN -> { downX = e.rawX; downY = e.rawY; startX = lp.x; startY = lp.y; true }
                MotionEvent.ACTION_MOVE -> {
                    lp.x = startX - (e.rawX - downX).toInt(); lp.y = startY + (e.rawY - downY).toInt()
                    root?.let { runCatching { wm.updateViewLayout(it, lp) } }; true
                }
                else -> false
            }
        }

        fun button(label: String, onTap: (Button) -> Unit) = Button(app).apply {
            text = label; textSize = 11f; isAllCaps = false; minWidth = 0; minimumWidth = 0
            minHeight = 0; minimumHeight = 0
            setPadding((8 * dp).toInt(), (4 * dp).toInt(), (8 * dp).toInt(), (4 * dp).toInt())
            setOnClickListener { onTap(this) }
        }

        fun listener(b: Button?, label: String) = object : RunListener {
            override fun onState(s: String) { msg.text = s }
            override fun onPartial(text: String) { msg.text = "… $text" }
            override fun onFinal(text: String) { msg.text = "✓ $text" }
            override fun onDone(record: ProbeRecord, line: String) {
                b?.text = label
                msg.text = "${record.engine} ${record.outcome}" +
                    (record.timing.latencyMs?.let { " · ${it} ms" } ?: "") +
                    (record.error?.let { "\n$it" } ?: "") + (if (record.text.isNotEmpty()) "\n${record.text}" else "")
            }
        }

        fun clipOrSay(): ProbeRuntime.Clip? = ProbeRuntime.clip ?: run { msg.text = "No clip: make one on the probe screen"; null }

        val row1 = LinearLayout(app).apply { orientation = LinearLayout.HORIZONTAL }
        row1.addView(button("G mic") { b ->
            if (GenAiProbe.micRunning) GenAiProbe.stopMic()
            else { b.text = "■"; GenAiProbe.run(app, ProbeRuntime.genAiAdvanced, null, "overlay", listener(b, "G mic")) }
        })
        row1.addView(button("G clip") { _ ->
            clipOrSay()?.let { GenAiProbe.run(app, ProbeRuntime.genAiAdvanced, it, "overlay", listener(null, "G clip")) }
        })
        row1.addView(button("P mic") { b ->
            if (PlatformProbe.micRunning) PlatformProbe.stopMic()
            else { b.text = "■"; PlatformProbe.run(app, null, ProbeRuntime.biasing, ProbeRuntime.formatting, "overlay", listener(b, "P mic")) }
        })
        row1.addView(button("P clip") { _ ->
            clipOrSay()?.let { PlatformProbe.run(app, it, ProbeRuntime.biasing, ProbeRuntime.formatting, "overlay", listener(null, "P clip")) }
        })
        row1.addView(button("×") { _ -> hide(app) })
        panel.addView(row1)

        try {
            wm.addView(panel, lp)
            root = panel; status = msg
        } catch (t: Throwable) {
            android.util.Log.w(ProbeRuntime.TAG, "overlay refused", t)
        }
    }

    fun hide(ctx: Context) {
        val r = root ?: return
        runCatching { ctx.applicationContext.getSystemService(WindowManager::class.java).removeView(r) }
        root = null; status = null
    }
}
