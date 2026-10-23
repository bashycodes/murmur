package dev.flowclone.ui

import android.app.Activity
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.TextWatcher
import android.view.View
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.DevText
import dev.flowclone.core.HealthReport
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.core.SessionConfig
import dev.flowclone.data.CloudKeyPolicy
import dev.flowclone.data.CloudPresets
import dev.flowclone.data.CloudSettings
import dev.flowclone.data.CloudText
import dev.flowclone.data.CloudUsage
import dev.flowclone.data.DevSettings
import dev.flowclone.data.DevSettingsForm
import dev.flowclone.data.DevSettingsForm.Field
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * Developer settings (docs/hybrid-architecture.md §5): engine routes (server / client),
 * server address + token, timeouts, Test connection / Test session, last-engine and circuit
 * rows. Saved settings apply from the next dictation; models are never reloaded unless the
 * client route's need for them changed. Sections are independent blocks so further routes
 * (network flavours only) can be added as their own section.
 */
class DeveloperActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val caps get() = app.caps

    private lateinit var allowServer: CheckBox
    private lateinit var allowClient: CheckBox
    private lateinit var orderBox: LinearLayout
    private lateinit var host: EditText
    private lateinit var port: EditText
    private lateinit var tls: CheckBox
    private lateinit var pin: EditText
    private lateinit var token: EditText
    private lateinit var connectTimeout: EditText
    private lateinit var commitTimeout: EditText
    private lateinit var healthTtl: EditText
    private lateinit var keepWarm: CheckBox
    private lateinit var badge: CheckBox
    private lateinit var save: Button
    private lateinit var status: TextView
    private lateinit var probeLine: TextView
    private lateinit var serverInfo: TextView
    private lateinit var lastLine: TextView
    private lateinit var circuitLine: TextView
    private lateinit var debugToggle: CheckBox
    private lateinit var debugCount: TextView
    private val errorViews = HashMap<Field, TextView>()
    private var lastProbe: HealthReport? = null
    private var routeOrder = DevSettings.DEFAULT_ORDER
    // D18 cloud section (only built when the flavour has the cloud client)
    private var allowCloud: CheckBox? = null
    private var cloudBanner: TextView? = null
    private var providerBox: LinearLayout? = null
    private var budget: EditText? = null
    private var usageLine: TextView? = null
    private var cloudProbeLine: TextView? = null
    private lateinit var cloudCircuitLine: TextView
    /** Providers + active id as stored (the provider page and consent dialog save them directly). */
    private var cloudState = CloudSettings()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Developer settings"
        render(app.dev.load())
    }

    override fun onResume() {
        super.onResume()
        if (::lastLine.isInitialized) {
            cloudState = app.dev.load().cloud
            renderProviders()
            validate()
            refreshRows()
        }
    }

    private fun render(s: DevSettings) {
        routeOrder = s.routeOrder
        cloudState = s.cloud
        page {
            body("Build: ${caps.flavor} · ${capabilitiesLine()}")
            status = body("")

            heading("Engines")
            allowServer = check("Allow server (your flow-server over LAN / Tailscale)", s.allowServer, enabled = caps.remote, disabledNote = "not in this build")
            allowClient = check("Allow client (on-device speech model)", s.allowClient, enabled = caps.local, disabledNote = "not in this build")
            body("Route order (the first available route is used; the next ones are fallbacks)")
            orderBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also(::addView)
            keepWarm = check("Keep the on-device model loaded while the server is preferred (instant fallback)", s.keepLocalWarm, enabled = caps.local)
            badge = check("Show the engine on the preview line (S / L, • = fallback)", s.showEngineBadge)

            heading("Server")
            host = field("Host or IP (no scheme, no port), e.g. 100.64.0.10 or desktop.tail1234.ts.net", s.host, Field.HOST, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
            port = field("Port", s.port.toString(), Field.PORT, InputType.TYPE_CLASS_NUMBER)
            tls = check("Use TLS (wss)", s.tls)
            pin = field("Certificate pin (SHA-256 of the leaf cert, 64 hex; optional)", s.tlsPinSha256, Field.PIN, InputType.TYPE_CLASS_TEXT)
            token = field("Token (from `flow-server gen-token`, ≥ 16 chars)", s.token, Field.TOKEN, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
            row(
                Button(context).apply { text = "Show"; isAllCaps = false; setOnClickListener { toggleTokenVisible(this) } },
                Button(context).apply { text = "Paste"; isAllCaps = false; setOnClickListener { pasteToken() } },
                Button(context).apply { text = "Clear"; isAllCaps = false; setOnClickListener { token.setText("") } },
            )
            connectTimeout = field("Connect timeout (ms, 100-10000)", s.connectTimeoutMs.toString(), Field.CONNECT_TIMEOUT, InputType.TYPE_CLASS_NUMBER)
            commitTimeout = field("Commit timeout base (ms, 500-60000)", s.commitTimeoutBaseMs.toString(), Field.COMMIT_TIMEOUT, InputType.TYPE_CLASS_NUMBER)
            healthTtl = field("Health cache (s, 5-600)", s.healthTtlS.toString(), Field.HEALTH_TTL, InputType.TYPE_CLASS_NUMBER)

            cloudSection(s)

            save = button("Save") { onSave() }
            button("Test connection") { onTestConnection() }
            probeLine = body("")
            button("Test session (1 s of silence through the saved routes)") { onTestSession() }

            heading("Status")
            body("Active engine (last dictation)")
            lastLine = body("")
            body("Circuit")
            circuitLine = body("")
            cloudCircuitLine = body("")
            body("Server info (last probe)")
            serverInfo = body("")
            row(
                Button(context).apply { text = "Reset circuit"; isAllCaps = false; setOnClickListener { onResetCircuit() } },
                Button(context).apply { text = "Reset to defaults"; isAllCaps = false; setOnClickListener { onResetDefaults() } },
            )
            if (caps.local) parakeetSection()
            debugSection()

            if (!caps.remote) button("Lock (hide Developer settings)") {
                app.dev.devUnlocked = false
                Toast.makeText(this@DeveloperActivity, "Developer settings hidden", Toast.LENGTH_SHORT).show()
                finish()
            }
        }
        for (v in listOf(allowServer, allowClient, tls, keepWarm, badge)) v.setOnCheckedChangeListener { _, _ -> validate() }
        allowCloud?.setOnCheckedChangeListener { b, on -> onAllowCloud(b as CheckBox, on) }
        renderProviders()
        validate()
        refreshRows()
    }

    // ---- D22 Parakeet decoding (on-device engine) ----

    private fun LinearLayout.parakeetSection() {
        heading("Parakeet decoding (on-device)")
        body("Greedy (default) loads one recognizer. Auto adds a beam-search recognizer for dictionary words (≈ +0.6 GB RAM). Beam is the old behaviour. Reloads the engine.")
        val modes = dev.flowclone.core.ParakeetDecoding.entries
        val labels = listOf("Auto (beam with dictionary words, +0.6 GB)", "Greedy (default, less RAM)", "Beam only")
        val sp = android.widget.Spinner(context).apply {
            adapter = android.widget.ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, labels)
            setSelection(modes.indexOf(app.dev.parakeetDecoding))
        }.also(::addView)
        sp.onItemSelectedListener = object : android.widget.AdapterView.OnItemSelectedListener {
            override fun onItemSelected(p: android.widget.AdapterView<*>?, v: View?, pos: Int, id: Long) {
                val m = modes[pos]
                if (m == app.dev.parakeetDecoding) return
                app.dev.parakeetDecoding = m
                if (app.settings.load().asrEngine == dev.flowclone.core.AsrEngine.PARAKEET) app.engine.reload()
                Toast.makeText(this@DeveloperActivity, "Parakeet decoding: ${m.wire}", Toast.LENGTH_SHORT).show()
            }
            override fun onNothingSelected(p: android.widget.AdapterView<*>?) = Unit
        }
    }

    // ---- D21 debug log (device-local; includes transcripts) ----

    private fun LinearLayout.debugSection() {
        heading("Debug log")
        debugToggle = check("Debug log (includes transcripts)", app.dev.debugLog)
        body("Writes what you dictate, the app it went to and the timings to this phone's log (logcat tag FlowDebug) and to a file in the app's storage. Nothing is sent anywhere unless you tap Share.")
        debugCount = body("")
        row(
            Button(context).apply { text = "View"; isAllCaps = false; setOnClickListener { onViewDebugLog() } },
            Button(context).apply { text = "Share / Export"; isAllCaps = false; setOnClickListener { onShareDebugLog() } },
            Button(context).apply { text = "Clear"; isAllCaps = false; setOnClickListener { clearDebugLog() } },
        )
        debugToggle.setOnCheckedChangeListener { _, on -> onDebugToggle(on) }
        audioSection()
    }

    // ---- "Keep last dictation audio (debug)" ----

    private lateinit var audioCount: TextView

    private fun LinearLayout.audioSection() {
        val keep = check("Keep last dictation audio (debug)", app.dev.keepDebugAudio)
        body(
            "Saves what the microphone heard for the last ${dev.flowclone.debug.DebugAudioStore.MAX_FILES} dictations as WAV files " +
                "in the app's private storage (files/debug/audio/), named like the debug log's ts and linked from its \"audio\" field. " +
                "Never for password fields, banking or excluded apps. Deleted after 24 hours. Nothing is sent anywhere.",
        )
        audioCount = body("")
        row(Button(context).apply { text = "Clear audio"; isAllCaps = false; setOnClickListener { clearDebugAudio() } })
        keep.setOnCheckedChangeListener { _, on -> app.dev.keepDebugAudio = on; refreshAudioCount() }
        refreshAudioCount()
    }

    private fun refreshAudioCount() {
        if (!::audioCount.isInitialized) return
        thread(name = "dev-audio-count") {
            val n = runCatching { app.debugAudio.prune(); app.debugAudio.list().size }.getOrDefault(0)
            val line = "$n file${if (n == 1) "" else "s"} kept" + if (app.dev.keepDebugAudio) "" else " · off"
            runOnUiThread { audioCount.text = line }
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
                android.app.AlertDialog.Builder(this)
                    .setTitle("Clear the debug log?")
                    .setMessage("Logging is off. The file still holds $n entries with your transcripts.")
                    .setPositiveButton("Clear") { _, _ -> clearDebugLog() }
                    .setNegativeButton("Keep") { _, _ -> refreshDebugCount() }
                    .show()
            }
        }
    }

    private fun refreshDebugCount() {
        if (!::debugCount.isInitialized) return
        thread(name = "dev-debug-count") {
            val n = runCatching { app.debugFile.size() }.getOrDefault(0)
            val errs = app.debugLog.errors.get(); val dropped = app.debugLog.dropped.get()
            val extra = listOfNotNull(errs.takeIf { it > 0 }?.let { "$it write errors" }, dropped.takeIf { it > 0 }?.let { "$it dropped" })
            val line = "$n entries (newest ${dev.flowclone.debug.DebugLogFile.MAX_ENTRIES} kept)" + extra.joinToString("") { " · $it" } +
                if (app.dev.debugLog) "" else " · logging off"
            runOnUiThread { debugCount.text = line }
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
                android.app.AlertDialog.Builder(this)
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

    private fun capabilitiesLine(): String {
        val c = NativeFlowCore.capabilities() ?: return "native library not loaded"
        val cc = NativeFlowCore.cloudCapabilities()
        val mismatch = if (c.local != caps.local || c.remote != caps.remote || cc.cloud != caps.cloud) " · ⚠ native library flavour mismatch" else ""
        return "native: local=${c.local} remote=${c.remote} cloud=${cc.cloud} asr=${c.asrEngines.joinToString("/").ifEmpty { "-" }} llm=${c.llm} proto=${c.proto} core ${c.coreVersion}$mismatch"
    }

    // ---- D18 cloud STT section (docs/cloud-stt.md §8.3) ----

    private fun LinearLayout.cloudSection(s: DevSettings) {
        heading(CloudText.SECTION)
        if (!caps.cloud) {
            check("Allow cloud", false, enabled = false, disabledNote = CloudText.NOT_IN_BUILD)
            body("This build has no cloud client: audio never leaves the device except to your own server.")
            return
        }
        cloudBanner = body(CloudText.banner(s.cloud.activeProvider))
        allowCloud = check("Allow cloud (third-party speech-to-text; off by default)", s.allowCloud)
        body("Providers (● = used by the cloud route)")
        providerBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also(::addView)
        button("Add provider…") { pickPreset() }
        budget = field("Monthly budget in USD (empty = none; the cloud pauses once it is reached)", s.cloud.monthlyBudgetUsd?.let { DevSettingsForm.Input.budgetText(it) }.orEmpty(), Field.CLOUD_BUDGET, InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL)
        body("Usage this month (this device only, never sent anywhere)")
        usageLine = body("")
        row(
            Button(context).apply { text = "Test active provider"; isAllCaps = false; setOnClickListener { onTestCloud() } },
            Button(context).apply { text = "Reset counters"; isAllCaps = false; setOnClickListener { onResetUsage() } },
        )
        cloudProbeLine = body("")
    }

    /** Route order rows with ↑ / ↓ for the routes this build and the toggles allow. */
    private fun renderOrder() {
        if (!::orderBox.isInitialized) return
        orderBox.removeAllViews()
        val shown = routeOrder.filter { routeShown(it) }
        shown.forEachIndexed { i, r ->
            val name = when (r) {
                DevSettings.Route.SERVER -> "Server (your flow-server)"
                DevSettings.Route.CLIENT -> "On-device"
                DevSettings.Route.CLOUD -> "Cloud: " + (cloudState.activeProvider?.let { "${it.displayName} (${it.host})" } ?: "no provider")
            }
            val label = TextView(this).apply { text = "${i + 1}. $name"; textSize = 14f }
            val up = Button(this).apply { text = "↑"; isEnabled = i > 0; setOnClickListener { move(r, -1) } }
            val down = Button(this).apply { text = "↓"; isEnabled = i < shown.lastIndex; setOnClickListener { move(r, +1) } }
            orderBox.addView(LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                addView(label, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 3f))
                addView(up, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                addView(down, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
            })
        }
        orderBox.visibility = if (shown.size >= 2) View.VISIBLE else View.GONE
    }

    private fun routeShown(r: DevSettings.Route) = when (r) {
        DevSettings.Route.SERVER -> allowServer.isChecked
        DevSettings.Route.CLIENT -> allowClient.isChecked
        DevSettings.Route.CLOUD -> allowCloud?.isChecked == true
    }

    /** Swap [r] with its visible neighbour; hidden routes keep their place in the stored order. */
    private fun move(r: DevSettings.Route, dir: Int) {
        val shown = routeOrder.filter { routeShown(it) }
        val i = shown.indexOf(r); val j = i + dir
        if (i < 0 || j !in shown.indices) return
        val other = shown[j]
        routeOrder = routeOrder.map { when (it) { r -> other; other -> r; else -> it } }
        validate()
    }

    private fun renderProviders() {
        val box = providerBox ?: return
        box.removeAllViews()
        cloudBanner?.text = CloudText.banner(cloudState.activeProvider)
        if (cloudState.providers.isEmpty()) {
            box.addView(TextView(this).apply { text = "No provider yet."; textSize = 14f })
        }
        for (p in cloudState.providers) {
            val active = p.id == cloudState.active
            val line = "${if (active) "●" else "○"} ${p.displayName} (${p.host.ifEmpty { "?" }}) · ${p.model} · key ${app.cloudKeys.masked(p.id)} · consent ${if (p.consentOk) "✓" else "✗"}"
            box.addView(TextView(this).apply { text = line; textSize = 14f })
            box.addView(LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                if (!active) addView(Button(context).apply { text = "Use"; isAllCaps = false; setOnClickListener { useProvider(p.id) } }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                addView(Button(context).apply { text = "Edit / key / test"; isAllCaps = false; setOnClickListener { CloudProviderActivity.edit(this@DeveloperActivity, p.id) } }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 2f))
            })
        }
        renderOrder()
    }

    private fun pickPreset() {
        val presets = CloudPresets.ALL
        android.app.AlertDialog.Builder(this)
            .setTitle("Add a cloud provider")
            .setItems(presets.map { "${it.label} — ${it.model.ifEmpty { "your model" }}" }.toTypedArray()) { _, i -> CloudProviderActivity.add(this, presets[i].id) }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun useProvider(id: String) {
        cloudState = cloudState.copy(active = id)
        app.dev.saveCloud(cloud = cloudState.copy(monthlyBudgetUsd = app.dev.load().cloud.monthlyBudgetUsd))
        app.engine.applyDevSettings(app.dev.load())
        renderProviders(); validate()
    }

    /**
     * First enable: the consent dialog for the active provider (§8.3); accepting records the
     * consent (host + time) and puts the cloud first in the route order (§3.1 step 4).
     */
    private fun onAllowCloud(b: CheckBox, on: Boolean) {
        if (!on) { validate(); return }
        val p = cloudState.activeProvider
        if (p == null) {
            b.isChecked = false
            Toast.makeText(this, "Add a provider first", Toast.LENGTH_LONG).show()
            pickPreset()
            return
        }
        val enable = {
            if (DevSettings.Route.CLOUD !in routeOrder) routeOrder = DevSettings.withCloudFirst(routeOrder)
            validate()
        }
        if (p.consentOk) { enable(); return }
        CloudProviderActivity.consentDialog(this, p, onCancel = { b.isChecked = false; validate() }) {
            cloudState = cloudState.replace(p.withConsent(System.currentTimeMillis()))
            app.dev.saveCloud(cloud = cloudState.copy(monthlyBudgetUsd = app.dev.load().cloud.monthlyBudgetUsd))
            renderProviders()
            enable()
        }
    }

    private fun onTestCloud() {
        val line = cloudProbeLine ?: return
        val p = cloudState.activeProvider ?: run { line.text = "No active provider."; return }
        val key = app.cloudKeys.get(p.id)
            ?: if (CloudKeyPolicy.keylessOk(p)) null else run { line.text = "${p.displayName}: no API key stored."; return }
        line.text = "Testing ${p.displayName} (${p.host})…"
        val json = p.toJson().toString()
        thread(name = "dev-cloud-probe") {
            val h = app.engine.router()?.probeCloud(json, key) ?: NativeFlowCore.probeCloud(json, key)
            if (!h.ok) app.debugLog.event("cloud_probe", DevText.cloudProbe(h))
            runOnUiThread {
                line.text = DevText.cloudProbe(h)
                line.setTextColor(if (h.ok) OK_COLOR else BAD_COLOR)
                refreshRows()
            }
        }
    }

    private fun onResetUsage() {
        val r = app.engine.router()
        if (r != null) { r.resetCloudUsage(); app.engine.persistCloudUsage() } else app.dev.cloudUsageJson = null
        refreshRows()
    }

    private fun LinearLayout.check(text: String, checked: Boolean, enabled: Boolean = true, disabledNote: String? = null) =
        CheckBox(context).apply {
            this.text = if (!enabled && disabledNote != null) "$text — $disabledNote" else text
            isChecked = checked && enabled; isEnabled = enabled
        }.also(::addView)

    private fun LinearLayout.field(label: String, value: String, f: Field, type: Int): EditText {
        body(label)
        val e = EditText(context).apply {
            inputType = type; setText(value); isSingleLine = true
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun afterTextChanged(s: Editable?) { if (::save.isInitialized) validate() }
            })
        }.also(::addView)
        errorViews[f] = body("", BAD_COLOR).apply { visibility = View.GONE }
        return e
    }

    private fun input() = DevSettingsForm.Input(
        allowServer = allowServer.isChecked, allowClient = allowClient.isChecked, routeOrder = routeOrder,
        host = host.text.toString(), port = port.text.toString(), tls = tls.isChecked, pin = pin.text.toString(),
        token = token.text.toString(), connectTimeoutMs = connectTimeout.text.toString(),
        commitTimeoutBaseMs = commitTimeout.text.toString(), healthTtlS = healthTtl.text.toString(),
        keepLocalWarm = keepWarm.isChecked, showEngineBadge = badge.isChecked,
        allowCloud = allowCloud?.isChecked == true, cloud = cloudState, cloudBudget = budget?.text?.toString().orEmpty(),
    )

    private fun validate(): DevSettingsForm.Result {
        val r = DevSettingsForm.validate(input(), caps)
        for ((f, v) in errorViews) {
            val e = r.errors[f]
            v.text = e ?: ""; v.visibility = if (e == null) View.GONE else View.VISIBLE
        }
        if (r.tlsLocked && !tls.isChecked) tls.isChecked = true
        tls.isEnabled = !r.tlsLocked
        tls.text = if (r.tlsLocked) "Use TLS (wss) — required for a public address" else "Use TLS (wss)"
        pin.isEnabled = tls.isChecked
        renderOrder()
        save.isEnabled = r.canSave
        status.text = r.warnings.joinToString("\n")
        status.setTextColor(if (DevSettingsForm.BOTH_OFF in r.warnings) BAD_COLOR else status.currentTextColor)
        return r
    }

    private fun onSave() {
        val r = validate()
        val s = r.settings ?: return
        app.dev.save(s)
        app.engine.applyDevSettings(s)
        Toast.makeText(this, if (s.effective(caps).bothOff) DevSettingsForm.BOTH_OFF else "Saved: applies to the next dictation", Toast.LENGTH_LONG).show()
        // §5: a probe runs after each save; its result fills "Server info".
        if (s.effective(caps).allowServer && s.serverConfigured) {
            val r = app.engine.router()
            if (r == null) probe(s) else {
                probeLine.text = "Probing ${s.host}:${s.port}…"
                thread(name = "dev-probe-saved") {
                    val h = runCatching { r.probeSaved() }.getOrElse { HealthReport(ok = false, error = "error", message = it.message) }
                    if (!h.ok) app.debugLog.event("server_probe", DevText.probe(h)) else rememberServerModel(h)
                    runOnUiThread { lastProbe = h; probeLine.text = DevText.probe(h); refreshRows() }
                }
            }
        }
    }

    /** The debug log's `model` for server dictations: what the server says it runs. */
    private fun rememberServerModel(h: HealthReport) {
        (h.asrModel ?: h.asrBackend)?.let { app.dev.serverAsrModel = it }
    }

    private fun onTestConnection() {
        val s = validate().settings ?: run { probeLine.text = "Fix the highlighted fields first."; return }
        if (!caps.remote) { probeLine.text = "This build has no network code."; return }
        probe(s)
    }

    /** `probe_server()` with the form's (unsaved) values, off the UI thread. */
    private fun probe(s: DevSettings) {
        probeLine.text = "Testing ${s.host}:${s.port}…"
        val json = s.effective(caps).toServerConfigJson().toString()
        thread(name = "dev-probe") {
            val h = NativeFlowCore.probeServer(json)
            if (!h.ok) app.debugLog.event("server_probe", DevText.probe(h)) else rememberServerModel(h)
            runOnUiThread {
                lastProbe = h
                probeLine.text = DevText.probe(h)
                probeLine.setTextColor(if (h.ok) OK_COLOR else BAD_COLOR)
                refreshRows()
            }
        }
    }

    /** 1 s of silence through the full router path with the saved toggles: expect EMPTY no_speech. */
    private fun onTestSession() {
        probeLine.text = "Test session…"
        thread(name = "dev-test-session") {
            val line = try {
                val core = app.engine.current()
                core.startSession(SessionConfig(maxSamples = 16_000L * 10))
                core.pushAudio(ShortArray(16_000))
                val r = core.commit()
                val ok = r.status == CommitStatus.EMPTY
                (if (ok) "OK: " else "Unexpected: ") + DevText.lastDictation(r)
            } catch (e: CoreException) {
                app.debugLog.event("test_session", e.message, e)
                "Failed: ${dev.flowclone.core.RouterErrors.userMessage(e.kind, e.message ?: "error")}"
            }
            runOnUiThread { probeLine.text = line; refreshRows() }
        }
    }

    /** Also forgets every cloud key (sealed copies and their Keystore keys), like deleting each provider. */
    private fun onResetDefaults() {
        for (p in app.dev.load().cloud.providers) { app.cloudKeys.remove(p.id); app.engine.setCloudKey(p.id, null) }
        app.dev.reset()
        app.engine.applyDevSettings(app.dev.load())
        recreate()
    }

    private fun onResetCircuit() {
        val r = app.engine.router()
        if (r == null) { Toast.makeText(this, "No router loaded", Toast.LENGTH_SHORT).show(); return }
        runCatching { r.resetCircuit() }
        refreshRows()
    }

    private fun refreshRows() {
        val last = app.lastRoute.value
        lastLine.text = DevText.lastDictation(last?.result) { id -> cloudState.provider(id)?.displayName } + (last?.let { " · " + DateFormat.getTimeInstance().format(Date(it.atMs)) } ?: "")
        val rs = app.engine.router()?.let { runCatching { it.routeStatus() }.getOrNull() }
        circuitLine.text = if (rs == null) "router not loaded (${app.engine.state.value::class.simpleName})"
        else DevText.circuit(rs.circuit, System.currentTimeMillis()) { DateFormat.getTimeInstance().format(Date(it)) }
        val cc = rs?.cloudCircuit
        cloudCircuitLine.text = if (cc == null) "" else "Cloud circuit: " + DevText.circuit(cc, System.currentTimeMillis(), "the key or provider changes") { DateFormat.getTimeInstance().format(Date(it)) }
        cloudCircuitLine.visibility = if (cc == null) View.GONE else View.VISIBLE
        usageLine?.text = CloudText.usageLines(
            CloudUsage.decode(app.engine.router()?.cloudUsageJson() ?: app.dev.cloudUsageJson), cloudState,
        ).joinToString("\n")
        serverInfo.text = DevText.serverInfo(lastProbe ?: rs?.health)
        refreshDebugCount()
    }

    private fun toggleTokenVisible(b: Button) {
        val hidden = token.inputType and InputType.TYPE_TEXT_VARIATION_PASSWORD == InputType.TYPE_TEXT_VARIATION_PASSWORD
        token.inputType = InputType.TYPE_CLASS_TEXT or if (hidden) InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD else InputType.TYPE_TEXT_VARIATION_PASSWORD
        b.text = if (hidden) "Hide" else "Show"
    }

    private fun pasteToken() {
        val cm = getSystemService(android.content.ClipboardManager::class.java)
        val t = cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()?.trim()
        if (t.isNullOrEmpty()) Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show() else token.setText(t)
    }
}
