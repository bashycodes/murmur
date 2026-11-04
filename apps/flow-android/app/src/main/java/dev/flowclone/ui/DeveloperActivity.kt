package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.DevText
import dev.flowclone.core.HealthReport
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.core.ParakeetDecoding
import dev.flowclone.core.SessionConfig
import dev.flowclone.data.CloudText
import dev.flowclone.data.OpenRouter
import dev.flowclone.data.CustomServer
import dev.flowclone.data.DevDraft
import dev.flowclone.data.DevPages
import dev.flowclone.data.DevPages.Page
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevSettingsForm
import dev.flowclone.data.DevSettingsForm.Field
import dev.flowclone.data.DevSummary
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * Developer settings (docs/hybrid-architecture.md §5) as a hub + focused sub-pages, one
 * activity instance per page ([EXTRA_PAGE]), under the design's top bar (back, title, Save):
 *   hub      status card (route in use, last dictation engine + client_ms, circuit) and one
 *            row per page showing its current value; build info, reset, lock
 *   engines  engine badge, Parakeet decoding (both apply at once); Test session. Which engines
 *            are on and their order: Settings → Speech to text (2026-10-30, [SpeechPage])
 *   server   host, port, TLS, pin, token (secret), timeouts; Test connection; with an admin
 *            key (D39) a USERS & KEYS row → [UsersKeysActivity]
 * The cloud (D28: OpenRouter only, speech-to-text and text cleanup) has its own page,
 * [OpenRouterActivity], opened from the hub's CONNECTION card.
 *   debug    debug log, keep last dictation audio (both apply at once, as before)
 *   status   active engine, circuits, server info, native capabilities; Reset circuit
 * The Server page has Save in the top bar, enabled once something changed and the
 * whole form validates. Save writes the whole [DevSettings] like the old single page
 * ([DevDraft], [DevSettingsForm]); settings apply from the next dictation.
 */
class DeveloperActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    private lateinit var page: Page
    private lateinit var draft: DevDraft
    private var saveButton: View? = null
    private val refreshers = ArrayList<() -> Unit>()
    private var backCallback: OnBackInvokedCallback? = null
    private var banner: DevUi.Result? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        page = intent.getStringExtra(EXTRA_PAGE)?.let { n -> Page.entries.firstOrNull { it.name == n } } ?: Page.HUB
        ui = DevUi(this)
        title = TITLES.getValue(page)
        load()
        when (page) {
            Page.HUB -> renderHub()
            Page.ENGINES -> renderEngines()
            Page.SERVER -> renderServer()
            Page.DEBUG -> renderDebug()
            Page.STATUS -> renderStatus()
        }
        if (hasForm) {
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
        // The OpenRouter page / another sub-page may have saved meanwhile.
        val stored = app.dev.load()
        draft = draft.rebase(stored)
        if (hasForm) validate()
        refresh()
    }

    private fun load() {
        draft = DevDraft.of(app.dev.load(), page)
    }

    private val hasForm get() = page == Page.SERVER

    private fun refresh() { for (r in refreshers) r() }

    // ---- top bar (back, Save on the Server page) ----

    private fun page(build: LinearLayout.() -> Unit): LinearLayout = with(ui) {
        screen {
            val bar = topBar(
                TITLES.getValue(page), DevUi.TopNav.BACK, onNav = { if (hasForm) confirmLeave() else finish() },
                actions = if (hasForm) listOf(DevUi.TopAction(R.drawable.ic_lw_check, "Save ${TITLES.getValue(page)}") { onSave() }) else emptyList(),
            )
            saveButton = bar.actions.firstOrNull()
            build()
        }
    }

    /** Unsaved edits: ask before dropping them. */
    private fun confirmLeave() {
        if (!draft.dirty) { finish(); return }
        AlertDialog.Builder(this)
            .setTitle("Discard changes?")
            .setMessage("Your changes on this page are not saved.")
            .setNegativeButton("Keep editing", null)
            .setPositiveButton("Discard") { _, _ -> finish() }
            .show()
    }

    // =====================================================================================
    // Hub
    // =====================================================================================

    private fun renderHub() = with(ui) {
        app.migrateCloud()
        page {
            cloudNotice()
            sectionLabel("STATUS")
            card {
                val route = infoRow("Route in use")
                val last = infoRow("Last dictation")
                val circuit = infoRow("Circuit")
                val warn = note("", p.danger, 14f).apply { visibility = View.GONE; accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }
                navRow(R.drawable.ic_dev_status, "Status & server info", "Engines, circuits, last probe") { open(Page.STATUS) }
                refreshers += {
                    val s = app.dev.load()
                    route.text = DevSummary.routing(s, caps)
                    val l = app.lastRoute.value
                    last.text = l?.let { d ->
                        DevSummary.lastShort(d.result) + " · " +
                            DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(d.atMs))
                    } ?: "No dictation yet"
                    circuit.text = circuitText()
                    val w = DevPages.warningsFor(Page.HUB, DevSettingsForm.validate(DevSettingsForm.Input.of(s), caps).warnings)
                    warn.text = w.joinToString("\n"); warn.visibility = if (w.isEmpty()) View.GONE else View.VISIBLE
                }
            }

            sectionLabel("DICTATION")
            card {
                val speech = navRow(R.drawable.ic_dev_route, "Speech to text", "") { SettingsActivity.open(this@DeveloperActivity, dev.flowclone.data.SettingsPages.Page.SPEECH) }
                val engines = navRow(R.drawable.ic_dev_engine, TITLES.getValue(Page.ENGINES)) { open(Page.ENGINES) }
                refreshers += {
                    speech.setSubtitle("Engines and order · " + DevSummary.routing(app.dev.load(), caps))
                    engines.setSubtitle(DevSummary.engineOptions(app.dev.load().showEngineBadge, if (caps.local) app.dev.parakeetDecoding.wire else null))
                }
            }

            sectionLabel("CONNECTION")
            card {
                val server = navRow(R.drawable.ic_dev_server, "Server") { open(Page.SERVER) }
                val cloud = navRow(R.drawable.ic_dev_cloud, OpenRouterActivity.TITLE) { OpenRouterActivity.open(this@DeveloperActivity) }
                refreshers += {
                    val s = app.dev.load()
                    server.setSubtitle(DevSummary.server(s, caps) + if (caps.remote && s.serverConfigured) "\n" + DevSummary.timeouts(s) else "")
                    cloud.setSubtitle(DevSummary.cloud(s, caps, app.cloudKeys.has(OpenRouter.ID)))
                }
            }

            sectionLabel("DIAGNOSTICS")
            card {
                val debug = navRow(R.drawable.ic_dev_debug, "Debug log & audio") { open(Page.DEBUG) }
                refreshers += { debug.setSubtitle(DevSummary.debug(app.dev.debugLog, app.dev.keepDebugAudio)) }
                // English 1 spike: the probe screen exists only in hybrid debug builds (src/hybridDebug),
                // so the row shows only where the activity resolves. It never touches dictation routing.
                probeIntent()?.let { i ->
                    navRow(R.drawable.ic_dev_debug, "English speech probe", "Gemini Nano (ML Kit GenAI) and the on-device recognizer") { startActivity(i) }
                }
            }

            sectionLabel("BUILD")
            card {
                infoRow("Version", dev.flowclone.data.AppVersion.current.detail())
                infoRow("Flavour · native library", "${caps.flavor} · ${capabilitiesLine()}")
                navRow(R.drawable.ic_dev_reset, "Reset to defaults", "Developer settings and the OpenRouter key", danger = true, chevron = false) { confirmResetDefaults() }
                if (!caps.remote) navRow(R.drawable.ic_dev_lock, "Lock", "Hide Developer settings again", chevron = false) {
                    app.dev.devUnlocked = false
                    Toast.makeText(this@DeveloperActivity, "Developer settings hidden", Toast.LENGTH_SHORT).show()
                    finish()
                }
            }
        }
    }

    private fun open(p: Page) = open(this, p)

    private fun probeIntent(): Intent? = Intent().setClassName(this, PROBE_ACTIVITY)
        .takeIf { dev.flowclone.BuildConfig.DEBUG && packageManager.resolveActivity(it, 0) != null }

    /**
     * D28 one-time notice (another provider was removed / the model changed) until dismissed.
     * A router that migrated a legacy config itself hands its notice over here too.
     */
    private fun LinearLayout.cloudNotice() = with(ui) {
        app.engine.router()?.let { r -> r.cloudNotice()?.let { n -> app.dev.cloudNotice = n; r.ackCloudNotice() } }
        val text = app.dev.cloudNotice ?: return@with
        val b = banner()
        b.show(text, Tone.WARN)
        chipRow(
            chip("Open OpenRouter") { OpenRouterActivity.open(this@DeveloperActivity) },
            chip("Dismiss") { app.dev.cloudNotice = null; b.show(""); (it.parent as? View)?.visibility = View.GONE }.apply { contentDescription = "Dismiss the cloud notice" },
        )
    }

    // =====================================================================================
    // Engine options (engines + order: Settings → Speech to text)
    // =====================================================================================

    private var sessionResult: DevUi.Result? = null

    private fun renderEngines() = with(ui) {
        val s = app.dev.load()
        page {
            note("Which engines are on and their order: Settings → Speech to text.")
            sectionLabel("PREVIEW")
            card {
                toggleRow("Engine badge", "Show S / L on the preview line (• = fallback). Applies at once", s.showEngineBadge)
                    .onChange { on ->
                        app.dev.save(app.dev.load().copy(showEngineBadge = on))
                        app.engine.applyDevSettings(app.dev.load())
                    }
            }
            if (caps.local) parakeetCard()
            action("Test session") { onTestSession() }
            sessionResult = result()
            note("Sends 1 s of silence through the saved routes; expect EMPTY (no_speech).")
        }
    }

    /** D22 Parakeet decoding: applies at once (not part of Save), as before. */
    private fun LinearLayout.parakeetCard() = with(ui) {
        sectionLabel("ON-DEVICE DECODING (PARAKEET)")
        card {
            choiceRows(
                listOf(
                    DevUi.Choice(ParakeetDecoding.AUTO, "Auto", "Beam search for dictionary words (≈ +0.6 GB RAM)"),
                    DevUi.Choice(ParakeetDecoding.GREEDY, "Greedy (default)", "One recognizer, less RAM"),
                    DevUi.Choice(ParakeetDecoding.BEAM, "Beam only", "The old behaviour"),
                ),
                app.dev.parakeetDecoding,
            ) { m ->
                app.dev.parakeetDecoding = m
                if (app.settings.load().asrEngine == dev.flowclone.core.AsrEngine.PARAKEET) app.engine.reload()
                Toast.makeText(this@DeveloperActivity, "Parakeet decoding: ${m.wire}", Toast.LENGTH_SHORT).show()
            }
        }
        note("Applies immediately and reloads the engine.")
    }

    /** 1 s of silence through the full router path with the saved toggles: expect EMPTY no_speech. */
    private fun onTestSession() {
        val line = sessionResult ?: return
        line.show("Test session…")
        thread(name = "dev-test-session") {
            val (text, ok) = try {
                val core = app.engine.current()
                core.startSession(SessionConfig(maxSamples = 16_000L * 10))
                core.pushAudio(ShortArray(16_000))
                val r = core.commit()
                val ok = r.status == CommitStatus.EMPTY
                ((if (ok) "OK: " else "Unexpected: ") + DevText.lastDictation(r)) to ok
            } catch (e: CoreException) {
                app.debugLog.event("test_session", e.message, e)
                "Failed: ${dev.flowclone.core.RouterErrors.userMessage(e.kind, e.message ?: "error")}" to false
            }
            runOnUiThread { line.show(text, if (ok) Tone.OK else Tone.BAD); refresh() }
        }
    }

    // =====================================================================================
    // Server
    // =====================================================================================

    private lateinit var host: DevUi.FieldRow
    private lateinit var port: DevUi.FieldRow
    private lateinit var tls: DevUi.ToggleRow
    private lateinit var pin: DevUi.FieldRow
    private lateinit var token: DevUi.FieldRow
    private lateinit var connectTimeout: DevUi.FieldRow
    private lateinit var commitTimeout: DevUi.FieldRow
    private lateinit var healthTtl: DevUi.FieldRow
    private var probeResult: DevUi.Result? = null
    private var usersKeys: UsersKeysActivity.Entry? = null
    private val fieldRows = HashMap<Field, DevUi.FieldRow>()

    private fun renderServer() = with(ui) {
        val s = app.dev.load()
        val edit = { if (::draft.isInitialized && fieldRows.isNotEmpty()) validate(); Unit }
        page {
            banner = banner()
            if (!caps.remote) note("This build has no network code: the server route is not available.", p.danger, 14f)
            sectionLabel("ADDRESS")
            card {
                host = fieldRow("Host or IP", s.host, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI,
                    help = "No scheme, no port", hint = "100.64.0.10 or desktop.local", onEdit = edit)
                port = fieldRow("Port", s.port.toString(), InputType.TYPE_CLASS_NUMBER, onEdit = edit)
                tls = toggleRow("Use TLS (wss)", "Encrypt the connection", s.tls)
                pin = fieldRow("Certificate pin (optional)", s.tlsPinSha256, InputType.TYPE_CLASS_TEXT,
                    help = "SHA-256 of the leaf certificate, 64 hex", onEdit = edit)
            }
            sectionLabel("AUTHENTICATION")
            card {
                token = secretRow("Token", s.token, "From `flow-server gen-token`, at least ${DevSettings.MIN_TOKEN_LEN} characters", "token", onEdit = edit)
            }
            sectionLabel("TIMEOUTS")
            card {
                connectTimeout = fieldRow("Connect timeout (ms)", s.connectTimeoutMs.toString(), InputType.TYPE_CLASS_NUMBER, help = "100-10000; default ${DevSettings.DEFAULT_CONNECT_TIMEOUT_MS}, raised to RTT × 6 after a Test connection", onEdit = edit)
                commitTimeout = fieldRow("Commit timeout base (ms)", s.commitTimeoutBaseMs.toString(), InputType.TYPE_CLASS_NUMBER, help = "500-60000", onEdit = edit)
                healthTtl = fieldRow("Health cache (s)", s.healthTtlS.toString(), InputType.TYPE_CLASS_NUMBER, help = "5-600", onEdit = edit)
            }
            action("Test connection") { onTestConnection() }
            probeResult = result()
            note("Uses the values on this page, saved or not.")
            // D39: only for an admin key (saved settings); hidden otherwise and in the local flavour.
            usersKeys = UsersKeysActivity.entry(this@DeveloperActivity, ui, this)
        }
        fieldRows += mapOf(
            Field.HOST to host, Field.PORT to port, Field.PIN to pin, Field.TOKEN to token,
            Field.CONNECT_TIMEOUT to connectTimeout, Field.COMMIT_TIMEOUT to commitTimeout, Field.HEALTH_TTL to healthTtl,
        )
        tls.onChange { validate() }
    }

    private fun onTestConnection() {
        val line = probeResult ?: return
        val s = validate().settings ?: run { line.show("Fix the highlighted fields first.", Tone.BAD); return }
        if (!caps.remote) { line.show("This build has no network code.", Tone.BAD); return }
        probe(s)
    }

    /** `probe_server()` with the form's (unsaved) values, off the UI thread. */
    private fun probe(s: DevSettings) {
        probeResult?.show("Testing ${s.host}:${s.port}…")
        val json = s.effective(caps).toServerConfigJson().toString()
        thread(name = "dev-probe") {
            val h = NativeFlowCore.probeServer(json)
            if (!h.ok) app.debugLog.event("server_probe", DevText.probe(h)) else rememberServerModel(h)
            runOnUiThread { lastProbe = h; probeResult?.show(DevText.probe(h), if (h.ok) Tone.OK else Tone.BAD); refresh() }
        }
    }

    /** The debug log's `model` for server dictations: what the server says it runs. */
    private fun rememberServerModel(h: HealthReport) {
        (h.asrModel ?: h.asrBackend)?.let { app.dev.serverAsrModel = it }
    }

    // =====================================================================================
    // Debug log + audio (D21; device-local, includes transcripts). Apply at once, as before.
    // =====================================================================================

    private var debugCount: TextView? = null
    private var audioCount: TextView? = null

    private fun renderDebug() = with(ui) {
        var debugToggle: DevUi.ToggleRow? = null
        page {
            sectionLabel("DEBUG LOG")
            card {
                debugToggle = toggleRow("Debug log", "Includes transcripts", app.dev.debugLog)
                note("Writes what you dictate, the app it went to and the timings to this phone's log (logcat tag FlowDebug) and to a file in the app's storage. Nothing is sent anywhere unless you tap Share.")
                debugCount = infoRow("Entries")
                chipRow(
                    chip("View") { onViewDebugLog() }.apply { contentDescription = "View the debug log" },
                    chip("Share") { onShareDebugLog() }.apply { contentDescription = "Share or export the debug log" },
                    chip("Clear") { clearDebugLog() }.apply { contentDescription = "Clear the debug log" },
                )
            }
            sectionLabel("DICTATION AUDIO")
            card {
                val keep = toggleRow("Keep last dictation audio", "Debug: WAVs of the last ${dev.flowclone.debug.DebugAudioStore.MAX_FILES} dictations", app.dev.keepDebugAudio)
                note(
                    "Saved in the app's private storage (files/debug/audio/), named like the debug log's ts and linked from its \"audio\" field. " +
                        "Never for password fields, banking or excluded apps. Deleted after ${dev.flowclone.debug.DebugAudioStore.MAX_AGE_MS / 3_600_000} hours. Nothing is sent anywhere.",
                )
                audioCount = infoRow("Files")
                chipRow(chip("Clear audio") { clearDebugAudio() }.apply { contentDescription = "Clear kept dictation audio" })
                keep.onChange { on -> app.dev.keepDebugAudio = on; refreshAudioCount() }
            }
            note("These switches apply immediately.")
        }
        debugToggle?.onChange { on -> onDebugToggle(on) }
        refreshers += { refreshDebugCount(); refreshAudioCount() }
    }

    private fun refreshAudioCount() {
        val v = audioCount ?: return
        thread(name = "dev-audio-count") {
            val n = runCatching { app.debugAudio.prune(); app.debugAudio.list().size }.getOrDefault(0)
            val line = "$n file${if (n == 1) "" else "s"} kept" + if (app.dev.keepDebugAudio) "" else " · off"
            runOnUiThread { v.text = line }
        }
    }

    private fun clearDebugAudio() {
        thread(name = "dev-audio-clear") {
            runCatching { app.debugAudio.clear() }
            runOnUiThread { Toast.makeText(this, "Debug audio cleared", Toast.LENGTH_SHORT).show(); refreshAudioCount() }
        }
    }

    private fun onDebugToggle(on: Boolean) {
        app.dev.debugLog = on
        if (on) { refreshDebugCount(); return }
        thread(name = "dev-debug-count") {
            val n = runCatching { app.debugFile.size() }.getOrDefault(0)
            runOnUiThread {
                if (n == 0 || isFinishing) { refreshDebugCount(); return@runOnUiThread }
                AlertDialog.Builder(this)
                    .setTitle("Clear the debug log?")
                    .setMessage("Logging is off. The file still holds $n entries with your transcripts.")
                    .setPositiveButton("Clear") { _, _ -> clearDebugLog() }
                    .setNegativeButton("Keep") { _, _ -> refreshDebugCount() }
                    .show()
            }
        }
    }

    private fun refreshDebugCount() {
        val v = debugCount ?: return
        thread(name = "dev-debug-count") {
            val n = runCatching { app.debugFile.size() }.getOrDefault(0)
            val errs = app.debugLog.errors.get(); val dropped = app.debugLog.dropped.get()
            val extra = listOfNotNull(errs.takeIf { it > 0 }?.let { "$it write errors" }, dropped.takeIf { it > 0 }?.let { "$it dropped" })
            val line = "$n entries (newest ${dev.flowclone.debug.DebugLogFile.MAX_ENTRIES} kept)" + extra.joinToString("") { " · $it" } +
                if (app.dev.debugLog) "" else " · logging off"
            runOnUiThread { v.text = line }
        }
    }

    private fun onViewDebugLog() {
        thread(name = "dev-debug-view") {
            val rows = runCatching { app.debugFile.newestFirst().map(dev.flowclone.debug.DebugFormat::summary) }.getOrElse { listOf("Could not read the log: ${it.message}") }
            runOnUiThread {
                if (isFinishing) return@runOnUiThread
                val tv = TextView(this).apply {
                    text = if (rows.isEmpty()) "No entries yet." else rows.joinToString("\n\n")
                    textSize = 12f; setTextIsSelectable(true); setPadding(dp(16), dp(8), dp(16), dp(8))
                }
                AlertDialog.Builder(this)
                    .setTitle("Debug log (newest first)")
                    .setView(android.widget.ScrollView(this).apply { addView(tv) })
                    .setPositiveButton("Close", null)
                    .show()
            }
        }
    }

    private fun onShareDebugLog() {
        thread(name = "dev-debug-share") {
            val n = runCatching { app.debugFile.size() }.getOrDefault(0)
            runOnUiThread {
                if (n == 0) Toast.makeText(this, "The debug log is empty", Toast.LENGTH_SHORT).show()
                else runCatching { startActivity(DebugLogProvider.shareIntent(this)) }
                    .onFailure { Toast.makeText(this, "No app to share with", Toast.LENGTH_SHORT).show() }
            }
        }
    }

    private fun clearDebugLog() {
        thread(name = "dev-debug-clear") {
            runCatching { app.debugFile.clear() }
            runOnUiThread { Toast.makeText(this, "Debug log cleared", Toast.LENGTH_SHORT).show(); refreshDebugCount() }
        }
    }

    // =====================================================================================
    // Status
    // =====================================================================================

    private fun renderStatus() = with(ui) {
        page {
            sectionLabel("LAST DICTATION")
            lateinit var last: TextView
            card { last = infoRow("Active engine") }
            sectionLabel("CIRCUITS")
            lateinit var circuit: TextView
            var cloudCircuit: TextView? = null
            var cleanupCircuit: TextView? = null
            var cleanup: TextView? = null
            card {
                circuit = infoRow("Server / on-device circuit")
                if (caps.cloud) cloudCircuit = infoRow("Cloud speech-to-text circuit")
                if (caps.cloud) cleanupCircuit = infoRow("Cloud cleanup circuit")
            }
            if (caps.cloud) {
                sectionLabel("CLOUD CLEANUP (OPENROUTER)")
                card { cleanup = infoRow("Status") }
            }
            chipRow(chip("Reset circuit") { onResetCircuit() })
            sectionLabel("SERVER INFO (LAST PROBE)")
            lateinit var info: TextView
            card { info = infoRow("Server") }
            sectionLabel("BUILD")
            card {
                infoRow("Version", dev.flowclone.data.AppVersion.current.detail())
                infoRow("Flavour · native library", "${caps.flavor} · ${capabilitiesLine()}")
            }
            refreshers += {
                val l = app.lastRoute.value
                last.text = DevText.lastDictation(l?.result) +
                    (l?.let { " · " + DateFormat.getTimeInstance().format(Date(it.atMs)) } ?: "")
                circuit.text = circuitText()
                val rs = routeStatus()
                val cc = rs?.cloudCircuit
                cloudCircuit?.text = if (cc == null) "Not loaded" else DevText.circuit(cc, System.currentTimeMillis(), "the key or model changes") { DateFormat.getTimeInstance().format(Date(it)) }
                val lc = rs?.cleanupCircuit
                cleanupCircuit?.text = if (lc == null) "Not loaded" else DevText.circuit(lc, System.currentTimeMillis(), "the key or cleanup settings change") { DateFormat.getTimeInstance().format(Date(it)) }
                cleanup?.text = app.engine.router()?.let { DevText.cleanupStatus(it.cleanupStatus()) } ?: "Not loaded"
                info.text = DevText.serverInfo(lastProbe ?: rs?.health)
            }
        }
    }

    private fun routeStatus() = app.engine.router()?.let { runCatching { it.routeStatus() }.getOrNull() }

    private fun circuitText(): String {
        val rs = routeStatus() ?: return "router not loaded (${app.engine.state.value::class.simpleName})"
        return DevText.circuit(rs.circuit, System.currentTimeMillis()) { DateFormat.getTimeInstance().format(Date(it)) }
    }

    private fun onResetCircuit() {
        val r = app.engine.router()
        if (r == null) { Toast.makeText(this, "No router loaded", Toast.LENGTH_SHORT).show(); return }
        runCatching { r.resetCircuit() }
        refresh()
    }

    private fun capabilitiesLine(): String {
        val c = NativeFlowCore.capabilities() ?: return "native library not loaded"
        val cc = NativeFlowCore.cloudCapabilities()
        val mismatch = if (c.local != caps.local || c.remote != caps.remote || cc.cloud != caps.cloud) " · ⚠ native library flavour mismatch" else ""
        return "native: local=${c.local} remote=${c.remote} cloud=${cc.cloud} asr=${c.asrEngines.joinToString("/").ifEmpty { "-" }} llm=${c.llm} proto=${c.proto} core ${c.coreVersion}$mismatch"
    }

    // =====================================================================================
    // Form: validate / save (the whole DevSettings, as before) / reset
    // =====================================================================================

    /** This page's views → the draft (other pages' values stay as stored). */
    private fun readViews(i: DevSettingsForm.Input): DevSettingsForm.Input = when (page) {
        Page.SERVER -> i.copy(
            host = host.text, port = port.text, tls = tls.isChecked, pin = pin.text, token = token.text,
            connectTimeoutMs = connectTimeout.text, commitTimeoutBaseMs = commitTimeout.text, healthTtlS = healthTtl.text,
        )
        else -> i
    }

    private fun validate(): DevSettingsForm.Result {
        if (!hasForm) return draft.result(caps)
        draft = draft.edit { readViews(this) }
        val r = draft.result(caps)
        for ((f, row) in fieldRows) row.setError(r.errors[f])
        if (page == Page.SERVER) {
            if (r.tlsLocked && !tls.isChecked) { tls.isChecked = true; draft = draft.edit { readViews(this) } }
            tls.isEnabled = !r.tlsLocked
            tls.setDescription(if (r.tlsLocked) "Required for a public address" else "Encrypt the connection")
            pin.edit.isEnabled = tls.isChecked
            pin.edit.alpha = if (tls.isChecked) 1f else 0.5f
        }
        val off = DevPages.offPageErrors(r, page)
        val lines = DevPages.warningsFor(page, r.warnings) + off.map { (f, e) -> "$e (on the ${TITLES.getValue(DevPages.pageOf(f))} page)" }
        banner?.show(lines.joinToString("\n"), if (off.isNotEmpty() || DevSettingsForm.BOTH_OFF in lines) Tone.BAD else Tone.WARN)
        val can = draft.canSave(caps)
        saveButton?.let { if (it.isEnabled != can) { it.isEnabled = can; it.alpha = if (can) 1f else 0.35f } }
        return r
    }

    private fun onSave() {
        val r = validate()
        val s = r.settings ?: return
        if (!draft.dirty) return
        app.dev.save(s)
        app.engine.applyDevSettings(s)
        Toast.makeText(this, if (s.effective(caps).bothOff) DevSettingsForm.BOTH_OFF else "Saved: applies to the next dictation", Toast.LENGTH_LONG).show()
        draft = DevDraft.of(app.dev.load(), page)
        validate()
        usersKeys?.check()
        // §5: a probe runs after each save; its result fills "Server info".
        if (s.effective(caps).allowServer && s.serverConfigured) {
            val router = app.engine.router()
            if (router == null) probe(s) else {
                probeResult?.show("Probing ${s.host}:${s.port}…")
                thread(name = "dev-probe-saved") {
                    val h = runCatching { router.probeSaved() }.getOrElse { HealthReport(ok = false, error = "error", message = it.message) }
                    if (!h.ok) app.debugLog.event("server_probe", DevText.probe(h)) else rememberServerModel(h)
                    runOnUiThread { lastProbe = h; probeResult?.show(DevText.probe(h), if (h.ok) Tone.OK else Tone.BAD); refresh() }
                }
            }
        }
        refresh()
    }

    private fun confirmResetDefaults() {
        AlertDialog.Builder(this)
            .setTitle("Reset Developer settings?")
            .setMessage("Restores the default engines, server and timeouts, turns cloud speech-to-text and cleanup off, and deletes the OpenRouter and custom server API keys from this device.")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Reset") { _, _ -> onResetDefaults() }
            .show()
    }

    /** Also forgets the OpenRouter and custom server keys (sealed copies and their Keystore keys). */
    private fun onResetDefaults() {
        app.cloudKeys.remove(OpenRouter.ID); app.engine.setCloudKey(null)
        app.cloudKeys.remove(CustomServer.ID); app.engine.setCloudKey(null, CustomServer.ID)
        app.dev.reset()
        app.engine.applyDevSettings(app.dev.load())
        lastProbe = null
        recreate()
    }

    companion object {
        const val EXTRA_PAGE = "page"
        private const val PROBE_ACTIVITY = "dev.flowclone.probe.EnglishProbeActivity"

        val TITLES = mapOf(
            Page.HUB to "Developer settings", Page.ENGINES to "Engine options", Page.SERVER to "Server",
            Page.DEBUG to "Debug log & audio", Page.STATUS to "Status",
        )

        /** Last server probe of this process (Status page "Server info"); else the router's cached health. */
        @Volatile private var lastProbe: HealthReport? = null

        fun open(ctx: Context, p: Page) = ctx.startActivity(Intent(ctx, DeveloperActivity::class.java).putExtra(EXTRA_PAGE, p.name))

        private fun String.cap() = replaceFirstChar { it.uppercase() }
    }
}
