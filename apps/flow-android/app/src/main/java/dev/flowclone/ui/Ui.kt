package dev.flowclone.ui

import android.app.Activity
import android.graphics.Color
import android.graphics.Typeface
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

/** Tiny programmatic-view helpers: the app has a handful of simple screens and no UI deps. */
internal fun Activity.dp(v: Int) = (v * resources.displayMetrics.density).toInt()

internal fun Activity.page(build: LinearLayout.() -> Unit): LinearLayout {
    val col = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(16), dp(16), dp(16), dp(32))
        build()
    }
    val scroll = ScrollView(this).apply { addView(col); fitsSystemWindows = true }
    setContentView(scroll)
    return col
}

internal fun LinearLayout.heading(text: String) = TextView(context).apply {
    this.text = text; textSize = 18f; setTypeface(typeface, Typeface.BOLD)
    setPadding(0, (16 * resources.displayMetrics.density).toInt(), 0, (4 * resources.displayMetrics.density).toInt())
}.also(::addView)

internal fun LinearLayout.body(text: CharSequence, color: Int? = null) = TextView(context).apply {
    this.text = text; textSize = 14f
    color?.let(::setTextColor)
}.also(::addView)

internal fun LinearLayout.button(text: String, onClick: () -> Unit) = Button(context).apply {
    this.text = text; isAllCaps = false; setOnClickListener { onClick() }
}.also { addView(it, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)) }

internal fun LinearLayout.row(vararg views: View) = LinearLayout(context).apply {
    orientation = LinearLayout.HORIZONTAL
    for (v in views) addView(v, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
}.also(::addView)

internal val OK_COLOR = Color.parseColor("#FF2E7D32")
internal val BAD_COLOR = Color.parseColor("#FFB3261E")
