package dev.murmur.ui

import android.app.Activity
import android.content.Intent
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Switch
import android.widget.TextView
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.AsrEngine
import dev.murmur.core.HealthReport
import dev.murmur.data.CloudText
import dev.murmur.data.DevPages
import dev.murmur.data.DevSettings
import dev.murmur.data.DevSettings.Route
import dev.murmur.data.DevSettingsForm
import dev.murmur.data.OpenRouter
import dev.murmur.data.OpenRouterPage
import dev.murmur.data.Settings
import dev.murmur.data.SettingsSummary
import java.io.File

/**
 * Settings → Speech to text ([dev.murmur.data.SettingsPages.Page.SPEECH], AndroidSpeech.dc.html):
 *   Engines        ranked rows (rank square, name, state line, switch) for this build's engines in
 *                  the stored try order; "Change order" swaps the switches for ↑ / ↓. Everything
 *                  applies at once ([DevPages.withEngine], [DevPages.reorder]). OpenRouter asks for
 *                  its consent first, as on the OpenRouter page ([OpenRouterPage.enableStt]).
 *                  Tapping a row opens that engine's page (Your server, the phone's model,
 *                  OpenRouter); only the switch turns it on or off.
 *   On this phone  the model in use (name, language, size, import date) → engine choice and imports;
 *                  "Import another model"; "Hebrew model" (D37: visible here, not only on the engine
 *                  page); Keep both languages loaded (D54); keep it loaded (hybrid);
 *                  Transcribe while I speak.
 * "Use dictionary words" from the design is left out: Android has no such switch (Parakeet's beam
 * search mode stays in Developer settings → Engine options).
 */
internal class SpeechPage(
    private val activity: Activity,
    private val ui: DevUi,
    /** SettingsActivity.applyNow: writes [Settings] and runs its effects. */
    private val update: (Settings.() -> Settings) -> Unit,
    /** Opens the on-device engine page (whisper / Parakeet, imports). */
    private val openEngine: () -> Unit,
) {
    private val app get() = MurmurApp.of(activity)
    private val caps get() = app.caps
    private lateinit var banner: DevUi.Result
    private lateinit var engineBox: LinearLayout
    private var orderRow: View? = null
    private var orderLabel: TextView? = null
    private var modelTitle: TextView? = null
    private var modelSub: TextView? = null
    private var modelRow: View? = null
    private var importButton: android.widget.Button? = null
    private var hebrewRow: DevUi.NavRow? = null
    private var keepWarm: DevUi.ToggleRow? = null
    private var keepBoth: DevUi.ToggleRow? = null
    private lateinit var transcribe: DevUi.ToggleRow
    private var reordering = false
    /** Set by the activity from its health probe; [checking] while one runs. */
    var health: HealthReport? = null
    var checking = false

    private fun dp(v: Int) = (v * activity.resources.displayMetrics.density).toInt()

    fun render(col: LinearLayout) = with(ui) {
        col.note("Tried top to bottom. If one fails, the next picks up the same audio and nothing is lost.")
        banner = col.banner()
        col.sectionLabel("Engines")
        col.card {
            engineBox = LinearLayout(ctx).apply {
                orientation = LinearLayout.VERTICAL
                dividerDrawable = this@card.dividerDrawable
                showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
            }.also(::addView)
            if (DevPages.engines(app.dev.load(), caps).size > 1) orderRow = changeOrderRow().also(::addView)
        }
        val s = app.settings.load()
        col.sectionLabel(if (caps.local) "On this phone" else "Options")
        col.card {
            if (caps.local) {
                modelBlock()
                // D37: the Hebrew model was only reachable through the model block → engine page.
                hebrewRow = navRow(R.drawable.ic_set_language, "Hebrew model") { activity.startActivity(ModelImportActivity.hebrewIntent(activity)) }
                // D54: applies at the next dictation (it reloads once then).
                keepBoth = toggleRow("Keep both languages loaded", keepBothSummary(), s.keepBothLanguages)
                    .also { it.onChange { on -> update { copy(keepBothLanguages = on) } } }
                if (caps.remote) {
                    keepWarm = toggleRow("Keep loaded while the server is used", "Instant fallback if the server drops, at the cost of memory", app.dev.load().keepLocalWarm)
                        .also { it.onChange { on -> saveDev(app.dev.load().copy(keepLocalWarm = on)) } }
                }
            }
            transcribe = toggleRow("Transcribe while I speak", "Long dictations finish sooner. Not used for OpenRouter", s.decodeWhileSpeaking)
                .also { it.onChange { on -> update { copy(decodeWhileSpeaking = on) } } }
        }
        refresh()
    }

    /** Model name, language · size · import date; the block opens the engine page. Import button below. */
    private fun LinearLayout.modelBlock() = with(ui) {
        val title = tv("", TypeScale.ROW, p.text, TypeScale.W_MEDIUM)
        val sub = tv("", TypeScale.SUB, p.muted).apply { setPadding(0, dp(2), 0, 0) }
        val texts = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL; addView(title); addView(sub)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }
        val chevron = ImageView(ctx).apply {
            setImageResource(R.drawable.ic_lw_chevron); imageTintList = android.content.res.ColorStateList.valueOf(p.text)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(DevUi.ROW_MIN_DP)
            setPadding(dp(20), dp(16), dp(16), dp(8))
            isClickable = true; isFocusable = true
            background = android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), null, null)
            setOnClickListener { openEngine() }
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
            addView(chevron, LinearLayout.LayoutParams(dp(22), dp(22)))
        }
        val button = outlinedButton("Import another model") {
            val pk = app.settings.load().asrEngine == AsrEngine.PARAKEET
            activity.startActivity(if (pk) ModelImportActivity.parakeetIntent(activity) else Intent(activity, ModelImportActivity::class.java))
        }
        // One block in the card (no divider between the model and its button).
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            addView(row)
            addView(button, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                marginStart = dp(20); marginEnd = dp(20); topMargin = dp(4); bottomMargin = dp(16)
            })
        })
        modelTitle = title; modelSub = sub; modelRow = row; importButton = button
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
        renderEngines(d, s)
        orderLabel?.text = if (reordering) "Done" else "Change order"
        orderRow?.contentDescription = if (reordering) "Done changing the order" else "Change order"
        val w = DevPages.speechWarnings(DevSettingsForm.validate(DevSettingsForm.Input.of(d), caps).warnings)
        banner.show(w.joinToString("\n"), if (DevPages.NO_ENGINE in w) Tone.BAD else Tone.WARN)
        refreshModel(s)
        keepWarm?.let { if (it.isChecked != d.keepLocalWarm) it.isChecked = d.keepLocalWarm }
        keepBoth?.let {
            if (it.isChecked != s.keepBothLanguages) it.isChecked = s.keepBothLanguages
            it.setDescription(keepBothSummary(s))
        }
        if (transcribe.isChecked != s.decodeWhileSpeaking) transcribe.isChecked = s.decodeWhileSpeaking
    }

    private fun fileName(path: String?) = path?.let { File(it).name }

    private fun keepBothSummary(s: Settings = app.settings.load()) =
        SettingsSummary.keepBothLanguages(app.engine.hebrewModel() != null, app.engine.defaultModelInstalled(s))

    private fun refreshModel(s: Settings) {
        val title = modelTitle ?: return
        val pk = s.asrEngine == AsrEngine.PARAKEET && app.settings.parakeetPath != null
        val path = if (pk) app.settings.parakeetPath else app.settings.modelPath
        val f = path?.let(::File)?.takeIf { it.exists() }
        val engine = if (pk) AsrEngine.PARAKEET else AsrEngine.WHISPER
        if (f == null) {
            title.text = "No model imported"
            modelSub?.text = "Import one to dictate without your server or the internet"
            importButton?.text = "Import a model"
        } else {
            title.text = (if (pk) "Parakeet" else "Whisper") + " · " + (SettingsSummary.shortModel(f.name) ?: f.name)
            val bytes = if (f.isDirectory) f.walk().filter { it.isFile }.sumOf { it.length() } else f.length()
            modelSub?.text = SettingsSummary.modelLine(engine, f.name, bytes, f.lastModified()) {
                java.text.SimpleDateFormat("MMM d", java.util.Locale.getDefault()).format(java.util.Date(it))
            }
            importButton?.text = "Import another model"
        }
        modelRow?.contentDescription = "${title.text}, ${modelSub?.text}. Engine and models"
        hebrewRow?.setSubtitle(SettingsSummary.hebrewModel(app.engine.hebrewModel()?.second?.label))
    }

    private fun renderEngines(d: DevSettings, s: Settings) {
        engineBox.removeAllViews()
        val list = DevPages.engines(d, caps)
        list.forEachIndexed { i, r -> engineBox.engineRow(i, list.size, r, DevPages.engineOn(d, r), subtitle(r, d, s)) }
    }

    private fun subtitle(r: Route, d: DevSettings, s: Settings) = when (r) {
        Route.SERVER -> SettingsSummary.server(d, caps, health, checking)
        Route.CLIENT -> SettingsSummary.phoneEngine(s, fileName(app.settings.modelPath), fileName(app.settings.parakeetPath))
        Route.CLOUD -> SettingsSummary.cloudEngine(d, app.cloudKeys.has(OpenRouter.ID))
    }

    /** `use your server` / `use this phone` / `use OpenRouter for speech` (the design's switch labels). */
    private fun useLabel(r: Route) = when (r) {
        Route.SERVER -> "Use your server"
        Route.CLIENT -> "Use this phone"
        Route.CLOUD -> "Use ${OpenRouter.NAME} for speech"
    }

    /** The page that sets an engine up: your server's address and key, the phone's model, OpenRouter's key. */
    private fun openSettings(r: Route) = when (r) {
        Route.SERVER -> DeveloperActivity.open(activity, DevPages.Page.SERVER)
        Route.CLIENT -> openEngine()
        Route.CLOUD -> OpenRouterActivity.open(activity)
    }

    /** TalkBack reads the view as [className] saying [text]; [checked] makes it checkable, [clickLabel] names its tap. */
    private fun readsAs(className: String, text: String, checked: Boolean? = null, clickLabel: String? = null) = object : View.AccessibilityDelegate() {
        override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
            super.onInitializeAccessibilityNodeInfo(host, info)
            info.className = className
            info.text = text
            if (checked != null) { info.isCheckable = true; info.isChecked = checked }
            if (clickLabel != null) info.addAction(AccessibilityNodeInfo.AccessibilityAction(AccessibilityNodeInfo.ACTION_CLICK, clickLabel))
        }
    }

    /**
     * One ranked engine. While reordering: ↑ / ↓. Otherwise the row body opens the engine's own
     * page ([openSettings]) and only the switch at the end turns it on or off.
     */
    private fun LinearLayout.engineRow(i: Int, n: Int, r: Route, on: Boolean, sub: String) = with(ui) {
        val name = SettingsSummary.engineName(r)
        val title = tv(name, TypeScale.ROW, p.text, TypeScale.W_MEDIUM)
        val subtitle = tv(sub, TypeScale.SUB, p.muted).apply { setPadding(0, dp(2), 0, 0) }
        val texts = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL; addView(title); addView(subtitle)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }
        val ripple = { android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), null, null) }
        val body = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(DevUi.ROW_MIN_DP)
            setPadding(dp(20), dp(14), dp(if (reordering) 8 else 12), dp(14))
            addView(rankSquare(i + 1).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(14) })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        }
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            addView(body, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        }
        if (reordering) {
            row.addView(iconButton(R.drawable.ic_dev_up, "Move $name up") { move(r, -1) }.apply { isEnabled = i > 0; alpha = if (i > 0) 1f else 0.3f })
            row.addView(iconButton(R.drawable.ic_dev_down, "Move $name down") { move(r, +1) }.apply { isEnabled = i < n - 1; alpha = if (i < n - 1) 1f else 0.3f })
            row.setPadding(0, 0, dp(8), 0)
            body.isFocusable = true
            body.contentDescription = "${i + 1}. $name. $sub"
        } else {
            body.isClickable = true; body.isFocusable = true
            body.background = ripple()
            body.setOnClickListener { openSettings(r) }
            body.accessibilityDelegate = readsAs(android.widget.Button::class.java.name, "${i + 1}. $name. $sub", clickLabel = "open $name settings")
            val sw = designSwitch(on).apply {
                isClickable = false; isFocusable = false
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            // The switch's own 48 dp+ target: the full row height, a little wider than the pill.
            val target = android.widget.FrameLayout(ctx).apply {
                setPadding(dp(8), 0, dp(20), 0)
                minimumHeight = dp(DevUi.ROW_MIN_DP); minimumWidth = dp(48)
                isClickable = true; isFocusable = true
                background = ripple()
                addView(sw, android.widget.FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.CENTER))
                setOnClickListener { toggle(r, !on) }
                accessibilityDelegate = readsAs(Switch::class.java.name, "${i + 1}. ${useLabel(r)}. $sub", checked = on)
            }
            row.addView(target, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.MATCH_PARENT))
        }
        addView(row)
    }

    // ---- changes (apply at once, store + live router) ----

    private fun saveDev(s: DevSettings) {
        app.dev.save(s)
        app.engine.applyDevSettings(app.dev.load())
        refresh()
    }

    private fun toggle(r: Route, on: Boolean) {
        if (r == Route.CLOUD && on) { enableCloud(); return }
        if (r == Route.CLOUD) {
            val s = DevPages.withEngine(app.dev.load(), r, false)
            app.dev.saveCloud(allowCloud = s.allowCloud, cloud = s.cloud)
            app.engine.applyDevSettings(app.dev.load())
            refresh()
            return
        }
        saveDev(DevPages.withEngine(app.dev.load(), r, on))
    }

    /** Same consent as the OpenRouter page; the engine starts at its listed place. */
    private fun enableCloud() {
        val s = DevPages.placeCloud(app.dev.load(), caps)
        val go = {
            val n = OpenRouterPage.enableStt(s, System.currentTimeMillis())
            app.dev.saveCloud(allowCloud = n.allowCloud, cloud = n.cloud, routeOrder = n.routeOrder)
            app.engine.applyDevSettings(app.dev.load())
            refresh()
            if (!app.cloudKeys.has(OpenRouter.ID)) {
                android.widget.Toast.makeText(activity, "Add your OpenRouter API key on the OpenRouter page: nothing is sent without one.", android.widget.Toast.LENGTH_LONG).show()
            }
        }
        if (!OpenRouterPage.sttNeedsConsent(s)) { go(); return }
        ui.confirm(CloudText.STT_CONSENT_TITLE, CloudText.sttConsentBody(s.cloud.stt), CloudText.STT_CONSENT_BUTTON, onYes = go, onNo = { refresh() })
    }

    private fun move(r: Route, dir: Int) {
        val d = app.dev.load()
        saveDev(d.copy(routeOrder = DevPages.reorder(d, caps, r, dir)))
        ui.announce("${SettingsSummary.engineName(r)} is now number ${DevPages.engines(app.dev.load(), caps).indexOf(r) + 1}")
    }
}
