package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Menu
import android.view.MenuItem
import android.view.View
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.flowclone.EngineHolder
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.AsrEngine
import dev.flowclone.data.DevSummary
import dev.flowclone.data.DevUnlock
import dev.flowclone.data.EnginePlan
import dev.flowclone.data.LanguageDefault
import dev.flowclone.data.Retention
import dev.flowclone.data.Settings
import dev.flowclone.data.SettingsDraft
import dev.flowclone.data.SettingsPages
import dev.flowclone.data.SettingsPages.Field
import dev.flowclone.data.SettingsPages.Page
import dev.flowclone.data.SettingsSummary
import java.io.File
import kotlin.concurrent.thread

/**
 * Settings as a hub + focused sub-pages (the Developer settings layout, ui/DevUi), one activity
 * instance per page ([EXTRA_PAGE]):
 *   hub       status (engine, next dictation language, warnings); one row per page with its
 *             current value; Developer settings + Mock engine; About (version, 7-tap unlock)
 *   language  dictation language, auto-format
 *   style     writing style per bucket (personal / work / email / other), D27
 *   app_styles  per-app bucket overrides: app list + "Add app" picker, from the style page
 *   engine    whisper / Parakeet, model imports                       (not in `remote`)
 *   cleanup   local AI cleanup on/off, GGUF model, strength            (not in `remote`)
 *   timing    hands-free stop after silence, decode while speaking, session limit   [Save]
 *   bubble    live preview, shrink to a dot, shake to restore, bubble size
 *   apps      excluded apps (package names)                                          [Save]
 *   history   retention (asks before deleting)
 * Save rule ([SettingsPages]): pages with typed fields have Save in the top bar (changed + valid,
 * Discard prompt); switches and choices apply at once. Same prefs keys and [Settings] as before.
 */
class SettingsActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    private lateinit var page: Page
    private var draft: SettingsDraft? = null
    private val fieldRows = LinkedHashMap<Field, DevUi.FieldRow>()
    private var decodeToggle: DevUi.ToggleRow? = null
    private var saveItem: MenuItem? = null
    private var backCallback: OnBackInvokedCallback? = null
    private val refreshers = ArrayList<() -> Unit>()
    /** Hub status card: the engine loads / reloads while the page is open, so it is polled. */
    private var status: (() -> Unit)? = null
    private val main = Handler(Looper.getMainLooper())
    private val poll = object : Runnable {
        override fun run() { status?.invoke(); main.postDelayed(this, 2_000) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        page = intent.getStringExtra(EXTRA_PAGE)?.let { n -> Page.entries.firstOrNull { it.name == n } } ?: Page.HUB
        // A page this build does not have (e.g. a stale intent on `remote`) opens the hub.
        if (!SettingsPages.visible(page, caps)) page = Page.HUB
        ui = DevUi(this)
        title = SettingsPages.TITLES.getValue(page)
        if (page != Page.HUB) actionBar?.setDisplayHomeAsUpEnabled(true)
        if (SettingsPages.hasForm(page)) draft = SettingsDraft.of(app.settings.load(), page)
        when (page) {
            Page.HUB -> renderHub()
            Page.LANGUAGE -> renderLanguage()
            Page.STYLE -> renderStyle()
            Page.APP_STYLES -> renderAppStyles()
            Page.ENGINE -> renderEngine()
            Page.CLEANUP -> renderCleanup()
            Page.TIMING -> renderTiming()
            Page.BUBBLE -> renderBubble()
            Page.APPS -> renderApps()
            Page.HISTORY -> renderHistory()
        }
        if (draft != null) {
            backCallback = OnBackInvokedCallback { confirmLeave() }.also {
                onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, it)
            }
            validate()
        }
    }

    override fun onDestroy() {
        backCallback?.let { onBackInvokedDispatcher.unregisterOnBackInvokedCallback(it) }
        super.onDestroy()
    }

    override fun onResume() {
        super.onResume()
        // A sub-page, the model import or Developer settings may have changed things meanwhile.
        draft = draft?.rebase(app.settings.load())
        if (draft != null) validate()
        refresh()
        if (status != null) poll.run()
    }

    override fun onPause() {
        main.removeCallbacks(poll)
        super.onPause()
    }

    private fun refresh() { for (r in refreshers) r() }

    private fun open(p: Page) = open(this, p)

    private fun fileName(path: String?) = path?.let { File(it).name }

    // ---- apply at once (switches, choices) ----

    /** Writes the whole [Settings] from what is stored now with [f] applied, then its effects. */
    private fun applyNow(f: Settings.() -> Settings) {
        val old = app.settings.load()
        val n = old.f()
        if (n == old) return
        app.settings.save(n)
        effects(old, n)
        refresh()
        status?.invoke()
    }

    private fun effects(old: Settings, n: Settings) {
        val e = SettingsPages.effects(old, n)
        // Review P3: database work off the main thread (the engine reload is non-blocking since C1).
        if (e.purgeHistory) thread(name = "history-purge") { runCatching { app.history.purge(n.retention) } }
        if (e.reloadEngine) app.engine.reload()
        if (n.llmEnabled && !old.llmEnabled && app.settings.llmModelPath == null) {
            Toast.makeText(this, "AI cleanup needs a GGUF model: import one first.", Toast.LENGTH_LONG).show()
        }
    }

    // ---- Save (pages with typed fields) ----

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        val d = draft ?: return false
        saveItem = menu.add(Menu.NONE, MENU_SAVE, Menu.NONE, "Save").apply {
            setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS or MenuItem.SHOW_AS_ACTION_WITH_TEXT)
            isEnabled = d.canSave(app.settings.load())
            contentDescription = "Save ${SettingsPages.TITLES.getValue(page)}"
        }
        return true
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean = when (item.itemId) {
        MENU_SAVE -> { onSave(); true }
        android.R.id.home -> { if (draft != null) confirmLeave() else finish(); true }
        else -> super.onOptionsItemSelected(item)
    }

    private fun confirmLeave() {
        if (draft?.dirty != true) { finish(); return }
        AlertDialog.Builder(this)
            .setTitle("Discard changes?")
            .setMessage("Your changes on this page are not saved.")
            .setNegativeButton("Keep editing", null)
            .setPositiveButton("Discard") { _, _ -> finish() }
            .show()
    }

    private fun readViews(i: SettingsPages.Input): SettingsPages.Input = i.copy(
        endpointMs = fieldRows[Field.ENDPOINT]?.text ?: i.endpointMs,
        capMinutes = fieldRows[Field.CAP]?.text ?: i.capMinutes,
        decodeWhileSpeaking = decodeToggle?.isChecked ?: i.decodeWhileSpeaking,
        denylist = fieldRows[Field.DENYLIST]?.text ?: i.denylist,
    )

    private fun validate(): SettingsPages.Result? {
        val d = draft ?: return null
        val now = d.edit { readViews(this) }
        draft = now
        val stored = app.settings.load()
        val r = now.result(stored)
        for ((f, row) in fieldRows) row.setError(r.errors[f])
        val can = now.canSave(stored)
        saveItem?.let { if (it.isEnabled != can) it.isEnabled = can }
        return r
    }

    private fun onSave() {
        val d = draft ?: return
        val r = validate() ?: return
        val n = r.settings ?: return
        if (!d.dirty) return
        val old = app.settings.load()
        app.settings.save(n)
        effects(old, n)
        Toast.makeText(this, "Saved", Toast.LENGTH_SHORT).show()
        draft = SettingsDraft.of(app.settings.load(), page).edit { readViews(this) }
        validate()
    }

    // =====================================================================================
    // Hub
    // =====================================================================================

    private fun renderHub() = with(ui) {
        screen {
            sectionLabel("STATUS")
            card {
                val engine = infoRow("Engine")
                val lang = infoRow("Next dictation language")
                val warn = note("", p.danger, 14f).apply { visibility = View.GONE; accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }
                status = {
                    val s = app.settings.load()
                    // Only touch views that changed (TalkBack / scroll position stay put while polling).
                    engineStatus().let { if (engine.text.toString() != it) engine.text = it }
                    LanguageDefault.label(s.language, app.effectiveLanguage(s)).let { if (lang.text.toString() != it) lang.text = it }
                    val w = SettingsSummary.warnings(
                        s, caps, whisper = fileName(app.settings.modelPath), parakeet = fileName(app.settings.parakeetPath), llm = fileName(app.settings.llmModelPath),
                    )
                    val t = w.joinToString("\n")
                    if (warn.text.toString() != t) warn.text = t
                    warn.visibility = if (w.isEmpty()) View.GONE else View.VISIBLE
                }
            }

            sectionLabel("DICTATION")
            card {
                val language = navRow(R.drawable.ic_set_language, "Language") { open(Page.LANGUAGE) }
                val style = navRow(R.drawable.ic_set_style, "Writing style") { open(Page.STYLE) }
                val engine = if (SettingsPages.visible(Page.ENGINE, caps)) navRow(R.drawable.ic_set_mic, "Speech engine") { open(Page.ENGINE) } else null
                val cleanup = if (SettingsPages.visible(Page.CLEANUP, caps)) navRow(R.drawable.ic_set_sparkle, "AI cleanup") { open(Page.CLEANUP) } else null
                // D28: the user-facing door to OpenRouter (speech-to-text and text cleanup), cloud builds only.
                val cloud = if (caps.cloud) navRow(R.drawable.ic_dev_cloud, "Cloud (OpenRouter)") { OpenRouterActivity.open(this@SettingsActivity) } else null
                val timing = navRow(R.drawable.ic_set_timer, "Timing & limits") { open(Page.TIMING) }
                refreshers += {
                    val d = app.dev.load()
                    cloud?.setSubtitle(dev.flowclone.data.OpenRouterPage.settingsSummary(d, app.cloudKeys.has(dev.flowclone.data.OpenRouter.ID)))
                    val s = app.settings.load()
                    language.setSubtitle(SettingsSummary.language(s, app.effectiveLanguage(s)))
                    style.setSubtitle(dev.flowclone.data.WritingStyle.summary(s))
                    engine?.setSubtitle(SettingsSummary.engine(s, fileName(app.settings.modelPath), fileName(app.settings.parakeetPath)))
                    cleanup?.setSubtitle(SettingsSummary.cleanup(s, fileName(app.settings.llmModelPath), dev.flowclone.data.OpenRouterPage.cleanupEngineLine(d.effective(caps))))
                    timing.setSubtitle(SettingsSummary.timing(s))
                }
            }

            sectionLabel("BUBBLE & APPS")
            card {
                val bubble = navRow(R.drawable.ic_set_bubble, "Bubble") { open(Page.BUBBLE) }
                val apps = navRow(R.drawable.ic_set_block, "Excluded apps") { open(Page.APPS) }
                refreshers += {
                    val s = app.settings.load()
                    bubble.setSubtitle(SettingsSummary.bubble(s))
                    apps.setSubtitle(SettingsSummary.apps(s))
                }
            }

            sectionLabel("PRIVACY")
            card {
                val history = navRow(R.drawable.ic_set_history, "History") { open(Page.HISTORY) }
                refreshers += { history.setSubtitle(SettingsSummary.history(app.settings.load().retention)) }
            }

            sectionLabel("ADVANCED")
            card {
                if (app.dev.visible) {
                    val dev = navRow(R.drawable.ic_set_code, "Developer settings") { startActivity(Intent(this@SettingsActivity, DeveloperActivity::class.java)) }
                    refreshers += { dev.setSubtitle("Engines, server · " + DevSummary.routing(app.dev.load(), caps)) }
                }
                val mock = toggleRow("Mock engine", "Inserts a fixed sentence; for testing without a model", app.settings.load().mockEngine)
                mock.onChange { on -> applyNow { copy(mockEngine = on) } }
            }

            sectionLabel("ABOUT")
            card {
                // §5 Access: in builds without `remote`, 7 taps here reveal Developer settings.
                val unlock = DevUnlock({ android.os.SystemClock.elapsedRealtime() })
                navRow(R.drawable.ic_dev_info, versionLabel(), chevron = false) {
                    if (app.dev.visible) return@navRow
                    if (unlock.tap()) {
                        app.dev.devUnlocked = true
                        Toast.makeText(this@SettingsActivity, "Developer settings unlocked", Toast.LENGTH_SHORT).show()
                        recreate()
                    } else if (unlock.remaining <= 4) {
                        Toast.makeText(this@SettingsActivity, "${unlock.remaining} more taps", Toast.LENGTH_SHORT).show()
                    }
                }
            }
        }
    }

    /** The About row: the same build label as the main screen (D26), e.g. `Version 0.2.0 (hybrid) · 1a2b3c4`. */
    private fun versionLabel() = dev.flowclone.data.AppVersion.current.label(app.caps.flavor)

    private fun engineStatus(): String {
        val st = app.engine.state.value
        val base = when (st) {
            is EngineHolder.State.Ready -> "Loaded: ${st.description}"
            EngineHolder.State.Loading -> "Loading model…"
            is EngineHolder.State.Failed -> st.message
            EngineHolder.State.NotLoaded -> "Not loaded"
        }
        val llm = (st as? EngineHolder.State.Ready)?.llm?.let { l ->
            when (l) {
                EnginePlan.LlmStatus.Off -> null
                EnginePlan.LlmStatus.NoModel -> "AI cleanup: no model"
                is EnginePlan.LlmStatus.Loaded -> "AI cleanup: ${l.name}"
                is EnginePlan.LlmStatus.Failed -> "AI cleanup failed: ${l.message}"
            }
        }
        return listOfNotNull(base, llm).joinToString("\n")
    }

    // =====================================================================================
    // Language
    // =====================================================================================

    private fun renderLanguage() = with(ui) {
        val s = app.settings.load()
        val phone = java.util.Locale.getDefault().getDisplayLanguage(java.util.Locale.ENGLISH)
        screen {
            sectionLabel("FORMATTING")
            card {
                toggleRow("Auto-format", "Punctuation, quotes, fillers and corrections (\"comma\", \"quote … unquote\", \"um\", \"scratch that\", \"five dollars\"); runs with or without AI cleanup. Off = exactly what you said", s.spokenCleanup)
                    .onChange { on -> applyNow { copy(spokenCleanup = on) } }
            }
            sectionLabel("DICTATION LANGUAGE")
            val effective = note("")
            card {
                val choices = SettingsPages.languageChoices(s.language, phone).map { (code, name) ->
                    DevUi.Choice(code, name, if (code == Settings.LANGUAGE_UNSET) "If the speech model supports it, else auto-detect" else "")
                }
                choiceRows(choices, s.language) { code -> applyNow { copy(language = code) } }
            }
            note("Parakeet and *.en whisper models are English only. Changes apply at once.")
            refreshers += {
                val n = app.settings.load()
                effective.text = "Next dictation: " + LanguageDefault.label(n.language, app.effectiveLanguage(n))
            }
        }
        refresh()
    }

    // =====================================================================================
    // Writing style (D27)
    // =====================================================================================

    private fun renderStyle() = with(ui) {
        val s = app.settings.load()
        screen {
            note("The style depends on the app you dictate into. Changes apply at once.")
            for (b in dev.flowclone.data.WritingStyle.BUCKETS) {
                sectionLabel(b.title.uppercase())
                card {
                    val current = b.get(s.styles)
                    choiceRows(
                        dev.flowclone.data.WritingStyle.choicesFor(current).map { DevUi.Choice(it.style, it.name, it.description) },
                        current,
                    ) { st -> applyNow { copy(styles = b.set(styles, st)) } }
                }
                note(b.examples)
            }
            sectionLabel("APPS")
            card {
                val apps = navRow(R.drawable.ic_set_style, "Style per app") { open(Page.APP_STYLES) }
                refreshers += {
                    val n = app.settings.load().appBuckets.size
                    apps.setSubtitle(if (n == 0) "Built-in list only" else if (n == 1) "1 app moved to another group" else "$n apps moved to another group")
                }
            }
        }
        refresh()
    }

    /** Per-app overrides as an app list ([AppStylesPage]); changes apply at once. */
    private fun renderAppStyles() {
        val list = AppStylesPage(this, ui) { f -> applyNow(f) }
        list.render()
        refreshers += list::refresh
    }

    // =====================================================================================
    // Speech engine (on-device builds)
    // =====================================================================================

    private fun renderEngine() = with(ui) {
        val s = app.settings.load()
        screen {
            sectionLabel("ON-DEVICE ENGINE")
            card {
                choiceRows(
                    listOf(
                        DevUi.Choice(AsrEngine.WHISPER, "Whisper", "Many languages; uses the imported speech model"),
                        DevUi.Choice(AsrEngine.PARAKEET, "Parakeet", "English only, more accurate; needs the imported model folder and ~1 GB RAM. Falls back to whisper if it can't load"),
                    ),
                    if (s.asrEngine == AsrEngine.PARAKEET) AsrEngine.PARAKEET else AsrEngine.WHISPER,
                ) { e -> applyNow { copy(asrEngine = e) } }
            }
            note("Changes apply at once and reload the engine.")
            sectionLabel("MODELS")
            card {
                val whisper = navRow(R.drawable.ic_set_mic, "Speech model (whisper)") { startActivity(Intent(this@SettingsActivity, ModelImportActivity::class.java)) }
                val parakeet = navRow(R.drawable.ic_set_mic, "Parakeet model folder") { startActivity(ModelImportActivity.parakeetIntent(this@SettingsActivity)) }
                refreshers += {
                    whisper.setSubtitle(fileName(app.settings.modelPath) ?: "None imported")
                    parakeet.setSubtitle(fileName(app.settings.parakeetPath) ?: "None imported")
                }
            }
        }
    }

    // =====================================================================================
    // AI cleanup (on-device builds)
    // =====================================================================================

    private fun renderCleanup() = with(ui) {
        val s = app.settings.load()
        screen {
            note(
                "Off by default. Formatting rules always run while Auto-format (Language page) is on: spoken punctuation, " +
                    "quotes and brackets, fillers, \"scratch that\", numbers, lists and your writing style. " +
                    "When on, a small language model on this phone also polishes each transcript (run-on sentences, " +
                    "leftover fillers, corrections). It needs an imported GGUF model and adds 1–3 s; it is skipped when the " +
                    "text is already clean, and if it takes longer than 3 s, or changes too much, the formatted transcript " +
                    "is inserted instead. Loading the model takes a little longer than before (its instructions are prepared " +
                    "at load). Dictionary replacements and snippets apply either way.",
            )
            lateinit var strength: DevUi.ChoiceGroup<dev.flowclone.core.CleanupLevel>
            val cloudOn = { dev.flowclone.data.OpenRouterPage.cleanupEngineLine(app.dev.load().effective(caps)) != null }
            card {
                toggleRow("Clean up with local AI", "Off: formatting rules still run. Applies at once and reloads the engine", s.llmEnabled)
                    .onChange { on -> strength.isEnabled = on || cloudOn(); applyNow { copy(llmEnabled = on) } }
                // D28: cloud cleanup is its own opt-in (and consent) on the OpenRouter page.
                if (caps.cloud) {
                    val cloud = navRow(R.drawable.ic_dev_cloud, "Clean up with OpenRouter") { OpenRouterActivity.open(this@SettingsActivity) }
                    refreshers += {
                        cloud.setSubtitle(dev.flowclone.data.OpenRouterPage.cleanupEngineLine(app.dev.load().effective(caps)) ?: "Off · sends the text, not your voice, when on")
                        strength.isEnabled = app.settings.load().llmEnabled || cloudOn()
                    }
                }
            }
            sectionLabel("MODEL")
            card {
                val model = navRow(R.drawable.ic_set_sparkle, "AI cleanup model") { startActivity(ModelImportActivity.llmIntent(this@SettingsActivity)) }
                refreshers += { model.setSubtitle(fileName(app.settings.llmModelPath) ?: "None imported") }
            }
            sectionLabel("STRENGTH")
            card {
                strength = choiceRows(
                    listOf(
                        DevUi.Choice(dev.flowclone.core.CleanupLevel.LIGHT, "Light", "Punctuation and fillers"),
                        DevUi.Choice(dev.flowclone.core.CleanupLevel.MEDIUM, "Medium"),
                        DevUi.Choice(dev.flowclone.core.CleanupLevel.HIGH, "High", "Rewrite for clarity"),
                    ),
                    SettingsPages.shownStrength(s.cleanup),
                ) { l -> applyNow { copy(cleanup = l) } }
                strength.isEnabled = s.llmEnabled || cloudOn()
            }
        }
    }

    // =====================================================================================
    // Timing & limits (Save)
    // =====================================================================================

    private fun renderTiming() = with(ui) {
        val i = draft!!.draft
        screen {
            sectionLabel("HANDS-FREE")
            card {
                fieldRows[Field.ENDPOINT] = fieldRow(
                    "Stop after silence (ms)", i.endpointMs, InputType.TYPE_CLASS_NUMBER,
                    help = "Tap-to-talk ends by itself after this much silence and inserts the text. 300–10000; 0 = only when you tap. Hold-to-talk is not affected.",
                ) { validate() }
            }
            sectionLabel("SPEED")
            card {
                decodeToggle = toggleRow(
                    "Decode while speaking", "Long dictations appear sooner after you stop; on-device and own server, not cloud", i.decodeWhileSpeaking,
                ).also { it.onChange { validate() } }
            }
            sectionLabel("SESSION LIMIT")
            card {
                fieldRows[Field.CAP] = fieldRow("Longest dictation (minutes)", i.capMinutes, InputType.TYPE_CLASS_NUMBER, help = "1–20") { validate() }
            }
            note("Save is in the top bar.")
        }
    }

    // =====================================================================================
    // Bubble
    // =====================================================================================

    private fun renderBubble() = with(ui) {
        val s = app.settings.load()
        screen {
            card {
                toggleRow("Live preview", "Shows the transcript while you speak (uses more battery)", s.livePreview)
                    .onChange { on -> applyNow { copy(livePreview = on) } }
                lateinit var dot: DevUi.ToggleRow
                toggleRow("Shrink bubble", "After 5 s without use", s.shrinkBubble)
                    .onChange { on -> dot.isEnabled = on; applyNow { copy(shrinkBubble = on) } }
                dot = toggleRow("Shrink to a dot", "Shrinks to a small dot instead of a smaller bubble; still easy to tap", s.shrinkToDot, enabled = s.shrinkBubble)
                    .also { it.onChange { on -> applyNow { copy(shrinkToDot = on) } } }
                toggleRow("Shake to restore", "Shake the phone to bring a snoozed or hidden bubble back", s.shakeToReveal)
                    .onChange { on -> applyNow { copy(shakeToReveal = on) } }
            }
            card {
                sliderRow(
                    "Bubble size", "Buttons and the waveform grow with it",
                    min = Settings.BUBBLE_SIZE_PERCENT.first, max = Settings.BUBBLE_SIZE_PERCENT.last,
                    step = Settings.BUBBLE_SIZE_STEP_PERCENT,
                    value = SettingsSummary.bubbleSizePercent(s.bubbleScale),
                ) { "$it%" }.onChange { pct -> applyNow { copy(bubbleScale = Settings.bubbleScaleOf(pct / 100f)) } }
            }
            note("Drag the bubble onto the target at the bottom of the screen to snooze it for 10 minutes. Changes apply at once.")
        }
    }

    // =====================================================================================
    // Excluded apps (Save)
    // =====================================================================================

    private fun renderApps() = with(ui) {
        val i = draft!!.draft
        screen {
            note("The bubble stays hidden in these apps, in addition to the built-in banking and payment list.")
            card {
                fieldRows[Field.DENYLIST] = fieldRow(
                    "Package names, one per line", i.denylist,
                    InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
                    help = "Find an app's package name in Android Settings → Apps → the app → App details, or its Play Store link (id=…).",
                    hint = "com.example.app", minLines = 4,
                ) { validate() }
            }
            note("Save is in the top bar.")
        }
    }

    // =====================================================================================
    // History
    // =====================================================================================

    private fun renderHistory() = with(ui) {
        val s = app.settings.load()
        screen {
            sectionLabel("KEEP DICTATIONS")
            lateinit var group: DevUi.ChoiceGroup<Retention>
            card {
                group = choiceRows(
                    listOf(
                        DevUi.Choice(Retention.KEEP, "Keep", "Until you delete them in History"),
                        DevUi.Choice(Retention.DAY, "24 hours", "Older dictations are deleted"),
                        DevUi.Choice(Retention.NEVER, "Never save", "Nothing is written; saved history is deleted"),
                    ),
                    s.retention,
                ) { r ->
                    val old = app.settings.load().retention
                    if (!SettingsPages.deletesHistory(old, r)) applyNow { copy(retention = r) }
                    else AlertDialog.Builder(this@SettingsActivity)
                        .setTitle(if (r == Retention.NEVER) "Delete all history?" else "Delete older history?")
                        .setMessage(if (r == Retention.NEVER) "Every saved dictation is deleted now, and new ones are not saved." else "Dictations older than 24 hours are deleted now.")
                        .setNegativeButton("Cancel") { _, _ -> group.select(old) }
                        .setOnCancelListener { group.select(old) }
                        .setPositiveButton("Delete") { _, _ -> applyNow { copy(retention = r) } }
                        .show()
                }
            }
            note("History stays on this phone. Changes apply at once.")
        }
    }

    companion object {
        const val EXTRA_PAGE = "page"
        private const val MENU_SAVE = 1

        fun intent(ctx: Context, p: Page = Page.HUB): Intent = Intent(ctx, SettingsActivity::class.java).putExtra(EXTRA_PAGE, p.name)

        fun open(ctx: Context, p: Page) = ctx.startActivity(intent(ctx, p))
    }
}
