package dev.murmur.ui

import android.app.Activity
import android.content.Intent
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.data.HistoryVersion
import dev.murmur.data.RetryLogic
import dev.murmur.service.MurmurService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch

/** D35 "Retry with current settings" on Home and in History: start it, watch it, show versions. */
internal object RetryUi {
    const val NEEDS_SERVICE = "Retry needs Murmur running: turn it on, then try again."
    const val STARTED = "Retrying with your current settings…"

    /** Ask the service to transcribe row [id]'s recording again (never inserts). */
    fun start(activity: Activity, id: Long) {
        if (!MurmurService.running) {
            Toast.makeText(activity, NEEDS_SERVICE, Toast.LENGTH_LONG).show()
            return
        }
        activity.startService(Intent(activity, MurmurService::class.java).setAction(MurmurService.ACTION_RETRY).putExtra(MurmurService.EXTRA_HISTORY_ID, id))
        Toast.makeText(activity, STARTED, Toast.LENGTH_SHORT).show()
    }

    /**
     * Calls [onChange] on the main thread whenever a retry starts or finishes while [view] is on
     * screen (the caller reloads its rows).
     */
    fun watch(activity: Activity, view: View, onChange: () -> Unit) {
        val app = MurmurApp.of(activity)
        var scope: CoroutineScope? = null
        fun begin() {
            scope?.cancel()
            scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate).also { s ->
                s.launch { combine(app.retrying, app.historyRevision) { a, b -> a to b }.drop(1).collect { onChange() } }
            }
        }
        view.addOnAttachStateChangeListener(object : View.OnAttachStateChangeListener {
            override fun onViewAttachedToWindow(v: View) = begin()
            override fun onViewDetachedFromWindow(v: View) { scope?.cancel(); scope = null }
        })
        if (view.isAttachedToWindow) begin()
    }

    /** A watch bound to an activity's resume / pause (History). */
    class Watch(private val activity: Activity, private val onChange: () -> Unit) {
        private var job: Job? = null
        fun resume() {
            val app = MurmurApp.of(activity)
            job?.cancel()
            job = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate).launch {
                combine(app.retrying, app.historyRevision) { a, b -> a to b }.drop(1).collect { onChange() }
            }
        }
        fun pause() { job?.cancel(); job = null }
    }

    /**
     * Numbered pills (0 = the original, newest last and selected); a tap shows that version in
     * [target] ([newestText] for the newest, as the card shows it; view only: Copy and the rest keep using the newest). Older versions are dimmed.
     */
    fun DevUi.versionPills(versions: List<HistoryVersion>, target: TextView, newestText: String, describe: (String) -> Unit = {}): View? {
        val labels = RetryLogic.pills(versions)
        if (labels.isEmpty()) return null
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            contentDescription = "Versions"
        }
        val pills = mutableListOf<TextView>()
        fun select(i: Int) {
            val v = versions[i]
            val newest = i == versions.lastIndex
            target.text = if (newest) newestText else v.text.trim().ifEmpty { "(no speech)" }
            target.alpha = if (newest) 1f else 0.62f
            describe(target.text.toString())
            pills.forEachIndexed { j, t ->
                val on = j == i
                t.background = rounded(if (on) p.accent else p.tonal, 12)
                t.setTextColor(if (on) p.onAccent else p.text)
                t.isSelected = on
                t.contentDescription = "Version ${versions[j].n}" + (if (j == 0) ", original" else "") +
                    (if (j == versions.lastIndex) ", newest" else "") + (if (on) ", shown" else "")
            }
        }
        labels.forEachIndexed { i, label ->
            val t = tv(label, 13f, p.text, TypeScale.W_MEDIUM).apply {
                gravity = Gravity.CENTER
                minWidth = dp(32); minHeight = dp(32)
                setPadding(dp(10), 0, dp(10), 0)
                isClickable = true; isFocusable = true
                setOnClickListener { select(i) }
            }
            pills += t
            row.addView(t, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginEnd = dp(6) })
        }
        select(versions.lastIndex)
        return row
    }
}
