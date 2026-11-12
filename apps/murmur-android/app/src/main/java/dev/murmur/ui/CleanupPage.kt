package dev.murmur.ui

import android.app.Activity
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView
import dev.murmur.EngineHolder
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.HealthReport
import dev.murmur.data.EnginePlan
import dev.murmur.data.CleanupEngines
import dev.murmur.data.CloudText
import dev.murmur.data.DevSettings
import dev.murmur.data.DevSettings.CleanupRoute
import dev.murmur.data.OpenRouter
import dev.murmur.data.CloudPage
import dev.murmur.data.Settings

/**
 * Settings → AI cleanup's "Engines" card (D36): this build's cleanup engines in the stored order,
 * the same ranked rows as Speech to text ([SpeechPage]): rank square, name, state line, switch;
 * "Change order" swaps the switches for ↑ / ↓. Everything applies at once (store + live router).
 * OpenRouter asks for its own consent first, as on the Cloud page. The phone's switch is
 * also the on-device AI ([Settings.llmEnabled]), via [update] so the engine reloads.
 */
internal class CleanupPage(
    private val activity: Activity,
    private val ui: DevUi,
    /** SettingsActivity.applyNow: writes [Settings] and runs its effects. */
    private val update: (Settings.() -> Settings) -> Unit,
) {
    private val app get() = MurmurApp.of(activity)
    private val caps get() = app.caps
    private lateinit var engineBox: LinearLayout
    private var orderRow: View? = null
    private var orderLabel: TextView? = null
    private var reordering = false
    /** Set by the activity from its server health probe (as on Speech to text); [checking] while one runs. */
    var health: HealthReport? = null
    var checking = false
    /** What the rows show now; [refresh] rebuilds them only when it changes (polling keeps focus put). */
    private var shown: String? = null

    private fun dp(v: Int) = (v * activity.resources.displayMetrics.density).toInt()

    fun render(col: LinearLayout) = with(ui) {
        col.sectionLabel("Engines")
        col.card {
            engineBox = LinearLayout(ctx).apply {
                orientation = LinearLayout.VERTICAL
                dividerDrawable = this@card.dividerDrawable
                showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
            }.also(::addView)
            if (CleanupEngines.engines(app.dev.load(), caps).size > 1) orderRow = changeOrderRow().also(::addView)
        }
        col.note("Tried top to bottom: the first one that is on and ready cleans the text, whichever engine transcribed it. If it fails, the next one takes over.")
        refresh()
    }

    private fun changeOrderRow(): View = with(ui) {
        val label = tv("Change order", TypeScale.BUTTON, p.text, TypeScale.W_MEDIUM)
        orderLabel = label
        LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(52)
            setPadding(dp(20), 0, dp(20), 0)
            isClickable = true; isFocusable = true
            background = android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), null, null)
            addView(ImageView(ctx).apply {
                setImageResource(R.drawable.ic_lw_order); imageTintList = android.content.res.ColorStateList.valueOf(p.text)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, LinearLayout.LayoutParams(dp(20), dp(20)).apply { marginEnd = dp(10) })
            addView(label)
            setOnClickListener { reordering = !reordering; refresh(); ui.announce(if (reordering) "Use the arrows to change the order" else "Order saved") }
            accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.className = android.widget.Button::class.java.name
                }
            }
        }
    }

    // ---- refresh ----

    fun refresh() {
        val d = app.dev.load()
        val s = app.settings.load()
        val list = CleanupEngines.engines(d, caps)
        val rows = list.map { r -> Triple(r, CleanupEngines.on(d, s, r), subtitle(r, d, s)) }
        val key = "$reordering ${d.cloud.cleanupProviderName} " + rows.joinToString("|")
        if (key != shown) {
            shown = key
            engineBox.removeAllViews()
            rows.forEachIndexed { i, (r, on, sub) -> engineBox.engineRow(i, rows.size, r, on, sub, d) }
        }
        orderLabel?.text = if (reordering) "Done" else "Change order"
        orderRow?.contentDescription = if (reordering) "Done changing the order" else "Change order"
    }

    /** The phone's model: loaded (true), failed (false), or not settled yet (null). */
    private fun phoneLoaded(): Boolean? = when (val st = app.engine.state.value) {
        is EngineHolder.State.Ready -> when (st.llm) {
            is EnginePlan.LlmStatus.Loaded -> true
            is EnginePlan.LlmStatus.Failed -> false
            else -> null
        }
        is EngineHolder.State.Failed -> false
        else -> null
    }

    private fun subtitle(r: CleanupRoute, d: DevSettings, s: Settings): String = when (r) {
        CleanupRoute.LOCAL -> CleanupEngines.phoneLine(CleanupEngines.on(d, s, r), app.settings.llmModelPath?.let { java.io.File(it).name }, phoneLoaded())
        CleanupRoute.SERVER -> {
            val st = if (d.serverConfigured) app.engine.router()?.cleanupStatus() else null
            CleanupEngines.serverLine(d, caps, st?.serverLlm, st?.serverCircuit?.open == true, health, checking)
        }
        CleanupRoute.CLOUD -> CleanupEngines.cloudLine(d, app.cloudKeys.has(OpenRouter.ID))
    }

    private fun LinearLayout.engineRow(i: Int, n: Int, r: CleanupRoute, on: Boolean, sub: String, d: DevSettings) = with(ui) {
        val name = CleanupEngines.name(r)
        val title = tv(name, TypeScale.ROW, p.text, TypeScale.W_MEDIUM)
        val subtitle = tv(sub, TypeScale.SUB, p.muted).apply { setPadding(0, dp(2), 0, 0) }
        val texts = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL; addView(title); addView(subtitle)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(DevUi.ROW_MIN_DP)
            setPadding(dp(20), dp(14), dp(if (reordering) 8 else 20), dp(14))
            addView(rankSquare(i + 1).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(14) })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
        }
        if (reordering) {
            row.addView(iconButton(R.drawable.ic_dev_up, "Move $name up") { move(r, -1) }.apply { isEnabled = i > 0; alpha = if (i > 0) 1f else 0.3f })
            row.addView(iconButton(R.drawable.ic_dev_down, "Move $name down") { move(r, +1) }.apply { isEnabled = i < n - 1; alpha = if (i < n - 1) 1f else 0.3f })
            row.isFocusable = true
            row.contentDescription = "${i + 1}. $name. $sub"
        } else {
            val sw = designSwitch(on).apply {
                isClickable = false; isFocusable = false
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            row.addView(sw)
            row.isClickable = true; row.isFocusable = true
            row.background = android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), null, null)
            row.setOnClickListener { toggle(r, !on) }
            row.accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.className = Switch::class.java.name
                    info.isCheckable = true; info.isChecked = on
                    info.text = "${i + 1}. ${CleanupEngines.useLabel(r)}. $sub"
                }
            }
        }
        addView(row)
    }

    // ---- changes (apply at once, store + live router) ----

    private fun saveCleanup(d: DevSettings) {
        app.dev.saveCloud(cloud = d.cloud, allowCloudCleanup = d.allowCloudCleanup, cleanupOrder = d.cleanupOrder, cleanupOff = d.cleanupOff)
        app.engine.applyDevSettings(app.dev.load())
        refresh()
    }

    private fun toggle(r: CleanupRoute, on: Boolean) {
        when {
            r == CleanupRoute.CLOUD && on -> enableCloud()
            r == CleanupRoute.CLOUD -> saveCleanup(CloudPage.disableCleanup(app.dev.load()))
            r == CleanupRoute.LOCAL -> {
                // The cleanup entry first (live router updated in place), then the AI switch,
                // whose engine reload then reads the stored entry.
                val d = app.dev.load()
                app.dev.saveCloud(cloud = d.cloud, cleanupOff = d.withCleanup(r, on).cleanupOff)
                app.engine.applyDevSettings(app.dev.load())
                update { copy(llmEnabled = on) }
                refresh()
            }
            else -> saveCleanup(app.dev.load().withCleanup(r, on))
        }
    }

    /** Same consent as the Cloud page (D52: the selected provider's); the engine is switched on where the list shows it. */
    private fun enableCloud() {
        val s = app.dev.load()
        val go = {
            saveCleanup(CloudPage.enableCleanup(app.dev.load(), System.currentTimeMillis()))
            // A custom server's key is optional.
            if (!s.cloud.customProvider && !app.cloudKeys.has(OpenRouter.ID)) {
                android.widget.Toast.makeText(activity, "Add your OpenRouter API key on the Cloud page: nothing is sent without one.", android.widget.Toast.LENGTH_LONG).show()
            }
        }
        if (!CloudPage.cleanupNeedsConsent(s)) { go(); return }
        val (title, body, button) = CloudText.cleanupConsent(s.cloud)
        ui.confirm(title, body, button, onYes = go, onNo = { refresh() })
    }

    private fun move(r: CleanupRoute, dir: Int) {
        val d = app.dev.load()
        saveCleanup(d.copy(cleanupOrder = CleanupEngines.reorder(d, caps, r, dir)))
        ui.announce("${CleanupEngines.name(r)} is now number ${CleanupEngines.engines(app.dev.load(), caps).indexOf(r) + 1}")
    }
}
