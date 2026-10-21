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
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.core.CommitStatus
import dev.flowclone.core.CoreException
import dev.flowclone.core.DevText
import dev.flowclone.core.HealthReport
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.core.SessionConfig
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
    private lateinit var serverFirst: RadioButton
    private lateinit var preferGroup: RadioGroup
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
    private val errorViews = HashMap<Field, TextView>()
    private var lastProbe: HealthReport? = null
    private var routeOrder = DevSettings.DEFAULT_ORDER

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Developer settings"
        render(app.dev.load())
    }

    override fun onResume() {
        super.onResume()
        if (::lastLine.isInitialized) refreshRows()
    }

    private fun render(s: DevSettings) {
        routeOrder = s.routeOrder
        page {
            body("Build: ${caps.flavor} · ${capabilitiesLine()}")
            status = body("")

            heading("Engines")
            allowServer = check("Allow server (your flow-server over LAN / Tailscale)", s.allowServer, enabled = caps.remote, disabledNote = "not in this build")
            allowClient = check("Allow client (on-device speech model)", s.allowClient, enabled = caps.local, disabledNote = "not in this build")
            preferGroup = RadioGroup(context).apply { orientation = RadioGroup.HORIZONTAL }
            serverFirst = RadioButton(context).apply { text = "Server first"; id = View.generateViewId() }
            val clientFirst = RadioButton(context).apply { text = "Client first"; id = View.generateViewId() }
            preferGroup.addView(serverFirst); preferGroup.addView(clientFirst)
            preferGroup.check(if (s.prefer == DevSettings.Prefer.SERVER) serverFirst.id else clientFirst.id)
            addView(preferGroup)
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

            save = button("Save") { onSave() }
            button("Test connection") { onTestConnection() }
            probeLine = body("")
            button("Test session (1 s of silence through the saved routes)") { onTestSession() }

            heading("Status")
            body("Active engine (last dictation)")
            lastLine = body("")
            body("Circuit")
            circuitLine = body("")
            body("Server info (last probe)")
            serverInfo = body("")
            row(
                Button(context).apply { text = "Reset circuit"; isAllCaps = false; setOnClickListener { onResetCircuit() } },
                Button(context).apply { text = "Reset to defaults"; isAllCaps = false; setOnClickListener { app.dev.reset(); app.engine.applyDevSettings(app.dev.load()); recreate() } },
            )
            if (!caps.remote) button("Lock (hide Developer settings)") {
                app.dev.devUnlocked = false
                Toast.makeText(this@DeveloperActivity, "Developer settings hidden", Toast.LENGTH_SHORT).show()
                finish()
            }
        }
        for (v in listOf(allowServer, allowClient, tls, keepWarm, badge)) v.setOnCheckedChangeListener { _, _ -> validate() }
        preferGroup.setOnCheckedChangeListener { _, id ->
            routeOrder = DevSettings.orderFor(if (id == serverFirst.id) DevSettings.Prefer.SERVER else DevSettings.Prefer.CLIENT)
            validate()
        }
        validate()
        refreshRows()
    }

    private fun capabilitiesLine(): String {
        val c = NativeFlowCore.capabilities() ?: return "native library not loaded"
        val mismatch = if (c.local != caps.local || c.remote != caps.remote) " · ⚠ native library flavour mismatch" else ""
        return "native: local=${c.local} remote=${c.remote} asr=${c.asrEngines.joinToString("/").ifEmpty { "-" }} llm=${c.llm} proto=${c.proto} core ${c.coreVersion}$mismatch"
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
        preferGroup.visibility = if (allowServer.isChecked && allowClient.isChecked) View.VISIBLE else View.GONE
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
                    runOnUiThread { lastProbe = h; probeLine.text = DevText.probe(h); refreshRows() }
                }
            }
        }
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
                "Failed: ${dev.flowclone.core.RouterErrors.userMessage(e.kind, e.message ?: "error")}"
            }
            runOnUiThread { probeLine.text = line; refreshRows() }
        }
    }

    private fun onResetCircuit() {
        val r = app.engine.router()
        if (r == null) { Toast.makeText(this, "No router loaded", Toast.LENGTH_SHORT).show(); return }
        runCatching { r.resetCircuit() }
        refreshRows()
    }

    private fun refreshRows() {
        val last = app.lastRoute.value
        lastLine.text = DevText.lastDictation(last?.result) + (last?.let { " · " + DateFormat.getTimeInstance().format(Date(it.atMs)) } ?: "")
        val rs = app.engine.router()?.let { runCatching { it.routeStatus() }.getOrNull() }
        circuitLine.text = if (rs == null) "router not loaded (${app.engine.state.value::class.simpleName})"
        else DevText.circuit(rs.circuit, System.currentTimeMillis()) { DateFormat.getTimeInstance().format(Date(it)) }
        serverInfo.text = DevText.serverInfo(lastProbe ?: rs?.health)
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
