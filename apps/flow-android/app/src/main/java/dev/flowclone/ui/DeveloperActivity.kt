package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.Gravity
import android.view.Menu
import android.view.MenuItem
import android.view.View
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
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
import dev.flowclone.data.CloudKeyPolicy
import dev.flowclone.data.CloudPresets
import dev.flowclone.data.CloudSettings
import dev.flowclone.data.CloudText
import dev.flowclone.data.CloudUsage
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
 * activity instance per page ([EXTRA_PAGE]):
 *   hub      status card (route in use, last dictation engine + client_ms, circuit) and one
 *            row per page showing its current value; build info, reset, lock
 *   routing  engines, route order, keep warm, engine badge, Parakeet decoding; Test session
 *   server   host, port, TLS, pin, token (secret), timeouts; Test connection
 *   cloud    allow cloud (consent), providers, budget, usage; Test active provider
 *   debug    debug log, keep last dictation audio (both apply at once, as before)
 *   status   active engine, circuits, server info, native capabilities; Reset circuit
 * Pages with form fields have Save in the top bar, enabled once something changed and the
 * whole form validates. Save writes the whole [DevSettings] like the old single page
 * ([DevDraft], [DevSettingsForm]); settings apply from the next dictation.
 */
class DeveloperActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    private lateinit var page: Page
    private lateinit var draft: DevDraft
    private var saveItem: MenuItem? = null
    private var routeOrder = DevSettings.DEFAULT_ORDER
    /** Providers + active id as stored (the provider page and consent dialog save them directly). */
    private var cloudState = CloudSettings()
    private val refreshers = ArrayList<() -> Unit>()
    private var backCallback: OnBackInvokedCallback? = null
    private var banner: DevUi.Result? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        page = intent.getStringExtra(EXTRA_PAGE)?.let { n -> Page.entries.firstOrNull { it.name == n } } ?: Page.HUB
        ui = DevUi(this)
        title = TITLES.getValue(page)
        if (page != Page.HUB) actionBar?.setDisplayHomeAsUpEnabled(true)
        load()
        when (page) {
            Page.HUB -> renderHub()
            Page.ROUTING -> renderRouting()
            Page.SERVER -> renderServer()
            Page.CLOUD -> renderCloud()
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
        // The provider page / consent dialog / another sub-page may have saved meanwhile.
        val stored = app.dev.load()
        draft = draft.rebase(stored)
        cloudState = stored.cloud
        if (hasForm) validate()
        refresh()
    }

    private fun load() {
        val s = app.dev.load()
        draft = DevDraft.of(s, page)
        routeOrder = s.routeOrder
        cloudState = s.cloud
    }

    private val hasForm get() = page == Page.ROUTING || page == Page.SERVER || (page == Page.CLOUD && caps.cloud)

    private fun refresh() { for (r in refreshers) r() }

    // ---- top bar Save ----

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        if (!hasForm) return false
        saveItem = menu.add(Menu.NONE, MENU_SAVE, Menu.NONE, "Save").apply {
            setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS or MenuItem.SHOW_AS_ACTION_WITH_TEXT)
            isEnabled = draft.canSave(caps)
            contentDescription = "Save ${TITLES.getValue(page)}"
        }
        return true
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean = when (item.itemId) {
        MENU_SAVE -> { onSave(); true }
        android.R.id.home -> { if (hasForm) confirmLeave() else finish(); true }
        else -> super.onOptionsItemSelected(item)
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
        screen {
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
                        DevSummary.lastShort(d.result) { id -> s.cloud.provider(id)?.displayName } + " · " +
                            DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(d.atMs))
                    } ?: "No dictation yet"
                    circuit.text = circuitText()
                    val w = DevPages.warningsFor(Page.HUB, DevSettingsForm.validate(DevSettingsForm.Input.of(s), caps).warnings)
                    warn.text = w.joinToString("\n"); warn.visibility = if (w.isEmpty()) View.GONE else View.VISIBLE
                }
            }

            sectionLabel("DICTATION")
            card {
                val routing = navRow(R.drawable.ic_dev_route, "Routing & engines") { open(Page.ROUTING) }
                refreshers += {
                    routing.setSubtitle(DevSummary.routing(app.dev.load(), caps) + if (caps.local) " · Parakeet ${app.dev.parakeetDecoding.wire}" else "")
                }
            }

            sectionLabel("CONNECTION")
            card {
                val server = navRow(R.drawable.ic_dev_server, "Server") { open(Page.SERVER) }
                val cloud = navRow(R.drawable.ic_dev_cloud, "Cloud providers") { open(Page.CLOUD) }
                refreshers += {
                    val s = app.dev.load()
                    server.setSubtitle(DevSummary.server(s, caps) + if (caps.remote && s.serverConfigured) "\n" + DevSummary.timeouts(s) else "")
                    cloud.setSubtitle(DevSummary.cloud(s, caps))
                }
            }

            sectionLabel("DIAGNOSTICS")
            card {
                val debug = navRow(R.drawable.ic_dev_debug, "Debug log & audio") { open(Page.DEBUG) }
                refreshers += { debug.setSubtitle(DevSummary.debug(app.dev.debugLog, app.dev.keepDebugAudio)) }
            }

            sectionLabel("BUILD")
            card {
                infoRow("Version", dev.flowclone.data.AppVersion.current.detail())
                infoRow("Flavour · native library", "${caps.flavor} · ${capabilitiesLine()}")
                navRow(R.drawable.ic_dev_reset, "Reset to defaults", "Developer settings and every cloud key", danger = true, chevron = false) { confirmResetDefaults() }
                if (!caps.remote) navRow(R.drawable.ic_dev_lock, "Lock", "Hide Developer settings again", chevron = false) {
                    app.dev.devUnlocked = false
                    Toast.makeText(this@DeveloperActivity, "Developer settings hidden", Toast.LENGTH_SHORT).show()
                    finish()
                }
            }
        }
    }

    private fun open(p: Page) = open(this, p)

    // =====================================================================================
    // Routing & engines
    // =====================================================================================

    private lateinit var allowServer: DevUi.ToggleRow
    private lateinit var allowClient: DevUi.ToggleRow
    private lateinit var keepWarm: DevUi.ToggleRow
    private lateinit var badge: DevUi.ToggleRow
    private var orderBox: LinearLayout? = null
    private var orderCount: TextView? = null
    private var sessionResult: DevUi.Result? = null

    private fun renderRouting() = with(ui) {
        val s = app.dev.load()
        screen {
            banner = banner()
            sectionLabel("ENGINES")
            card {
                allowServer = toggleRow("Allow server", if (caps.remote) "Your flow-server over LAN / Tailscale" else CloudText.NOT_IN_BUILD.cap(), s.allowServer && caps.remote, enabled = caps.remote)
                allowClient = toggleRow("Allow on-device", if (caps.local) "The speech model on this phone" else CloudText.NOT_IN_BUILD.cap(), s.allowClient && caps.local, enabled = caps.local)
            }
            sectionLabel("ROUTE ORDER")
            card {
                orderCount = cardHeader("Try in this order", "")
                note("The first available route is used; the next ones are fallbacks.")
                orderBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also(::addView)
            }
            sectionLabel("OPTIONS")
            card {
                keepWarm = toggleRow("Keep on-device model warm", "Stays loaded while the server is preferred: instant fallback. Off: loaded on demand when the server is unreachable", s.keepLocalWarm && caps.local, enabled = caps.local)
                badge = toggleRow("Engine badge", "Show S / L on the preview line (• = fallback)", s.showEngineBadge)
            }
            if (caps.local) parakeetCard()
            action("Test session") { onTestSession() }
            sessionResult = result()
            note("Sends 1 s of silence through the saved routes; expect EMPTY (no_speech).")
        }
        for (t in listOf(allowServer, allowClient, keepWarm, badge)) t.onChange { validate() }
    }

    /** D22 Parakeet decoding: applies at once (not part of Save), as before. */
    private fun LinearLayout.parakeetCard() = with(ui) {
        sectionLabel("ON-DEVICE DECODING (PARAKEET)")
        card {
            note("Applies immediately and reloads the engine.")
            val labels = mapOf(
                ParakeetDecoding.AUTO to "Auto\nBeam search for dictionary words (≈ +0.6 GB RAM)",
                ParakeetDecoding.GREEDY to "Greedy (default)\nOne recognizer, less RAM",
                ParakeetDecoding.BEAM to "Beam only\nThe old behaviour",
            )
            val group = RadioGroup(context).apply { orientation = RadioGroup.VERTICAL; setPadding(dp(8), 0, dp(16), dp(8)) }
            for (m in ParakeetDecoding.entries) {
                group.addView(RadioButton(context).apply {
                    id = View.generateViewId(); tag = m
                    text = labels.getValue(m); textSize = 15f; setTextColor(p.text)
                    buttonTintList = android.content.res.ColorStateList.valueOf(p.accent)
                    minHeight = dp(56)
                    isChecked = m == app.dev.parakeetDecoding
                })
            }
            group.setOnCheckedChangeListener { g, id ->
                val m = g.findViewById<View>(id)?.tag as? ParakeetDecoding ?: return@setOnCheckedChangeListener
                if (m == app.dev.parakeetDecoding) return@setOnCheckedChangeListener
                app.dev.parakeetDecoding = m
                if (app.settings.load().asrEngine == dev.flowclone.core.AsrEngine.PARAKEET) app.engine.reload()
                Toast.makeText(this@DeveloperActivity, "Parakeet decoding: ${m.wire}", Toast.LENGTH_SHORT).show()
            }
            addView(group)
        }
    }

    private fun routeShown(r: DevSettings.Route) = when (r) {
        DevSettings.Route.SERVER -> if (page == Page.ROUTING) allowServer.isChecked else draft.draft.allowServer && caps.remote
        DevSettings.Route.CLIENT -> if (page == Page.ROUTING) allowClient.isChecked else draft.draft.allowClient && caps.local
        DevSettings.Route.CLOUD -> draft.draft.allowCloud && caps.cloud
    }

    /** Route order rows with ↑ / ↓ for the routes this build and the toggles allow. */
    private fun renderOrder() = with(ui) {
        val box = orderBox ?: return
        box.removeAllViews()
        val shown = routeOrder.filter { routeShown(it) }
        orderCount?.text = "${shown.size} on"
        if (shown.isEmpty()) box.note("No route is on: turn on an engine above.", p.danger)
        shown.forEachIndexed { i, r ->
            val name = when (r) {
                DevSettings.Route.SERVER -> "Server (your flow-server)"
                DevSettings.Route.CLIENT -> "Phone (on-device)"
                DevSettings.Route.CLOUD -> "Cloud: " + (cloudState.activeProvider?.let { "${it.displayName} (${it.host})" } ?: "no provider")
            }
            box.addView(LinearLayout(this@DeveloperActivity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                setPadding(dp(16), dp(4), dp(8), dp(4)); minimumHeight = dp(56)
                addView(TextView(context).apply { text = "${i + 1}.  $name"; textSize = 16f; setTextColor(p.text) }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                addView(iconButton(R.drawable.ic_dev_up, "Move $name up") { move(r, -1) }.apply { isEnabled = i > 0; alpha = if (i > 0) 1f else 0.3f })
                addView(iconButton(R.drawable.ic_dev_down, "Move $name down") { move(r, +1) }.apply { isEnabled = i < shown.lastIndex; alpha = if (i < shown.lastIndex) 1f else 0.3f })
            })
        }
    }

    private fun move(r: DevSettings.Route, dir: Int) {
        routeOrder = DevPages.move(routeOrder, r, dir, ::routeShown)
        validate()
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
    private val fieldRows = HashMap<Field, DevUi.FieldRow>()

    private fun renderServer() = with(ui) {
        val s = app.dev.load()
        val edit = { if (::draft.isInitialized && fieldRows.isNotEmpty()) validate(); Unit }
        screen {
            banner = banner()
            if (!caps.remote) note("This build has no network code: the server route is not available.", p.danger, 14f)
            sectionLabel("ADDRESS")
            card {
                host = fieldRow("Host or IP", s.host, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI,
                    help = "No scheme, no port", hint = "100.64.0.10 or desktop.tail1234.ts.net", onEdit = edit)
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
    // Cloud providers (D18, docs/cloud-stt.md §8.3)
    // =====================================================================================

    private var allowCloud: DevUi.ToggleRow? = null
    private var cloudBanner: TextView? = null
    private var providerBox: LinearLayout? = null
    private var providerCount: TextView? = null
    private var budget: DevUi.FieldRow? = null
    private var usageLine: TextView? = null
    private var cloudProbe: DevUi.Result? = null

    private fun renderCloud() = with(ui) {
        val s = app.dev.load()
        screen {
            banner = banner()
            if (!caps.cloud) {
                card {
                    toggleRow("Allow cloud", CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                    note("This build has no cloud client: audio never leaves the device except to your own server.")
                }
                return@screen
            }
            card {
                cloudBanner = note(CloudText.banner(s.cloud.activeProvider)).apply { setPadding(dp(16), dp(14), dp(16), dp(4)) }
                allowCloud = toggleRow("Allow cloud", "Third-party speech-to-text; off by default", s.allowCloud)
            }
            sectionLabel("PROVIDERS")
            card {
                providerCount = cardHeader("Providers", "")
                providerBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also(::addView)
                navRow(R.drawable.ic_dev_add, "Add provider…", chevron = false) { pickPreset() }
            }
            sectionLabel("BUDGET & USAGE")
            card {
                budget = fieldRow("Monthly budget (USD)", s.cloud.monthlyBudgetUsd?.let { DevSettingsForm.Input.budgetText(it) }.orEmpty(),
                    InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL,
                    help = "Empty = none; the cloud pauses once it is reached", hint = "5", onEdit = { if (fieldRows.isNotEmpty()) validate() })
                usageLine = infoRow("Usage this month (this device only, never sent anywhere)")
                chipRow(chip("Reset counters") { onResetUsage() }.apply { contentDescription = "Reset cloud usage counters" })
            }
            action("Test active provider") { onTestCloud() }
            cloudProbe = result()
            note("Uploads a 3 s public-domain clip to the active provider.")
        }
        budget?.let { fieldRows[Field.CLOUD_BUDGET] = it }
        allowCloud?.onChange { on -> onAllowCloud(on) }
        refreshers += { renderProviders(); refreshUsage() }
    }

    private fun renderProviders() = with(ui) {
        val box = providerBox ?: return
        box.removeAllViews()
        cloudBanner?.text = CloudText.banner(cloudState.activeProvider)
        providerCount?.text = if (cloudState.providers.isEmpty()) "" else "● = used by the cloud route"
        if (cloudState.providers.isEmpty()) box.note("No provider yet.")
        for (pr in cloudState.providers) {
            val active = pr.id == cloudState.active
            val sub = "${pr.host.ifEmpty { "?" }} · ${pr.model} · key ${app.cloudKeys.masked(pr.id)} · consent ${if (pr.consentOk) "✓" else "✗"}"
            val row = box.navRow(null, "${if (active) "●" else "○"}  ${pr.displayName}", sub) { CloudProviderActivity.edit(this@DeveloperActivity, pr.id) }
            row.view.contentDescription = "${pr.displayName}${if (active) ", in use" else ""}, ${pr.host}, ${pr.model}, " +
                "key ${if (app.cloudKeys.has(pr.id)) "stored" else "none"}, consent ${if (pr.consentOk) "given" else "not given"}. Edit"
            if (!active) box.chipRow(chip("Use ${pr.displayName}") { useProvider(pr.id) })
        }
        renderOrder()
    }

    private fun refreshUsage() {
        usageLine?.text = CloudText.usageLines(
            CloudUsage.decode(app.engine.router()?.cloudUsageJson() ?: app.dev.cloudUsageJson), cloudState,
        ).joinToString("\n")
    }

    private fun pickPreset() {
        val presets = CloudPresets.ALL
        AlertDialog.Builder(this)
            .setTitle("Add a cloud provider")
            .setItems(presets.map { "${it.label} — ${it.model.ifEmpty { "your model" }}" }.toTypedArray()) { _, i -> CloudProviderActivity.add(this, presets[i].id) }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun useProvider(id: String) {
        cloudState = cloudState.copy(active = id)
        app.dev.saveCloud(cloud = cloudState.copy(monthlyBudgetUsd = app.dev.load().cloud.monthlyBudgetUsd))
        app.engine.applyDevSettings(app.dev.load())
        draft = draft.rebase(app.dev.load())
        renderProviders(); validate()
    }

    /**
     * First enable: the consent dialog for the active provider (§8.3); accepting records the
     * consent (host + time) and puts the cloud first in the route order (§3.1 step 4).
     */
    private fun onAllowCloud(on: Boolean) {
        val b = allowCloud ?: return
        if (!on) { validate(); return }
        val pr = cloudState.activeProvider
        if (pr == null) {
            b.isChecked = false
            Toast.makeText(this, "Add a provider first", Toast.LENGTH_LONG).show()
            pickPreset()
            return
        }
        val enable = {
            if (DevSettings.Route.CLOUD !in routeOrder) routeOrder = DevSettings.withCloudFirst(routeOrder)
            validate()
        }
        if (pr.consentOk) { enable(); return }
        CloudProviderActivity.consentDialog(this, pr, onCancel = { b.isChecked = false; validate() }) {
            cloudState = cloudState.replace(pr.withConsent(System.currentTimeMillis()))
            app.dev.saveCloud(cloud = cloudState.copy(monthlyBudgetUsd = app.dev.load().cloud.monthlyBudgetUsd))
            draft = draft.rebase(app.dev.load())
            renderProviders()
            enable()
        }
    }

    private fun onTestCloud() {
        val line = cloudProbe ?: return
        val pr = cloudState.activeProvider ?: run { line.show("No active provider.", Tone.BAD); return }
        val key = app.cloudKeys.get(pr.id)
            ?: if (CloudKeyPolicy.keylessOk(pr)) null else run { line.show("${pr.displayName}: no API key stored.", Tone.BAD); return }
        line.show("Testing ${pr.displayName} (${pr.host})…")
        val json = pr.toJson().toString()
        thread(name = "dev-cloud-probe") {
            val h = app.engine.router()?.probeCloud(json, key) ?: NativeFlowCore.probeCloud(json, key)
            if (!h.ok) app.debugLog.event("cloud_probe", DevText.cloudProbe(h))
            runOnUiThread { line.show(DevText.cloudProbe(h), if (h.ok) Tone.OK else Tone.BAD); refresh() }
        }
    }

    private fun onResetUsage() {
        val r = app.engine.router()
        if (r != null) { r.resetCloudUsage(); app.engine.persistCloudUsage() } else app.dev.cloudUsageJson = null
        refresh()
    }

    // =====================================================================================
    // Debug log + audio (D21; device-local, includes transcripts). Apply at once, as before.
    // =====================================================================================

    private var debugCount: TextView? = null
    private var audioCount: TextView? = null

    private fun renderDebug() = with(ui) {
        var debugToggle: DevUi.ToggleRow? = null
        screen {
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
                        "Never for password fields, banking or excluded apps. Deleted after 24 hours. Nothing is sent anywhere.",
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
        screen {
            sectionLabel("LAST DICTATION")
            lateinit var last: TextView
            card { last = infoRow("Active engine") }
            sectionLabel("CIRCUITS")
            lateinit var circuit: TextView
            var cloudCircuit: TextView? = null
            card {
                circuit = infoRow("Server / on-device circuit")
                if (caps.cloud) cloudCircuit = infoRow("Cloud circuit")
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
                last.text = DevText.lastDictation(l?.result) { id -> cloudState.provider(id)?.displayName } +
                    (l?.let { " · " + DateFormat.getTimeInstance().format(Date(it.atMs)) } ?: "")
                circuit.text = circuitText()
                val rs = routeStatus()
                val cc = rs?.cloudCircuit
                cloudCircuit?.text = if (cc == null) "Not loaded" else DevText.circuit(cc, System.currentTimeMillis(), "the key or provider changes") { DateFormat.getTimeInstance().format(Date(it)) }
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
        Page.ROUTING -> i.copy(
            allowServer = allowServer.isChecked, allowClient = allowClient.isChecked, routeOrder = routeOrder,
            keepLocalWarm = keepWarm.isChecked, showEngineBadge = badge.isChecked,
        )
        Page.SERVER -> i.copy(
            host = host.text, port = port.text, tls = tls.isChecked, pin = pin.text, token = token.text,
            connectTimeoutMs = connectTimeout.text, commitTimeoutBaseMs = commitTimeout.text, healthTtlS = healthTtl.text,
        )
        Page.CLOUD -> i.copy(allowCloud = allowCloud?.isChecked == true, cloudBudget = budget?.text.orEmpty(), routeOrder = routeOrder)
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
        renderOrder()
        val off = DevPages.offPageErrors(r, page)
        val lines = DevPages.warningsFor(page, r.warnings) + off.map { (f, e) -> "$e (on the ${TITLES.getValue(DevPages.pageOf(f))} page)" }
        banner?.show(lines.joinToString("\n"), if (off.isNotEmpty() || DevSettingsForm.BOTH_OFF in lines) Tone.BAD else Tone.WARN)
        val can = draft.canSave(caps)
        saveItem?.let { if (it.isEnabled != can) it.isEnabled = can }
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
            .setMessage("Restores the default engines, server and timeouts, and deletes every cloud provider and its API key from this device.")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Reset") { _, _ -> onResetDefaults() }
            .show()
    }

    /** Also forgets every cloud key (sealed copies and their Keystore keys), like deleting each provider. */
    private fun onResetDefaults() {
        for (pr in app.dev.load().cloud.providers) { app.cloudKeys.remove(pr.id); app.engine.setCloudKey(pr.id, null) }
        app.dev.reset()
        app.engine.applyDevSettings(app.dev.load())
        lastProbe = null
        recreate()
    }

    companion object {
        const val EXTRA_PAGE = "page"
        private const val MENU_SAVE = 1

        val TITLES = mapOf(
            Page.HUB to "Developer settings", Page.ROUTING to "Routing & engines", Page.SERVER to "Server",
            Page.CLOUD to "Cloud providers", Page.DEBUG to "Debug log & audio", Page.STATUS to "Status",
        )

        /** Last server probe of this process (Status page "Server info"); else the router's cached health. */
        @Volatile private var lastProbe: HealthReport? = null

        fun open(ctx: Context, p: Page) = ctx.startActivity(Intent(ctx, DeveloperActivity::class.java).putExtra(EXTRA_PAGE, p.name))

        private fun String.cap() = replaceFirstChar { it.uppercase() }
    }
}
