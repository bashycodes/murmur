package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.View
import android.widget.LinearLayout
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.murmur.EngineHolder
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.AsrEngine
import dev.murmur.core.HealthReport
import dev.murmur.data.Backup
import dev.murmur.data.BackupIo
import dev.murmur.data.CloudUsage
import dev.murmur.data.DevPages
import dev.murmur.data.DevSettingsForm
import dev.murmur.data.DevUnlock
import dev.murmur.data.OpenRouter
import dev.murmur.data.LanguageDefault
import dev.murmur.data.Retention
import dev.murmur.data.RetryLogic
import dev.murmur.data.Settings
import dev.murmur.data.SettingsDraft
import dev.murmur.data.SettingsPages
import dev.murmur.data.SettingsPages.Field
import dev.murmur.data.SettingsPages.Page
import dev.murmur.data.SettingsSummary
import dev.murmur.data.ShellPages
import java.io.File
import kotlin.concurrent.thread

/**
 * Settings (docs/design/app-design/AndroidSettings.dc.html) as a hub + focused sub-pages, one
 * activity instance per page ([EXTRA_PAGE]), each under the design's 64 dp top bar (back, title,
 * Save as the trailing action on form pages):
 *   hub       General (language, writing style, bubble size, excluded apps, timing), Models (speech
 *             to text, AI cleanup), Connections (your server's latency from the router's health
 *             probe, OpenRouter's key + this month's cost), Data & privacy (keep history, learn
 *             from my edits, debug log), back up & restore, Developer settings + Mock engine,
 *             About (version, 7-tap unlock). Warnings sit in a callout at the top.
 *   language  dictation language, auto-format
 *   style     writing style per bucket (personal / work / email / other), D27
 *   app_styles  per-app bucket overrides: app list + "Add app" picker, from the style page
 *   speech    ranked engines + the phone's model (AndroidSpeech.dc.html, [SpeechPage])
 *   engine    whisper / Parakeet, model imports, from Speech to text           (not in `remote`)
 *   cleanup   cleanup engines in try order (D36, [CleanupPage]), GGUF model, strength (not in `remote`)
 *   timing    hands-free stop after silence, session limit                               [Save]
 *   bubble    live preview, shrink to a dot, shake to restore, bubble size, Command Mode
 *   apps      excluded apps (package names)                                              [Save]
 *   history   retention (asks before deleting)
 * The design's "Bubble opacity" and "Use what's on screen" rows are left out: no such settings.
 * Save rule ([SettingsPages]): pages with typed fields have Save in the top bar (changed + valid,
 * Discard prompt); switches and choices apply at once. Same prefs keys and [Settings] as before.
 */
class SettingsActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    private lateinit var page: Page
    private var draft: SettingsDraft? = null
    private val fieldRows = LinkedHashMap<Field, DevUi.FieldRow>()
    private var saveButton: View? = null
    private var backCallback: OnBackInvokedCallback? = null
    private val refreshers = ArrayList<() -> Unit>()
    /** Hub warnings callout: the engine loads / reloads while the page is open, so it is polled. */
    private var status: (() -> Unit)? = null
    private var speech: SpeechPage? = null
    private var cleanupEngines: CleanupPage? = null
    /** The router's last server health probe (Your server row, Speech to text, AI cleanup's Engines). */
    private var health: HealthReport? = null
    private var checking = false
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
        if (SettingsPages.hasForm(page)) draft = SettingsDraft.of(app.settings.load(), page)
        when (page) {
            Page.HUB -> renderHub()
            Page.LANGUAGE -> renderLanguage()
            Page.STYLE -> renderStyle()
            Page.APP_STYLES -> renderAppStyles()
            Page.SPEECH -> renderSpeech()
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
        if (page == Page.HUB || page == Page.SPEECH || page == Page.CLEANUP) checkServer()
    }

    override fun onPause() {
        main.removeCallbacks(poll)
        super.onPause()
    }

    private fun refresh() { for (r in refreshers) r() }

    private fun open(p: Page) = open(this, p)

    private fun fileName(path: String?) = path?.let { File(it).name }

    /**
     * This page's column under the top bar: back (asks first with unsaved edits) and, on a form
     * page, Save as the trailing action (enabled once something changed and everything validates).
     */
    private fun page(build: LinearLayout.() -> Unit): LinearLayout = with(ui) {
        screen {
            val bar = topBar(
                SettingsPages.TITLES.getValue(page), DevUi.TopNav.BACK, onNav = ::onBack,
                actions = if (draft != null) listOf(DevUi.TopAction(R.drawable.ic_lw_check, "Save ${SettingsPages.TITLES.getValue(page)}") { onSave() }) else emptyList(),
            )
            saveButton = bar.actions.firstOrNull()
            build()
        }
    }

    private fun onBack() { if (draft != null) confirmLeave() else finish() }

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
        if (e.pruneRecordings) thread(name = "history-recordings") { runCatching { app.history.pruneAudio(n.retryAudioKeep) } }
        if (e.reloadEngine) app.engine.reload()
        // H5: Hebrew with a Hebrew model installed loads that model instead (one model in RAM).
        else if (n.language != old.language) app.engine.ensureLanguage()
        if (n.llmEnabled && !old.llmEnabled && app.settings.llmModelPath == null) {
            Toast.makeText(this, "AI cleanup needs a GGUF model: import one first.", Toast.LENGTH_LONG).show()
        }
    }

    /**
     * Your server's state: the router's cached health when it is recent, else one probe of the
     * saved config (off the main thread; a few bytes over your LAN / VPN).
     */
    private fun checkServer() {
        val d = app.dev.load()
        // Also when only AI cleanup uses the server (its Engines row shows this state too).
        val used = d.allowServer || d.cleanupOn(dev.murmur.data.DevSettings.CleanupRoute.SERVER)
        if (!caps.remote || !used || !d.serverConfigured || checking) return
        val r = app.engine.router() ?: return
        checking = true
        refreshServer()
        thread(name = "settings-health") {
            var h = runCatching { r.routeStatus().health }.getOrNull()
            val fresh = h?.checkedAtMs?.let { System.currentTimeMillis() - it < HEALTH_MAX_AGE_MS } == true
            if (!fresh) h = runCatching { r.probeSaved() }.getOrNull() ?: h
            runOnUiThread {
                checking = false
                if (h != null) health = h
                if (alive()) refreshServer()
            }
        }
    }

    private fun refreshServer() {
        speech?.let { it.health = health; it.checking = checking; it.refresh() }
        cleanupEngines?.let { it.health = health; it.checking = checking; it.refresh() }
        refresh()
    }

    // ---- Save (pages with typed fields) ----

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
        saveButton?.let { if (it.isEnabled != can) { it.isEnabled = can; it.alpha = if (can) 1f else 0.35f } }
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
    // Hub (AndroidSettings.dc.html)
    // =====================================================================================

    /** A row in a card of its own (the design's General / Models / Connections rows). */
    private fun LinearLayout.rowCard(title: String, onClick: () -> Unit): DevUi.NavRow = with(ui) {
        lateinit var r: DevUi.NavRow
        card { r = navRow(null, title, onClick = onClick) }
        r
    }

    private fun renderHub() = with(ui) {
        page {
            val warn = callout("").apply { visibility = View.GONE; accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }
            status = {
                val s = app.settings.load()
                val w = SettingsSummary.warnings(
                    s, caps, whisper = fileName(app.settings.modelPath), parakeet = fileName(app.settings.parakeetPath), llm = fileName(app.settings.llmModelPath),
                ) + listOfNotNull((app.engine.state.value as? EngineHolder.State.Failed)?.message) +
                    DevPages.warningsFor(DevPages.Page.HUB, DevSettingsForm.validate(DevSettingsForm.Input.of(app.dev.load()), caps).warnings).map { DevPages.NO_ENGINE }
                // Only touch the view when it changed (TalkBack / scroll position stay put while polling).
                val t = w.joinToString("\n")
                if (warn.text.toString() != t) warn.text = t
                warn.visibility = if (w.isEmpty()) View.GONE else View.VISIBLE
            }

            sectionLabel("General")
            val language = rowCard(SettingsPages.TITLES.getValue(Page.LANGUAGE)) { open(Page.LANGUAGE) }
            val style = rowCard("Writing style") { open(Page.STYLE) }
            val bubble = rowCard("Bubble size") { open(Page.BUBBLE) }
            val apps = rowCard("Excluded apps") { open(Page.APPS) }
            val timing = rowCard("Timing and limits") { open(Page.TIMING) }

            sectionLabel("Models")
            val speechRow = rowCard("Speech to text") { open(Page.SPEECH) }
            // `remote` has no on-device AI: its cleanup is OpenRouter's (D28).
            val cleanup = when (val d = ShellPages.cleanupDest(caps)) {
                is ShellPages.Dest.Page -> rowCard("AI cleanup") { open(d.page) }
                ShellPages.Dest.OpenRouter -> rowCard("AI cleanup") { OpenRouterActivity.open(this@SettingsActivity) }
                null -> null
            }

            var server: DevUi.NavRow? = null
            var openRouter: DevUi.NavRow? = null
            if (caps.remote || caps.cloud) {
                sectionLabel("Connections")
                if (caps.remote) server = rowCard("Your server") { DeveloperActivity.open(this@SettingsActivity, DevPages.Page.SERVER) }
                if (caps.cloud) openRouter = rowCard(OpenRouter.NAME) { OpenRouterActivity.open(this@SettingsActivity) }
            }

            sectionLabel("Data & privacy")
            lateinit var history: DevUi.NavRow
            lateinit var debug: DevUi.ToggleRow
            card {
                history = navRow(null, "Keep history") { open(Page.HISTORY) }
                toggleRow(
                    "Learn from my edits",
                    "After a dictation, re-reads that field for up to 45 s; words you correct show up as suggestions in Dictionary. Never password fields; nothing is added until you tap Add",
                    app.settings.load().learnFromEdits,
                ).onChange { on -> applyNow { copy(learnFromEdits = on) } }
                debug = toggleRow("Debug log", SettingsSummary.DEBUG_LOG_DESC, app.dev.debugLog)
                debug.onChange { on -> app.dev.debugLog = on }
            }

            refreshers += {
                val s = app.settings.load()
                val d = app.dev.load()
                language.setSubtitle(SettingsSummary.language(s, app.effectiveLanguage(s)))
                style.setSubtitle(dev.murmur.data.WritingStyle.summary(s))
                bubble.setSubtitle(SettingsSummary.bubble(s))
                apps.setSubtitle(SettingsSummary.apps(s))
                timing.setSubtitle(SettingsSummary.timing(s))
                speechRow.setSubtitle(SettingsSummary.speech(d, caps))
                cleanup?.setSubtitle(SettingsSummary.cleanupRow(
                    s, d.effective(caps), caps,
                    serverLlm = if (d.serverConfigured) app.engine.router()?.cleanupStatus()?.serverLlm else null,
                ))
                server?.setSubtitle(SettingsSummary.server(d, caps, health, checking))
                openRouter?.setSubtitle(
                    SettingsSummary.openRouter(app.cloudKeys.has(OpenRouter.ID), CloudUsage.decode(app.engine.router()?.cloudUsageJson() ?: app.dev.cloudUsageJson)),
                )
                history.setSubtitle(SettingsSummary.keepHistory(s.retention))
                if (debug.isChecked != app.dev.debugLog) debug.isChecked = app.dev.debugLog
            }

            sectionLabel("Back up & restore")
            card {
                navRow(R.drawable.ic_scr_share, "Export all data", "Notes, History, Dictionary, Insights and settings in one file", chevron = false) { exportBackup() }
                navRow(R.drawable.ic_scr_import, "Restore from a backup", "Replaces this app's data with a backup file", chevron = false) { pickBackup() }
            }
            note(Backup.SECRETS_NOTE)

            sectionLabel("Advanced")
            card {
                if (app.dev.visible) {
                    navRow(null, "Developer settings", "Server, engine options, debug log, status") {
                        startActivity(Intent(this@SettingsActivity, DeveloperActivity::class.java))
                    }
                }
                val mock = toggleRow("Mock engine", "Inserts a fixed sentence; for testing without a model", app.settings.load().mockEngine)
                mock.onChange { on -> applyNow { copy(mockEngine = on) } }
            }

            sectionLabel("About")
            card {
                // §5 Access: in builds without `remote`, 7 taps here reveal Developer settings.
                val unlock = DevUnlock({ android.os.SystemClock.elapsedRealtime() })
                navRow(null, versionLabel(), chevron = false) {
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

    // =====================================================================================
    // Speech to text (AndroidSpeech.dc.html)
    // =====================================================================================

    private fun renderSpeech() {
        val sp = SpeechPage(this, ui, update = { f -> applyNow(f) }, openEngine = { open(Page.ENGINE) })
        speech = sp
        val col = page { }
        sp.render(col)
        refreshers += sp::refresh
    }

    // =====================================================================================
    // Back up & restore (SAF: the user picks the file; no storage permission)
    // =====================================================================================

    @Suppress("DEPRECATION")
    private fun exportBackup() = startActivityForResult(
        Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("application/json")
            .putExtra(Intent.EXTRA_TITLE, Backup.fileName(java.time.LocalDate.now())),
        REQ_BACKUP_EXPORT,
    )

    @Suppress("DEPRECATION")
    private fun pickBackup() = startActivityForResult(
        Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*"), REQ_BACKUP_IMPORT,
    )

    @Deprecated("platform Activity result API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        val uri = data?.data ?: return
        if (resultCode != RESULT_OK) return
        when (requestCode) {
            REQ_BACKUP_EXPORT -> writeBackup(uri)
            REQ_BACKUP_IMPORT -> readBackup(uri)
        }
    }

    private fun alive() = !isFinishing && !isDestroyed

    private fun writeBackup(uri: android.net.Uri) {
        Toast.makeText(this, "Exporting…", Toast.LENGTH_SHORT).show()
        thread(name = "backup-export") {
            val r = runCatching {
                val json = BackupIo(app).export()
                contentResolver.openOutputStream(uri, "wt")!!.use { it.write(json.toByteArray(Charsets.UTF_8)) }
            }
            runOnUiThread {
                val msg = r.fold({ "Backup saved. API keys and the server token are not in it." }, { "Export failed: ${it.message}" })
                Toast.makeText(this, msg, Toast.LENGTH_LONG).show()
            }
        }
    }

    /** Parse and check the whole file first; nothing is written until the user confirms. */
    private fun readBackup(uri: android.net.Uri) {
        thread(name = "backup-read") {
            val parsed = runCatching {
                contentResolver.openInputStream(uri)!!.use { input ->
                    val bytes = input.readNBytes(Backup.MAX_BYTES + 1)
                    if (bytes.size > Backup.MAX_BYTES) Backup.Parsed.Rejected("This file is too large to be a Murmur backup.")
                    else Backup.decode(bytes.toString(Charsets.UTF_8))
                }
            }.getOrElse { Backup.Parsed.Rejected("Couldn't read the file: ${it.message}") }
            runOnUiThread { if (alive()) confirmRestore(parsed) }
        }
    }

    private fun confirmRestore(p: Backup.Parsed) {
        when (p) {
            is Backup.Parsed.Rejected -> AlertDialog.Builder(this)
                .setTitle("Can't restore this file")
                .setMessage(p.reason + "\n\nNothing was changed.")
                .setPositiveButton("OK", null)
                .show()
            is Backup.Parsed.Ok -> AlertDialog.Builder(this)
                .setTitle("Replace your data with this backup?")
                .setMessage(
                    Backup.summary(p.meta, p.data).joinToString("\n") +
                        "\n\nWhat this backup holds replaces what the app has now; this can't be undone. " + Backup.SECRETS_NOTE,
                )
                .setNegativeButton("Cancel", null)
                .setPositiveButton("Replace") { _, _ -> restore(p.data) }
                .show()
        }
    }

    private fun restore(d: Backup.Data) {
        Toast.makeText(this, "Restoring…", Toast.LENGTH_SHORT).show()
        thread(name = "backup-restore") {
            val r = runCatching { BackupIo(app).restore(d) }
            runOnUiThread {
                if (r.isSuccess) {
                    Toast.makeText(this, "Restored. Enter your API keys again if you use them.", Toast.LENGTH_LONG).show()
                    if (alive()) recreate()
                } else if (alive()) {
                    AlertDialog.Builder(this)
                        .setTitle("Restore failed")
                        .setMessage("${r.exceptionOrNull()?.message}\n\nPart of your data may already be restored. The backup file is unchanged: restore it again to finish.")
                        .setPositiveButton("OK", null)
                        .show()
                }
            }
        }
    }

    /** The About row: the same build label as the main screen (D26), e.g. `Version 0.2.0 (hybrid) · 1a2b3c4`. */
    private fun versionLabel() = dev.murmur.data.AppVersion.current.label(app.caps.flavor)

    // =====================================================================================
    // Language
    // =====================================================================================

    private fun renderLanguage() = with(ui) {
        val s = app.settings.load()
        val phone = java.util.Locale.getDefault().getDisplayLanguage(java.util.Locale.ENGLISH)
        page {
            sectionLabel("FORMATTING")
            card {
                toggleRow("Auto-format", "Punctuation, quotes, fillers and corrections (\"comma\", \"quote … unquote\", \"um\", \"scratch that\", \"five dollars\"); runs with or without AI cleanup. Off = exactly what you said", s.spokenCleanup)
                    .onChange { on -> applyNow { copy(spokenCleanup = on) } }
            }
            sectionLabel("DICTATION LANGUAGE")
            val effective = note("")
            card {
                val choices = SettingsPages.languageChoices(s.language, phone, app.keyboardLanguage()).map { (code, name) ->
                    DevUi.Choice(
                        code, name,
                        when (code) {
                            Settings.LANGUAGE_KEYBOARD -> "Follows the keyboard you type with (e.g. Hebrew on Gboard → Hebrew), read when you start dictating"
                            Settings.LANGUAGE_UNSET -> "If the speech model supports it, else auto-detect"
                            else -> ""
                        },
                    )
                }
                choiceRows(choices, s.language) { code -> applyNow { copy(language = code) } }
            }
            note("Parakeet and *.en whisper models are English only. Changes apply at once.")
            val hebrew = note("")
            refreshers += {
                val n = app.settings.load()
                val r = app.resolveLanguage(n)
                val eff = r.language
                effective.text = "Next dictation: " + LanguageDefault.label(n.language, eff, r.source)
                val h = SettingsPages.hebrewNote(eff, app.engine.hebrewModel()?.second?.label, caps.local)
                hebrew.text = h.orEmpty()
                hebrew.visibility = if (h == null) View.GONE else View.VISIBLE
            }
        }
        refresh()
    }

    // =====================================================================================
    // Writing style (D27)
    // =====================================================================================

    private fun renderStyle() = with(ui) {
        val s = app.settings.load()
        page {
            note("The style depends on the app you dictate into. Changes apply at once.")
            for (b in dev.murmur.data.WritingStyle.BUCKETS) {
                sectionLabel(b.title.uppercase())
                card {
                    val current = b.get(s.styles)
                    choiceRows(
                        dev.murmur.data.WritingStyle.choicesFor(current).map { DevUi.Choice(it.style, it.name, it.description) },
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
        list.render { with(ui) { topBar(SettingsPages.TITLES.getValue(page), DevUi.TopNav.BACK, onNav = ::onBack) } }
        refreshers += list::refresh
    }

    // =====================================================================================
    // Speech engine (on-device builds)
    // =====================================================================================

    private fun renderEngine() = with(ui) {
        val s = app.settings.load()
        page {
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
                val hebrew = navRow(R.drawable.ic_set_language, "Hebrew model") { startActivity(ModelImportActivity.hebrewIntent(this@SettingsActivity)) }
                refreshers += {
                    whisper.setSubtitle(fileName(app.settings.modelPath) ?: "None imported")
                    parakeet.setSubtitle(fileName(app.settings.parakeetPath) ?: "None imported")
                    hebrew.setSubtitle(SettingsSummary.hebrewModel(app.engine.hebrewModel()?.second?.label))
                }
            }
        }
    }

    // =====================================================================================
    // AI cleanup (on-device builds)
    // =====================================================================================

    private fun renderCleanup() = with(ui) {
        val s = app.settings.load()
        page {
            note(
                "Formatting rules always run while Auto-format (Language page) is on: spoken punctuation, " +
                    "quotes and brackets, fillers, \"scratch that\", numbers, lists and your writing style. " +
                    "AI cleanup also polishes each transcript (run-on sentences, leftover fillers, corrections) with the " +
                    "first engine below that is on and ready: a small model on this phone (needs an imported GGUF model, adds " +
                    "1–3 s), your server's, or OpenRouter's. It is skipped when the text is already clean, and if it takes too " +
                    "long, or changes too much, the formatted transcript is inserted instead. Dictionary replacements and " +
                    "snippets apply either way.",
            )
            lateinit var strength: DevUi.ChoiceGroup<dev.murmur.core.CleanupLevel>
            // D36: the engines (this phone, your server, OpenRouter) in try order, as on Speech to text.
            val engines = CleanupPage(this@SettingsActivity, ui, update = { f -> applyNow(f) })
                .also { it.health = health; it.checking = checking; cleanupEngines = it }
            engines.render(this)
            val anyOn = { app.settings.load().llmEnabled || app.dev.load().effective(caps).offPhoneCleanup }
            refreshers += {
                engines.refresh()
                strength.isEnabled = anyOn()
            }
            // The phone model loads (and the server reports its AI) while the page is open.
            status = { engines.refresh() }
            sectionLabel("MODEL")
            card {
                val model = navRow(R.drawable.ic_set_sparkle, "AI cleanup model") { startActivity(ModelImportActivity.llmIntent(this@SettingsActivity)) }
                refreshers += { model.setSubtitle(fileName(app.settings.llmModelPath) ?: "None imported") }
            }
            sectionLabel("STRENGTH")
            card {
                strength = choiceRows(
                    listOf(
                        DevUi.Choice(dev.murmur.core.CleanupLevel.LIGHT, "Light", "Punctuation and fillers"),
                        DevUi.Choice(dev.murmur.core.CleanupLevel.MEDIUM, "Medium"),
                        DevUi.Choice(dev.murmur.core.CleanupLevel.HIGH, "High", "Rewrite for clarity"),
                    ),
                    SettingsPages.shownStrength(s.cleanup),
                ) { l -> applyNow { copy(cleanup = l) } }
                strength.isEnabled = anyOn()
            }
        }
    }

    // =====================================================================================
    // Timing & limits (Save)
    // =====================================================================================

    private fun renderTiming() = with(ui) {
        val i = draft!!.draft
        page {
            sectionLabel("HANDS-FREE")
            card {
                fieldRows[Field.ENDPOINT] = fieldRow(
                    "Stop after silence (ms)", i.endpointMs, InputType.TYPE_CLASS_NUMBER,
                    help = "Tap-to-talk ends by itself after this much silence and inserts the text. 300–10000; 0 = only when you tap. Hold-to-talk is not affected.",
                ) { validate() }
            }
            sectionLabel("SESSION LIMIT")
            card {
                fieldRows[Field.CAP] = fieldRow("Longest dictation (minutes)", i.capMinutes, InputType.TYPE_CLASS_NUMBER, help = "1–20") { validate() }
            }
            note("Save is at the top right.")
        }
    }

    // =====================================================================================
    // Bubble
    // =====================================================================================

    private fun renderBubble() = with(ui) {
        val s = app.settings.load()
        page {
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
            sectionLabel("COMMAND MODE")
            card {
                choiceRows(
                    listOf(
                        DevUi.Choice(dev.murmur.data.CommandButton.WITH_SELECTION, "When text is selected", "Select text, tap ✎ next to the mic and say what to do (\"make it shorter\")"),
                        DevUi.Choice(dev.murmur.data.CommandButton.ALWAYS, "Always", "Also without a selection: say what to write at the cursor (the bubble no longer shrinks)"),
                        DevUi.Choice(dev.murmur.data.CommandButton.OFF, "Off", "No ✎ button"),
                    ),
                    s.commandButton,
                ) { v -> applyNow { copy(commandButton = v) } }
            }
            note("Command Mode uses AI cleanup: the on-device model, or OpenRouter only when \"send text around the cursor\" is on. Never in password, sensitive or banking fields.")
            note("Drag the bubble onto the target at the bottom of the screen to snooze it for 10 minutes. Changes apply at once.")
        }
    }

    // =====================================================================================
    // Excluded apps (Save)
    // =====================================================================================

    private fun renderApps() = with(ui) {
        val i = draft!!.draft
        page {
            note("The bubble stays hidden in these apps, in addition to the built-in banking and payment list.")
            card {
                fieldRows[Field.DENYLIST] = fieldRow(
                    "Package names, one per line", i.denylist,
                    InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
                    help = "Find an app's package name in Android Settings → Apps → the app → App details, or its Play Store link (id=…).",
                    hint = "com.example.app", minLines = 4,
                ) { validate() }
            }
            note("Save is at the top right.")
        }
    }

    // =====================================================================================
    // History
    // =====================================================================================

    private fun renderHistory() = with(ui) {
        val s = app.settings.load()
        page {
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
            sectionLabel("KEEP RECORDINGS FOR RETRY")
            card {
                choiceRows(
                    RetryLogic.KEEP_CHOICES.map { DevUi.Choice(it, RetryLogic.keepLabel(it)) },
                    RetryLogic.KEEP_CHOICES.minByOrNull { kotlin.math.abs(it - s.retryAudioKeep) } ?: RetryLogic.DEFAULT_KEEP,
                ) { n -> applyNow { copy(retryAudioKeep = n) } }
            }
            note(
                "The recordings of your newest dictations stay on this phone, so History can transcribe them " +
                    "again with your current settings. Never from password or protected fields; deleted with their dictation.",
            )
        }
    }

    companion object {
        const val EXTRA_PAGE = "page"
        /** A cached health probe younger than this is shown as is (else the page probes once). */
        private const val HEALTH_MAX_AGE_MS = 60_000L
        private const val REQ_BACKUP_EXPORT = 30
        private const val REQ_BACKUP_IMPORT = 31

        fun intent(ctx: Context, p: Page = Page.HUB): Intent = Intent(ctx, SettingsActivity::class.java).putExtra(EXTRA_PAGE, p.name)

        fun open(ctx: Context, p: Page) = ctx.startActivity(intent(ctx, p))
    }
}
