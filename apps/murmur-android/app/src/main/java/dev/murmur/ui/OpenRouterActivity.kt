package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.DevText
import dev.murmur.core.NativeMurmurCore
import dev.murmur.data.CloudCatalog
import dev.murmur.data.CloudInput
import dev.murmur.data.CloudText
import dev.murmur.data.CloudUsage
import dev.murmur.data.CleanupProvider
import dev.murmur.data.CustomServer
import dev.murmur.data.DevSettings
import dev.murmur.data.OpenRouter
import dev.murmur.data.OpenRouterPage
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * D28: the one cloud page (docs/proposals/openrouter-only-cloud.md; replaces the D18 provider
 * list), in the app design's look (top bar, cards; desktop SettingsOpenRouter.dc.html adapted to
 * the phone). Reached from Settings → Connections "OpenRouter" and from Developer settings.
 *   API KEY          SecretField (never in the a11y tree), Paste / Clear, Save, Check key
 *   PRIVACY          zero-retention providers only, deny data collection (confirm to turn off)
 *   SPEECH-TO-TEXT   opt-in + its consent dialog, model → picker, language, dictionary; Test
 *   TEXT CLEANUP     opt-in + its own consent dialog, service (D52: OpenRouter or a custom
 *                    OpenAI-compatible server), model → picker, order, provider choice,
 *                    time limit, what is sent (PromptScope), live status
 *   CUSTOM SERVER    D52: base URL, optional key, model (typed or loaded), time limit, Test
 *   USAGE            this month's cost (total / speech / cleanup), requests, budget, reset
 * Every change applies at once (store + live router), like the old consent dialog did.
 */
class OpenRouterActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    /** Redraws, each given the one settings value [refresh] loaded (one custom-address lookup per refresh, D52). */
    private val refreshers = ArrayList<(DevSettings) -> Unit>()
    /** Set while a switch is flipped back in code (a cancelled dialog), so its listener ignores it. */
    private var quiet = false

    private lateinit var keyLine: TextView
    private lateinit var keyField: EditText
    private lateinit var keyResult: DevUi.Result
    private lateinit var testResult: DevUi.Result
    private lateinit var customKeyField: EditText
    private lateinit var customResult: DevUi.Result
    /** "Custom server" was picked before it was set up: its card shows, and it is selected once ready. */
    private var pickingCustom = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = TITLE
        ui = DevUi(this)
        if (!caps.cloud) { renderNotInBuild(); return }
        app.migrateCloud()
        render()
        refresh()
        // §2.2: the page opening is one of the two moments the catalogue may be fetched (24 h cache).
        refreshCatalog()
    }

    override fun onResume() {
        super.onResume()
        refresh()
    }

    /** The page under the design's top bar (back, "Cloud server"). */
    private fun page(build: LinearLayout.() -> Unit) = with(ui) {
        screen {
            topBar(TITLE, DevUi.TopNav.BACK, onNav = ::finish)
            build()
        }
    }

    private fun refresh() { val s = app.dev.load(); for (r in refreshers) r(s) }

    private fun renderNotInBuild() = with(ui) {
        page {
            card {
                toggleRow("Cloud speech-to-text", CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                toggleRow(CLEANUP_SWITCH, CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                note("This build has no cloud client: nothing leaves the phone except to your own server.")
            }
        }
    }

    private fun render() = with(ui) {
        val s0 = app.dev.load()
        page {
            noticeBanner()
            note(CloudText.INTRO)

            sectionLabel("API KEY")
            card {
                keyLine = infoRow("Stored key")
                keyField = secretRow(
                    "OpenRouter API key", "", "Paste a key from openrouter.ai/keys (${OpenRouter.KEY_HINT}). Stored only Keystore-sealed on this phone; one key for both uses.",
                    "API key", onEdit = {}, clearLabel = "Clear", onClear = { confirmClearKey() },
                ).edit
                chipRow(
                    chip("Save key") { saveKey() }.apply { contentDescription = "Save the API key" },
                    chip("Check key") { checkKey() }.apply { contentDescription = "Check the stored API key with OpenRouter" },
                )
            }
            keyResult = result()

            sectionLabel("PRIVACY")
            card {
                val zdr = toggleRow("Zero-retention providers only", "Providers that keep no copy of your audio or text", s0.cloud.privacy.zdr)
                val deny = toggleRow("Deny data collection", "Never use providers that may store or train on your data", s0.cloud.privacy.denyDataCollection)
                note(CloudText.PRIVACY_NOTE)
                zdr.onChange { on -> if (!quiet) onPrivacy(zdr, on, isZdr = true) }
                deny.onChange { on -> if (!quiet) onPrivacy(deny, on, isZdr = false) }
                refreshers += { s -> quietly { zdr.isChecked = s.cloud.privacy.zdr; deny.isChecked = s.cloud.privacy.denyDataCollection } }
            }

            sttCard(s0)
            cleanupCard(s0)

            sectionLabel("USAGE")
            val stats = statRow(listOf("$0" to "this month", "$0" to "speech", "$0" to "cleanup"))
            card {
                val usage = infoRow("Requests and budget (counted on this phone only, never sent anywhere)")
                val budget = navRow(R.drawable.ic_dev_cloud, "Monthly budget") { editBudget() }
                chipRow(chip("Reset counters") { onResetUsage() }.apply { contentDescription = "Reset cloud usage counters" })
                refreshers += { s ->
                    val u = CloudUsage.decode(app.engine.router()?.cloudUsageJson() ?: app.dev.cloudUsageJson)
                    CloudText.usageStats(u, s.cloud).forEachIndexed { i, (n, unit) -> stats.set(i, n, unit) }
                    usage.text = CloudText.usageLines(u, s.cloud).drop(1).joinToString("\n")
                    budget.setSubtitle(s.cloud.monthlyBudgetUsd?.let { "${CloudText.usd(it)} per month for speech and cleanup together; both pause once it is reached" } ?: "None")
                }
            }
            if (app.dev.visible) note("The cloud's place among your server and the phone: Developer settings → Routing & engines.")
        }
        refreshers += { _ -> keyLine.text = app.cloudKeys.masked(OpenRouter.ID) }
    }

    /** One-time migration notice (D28) until dismissed. */
    private fun LinearLayout.noticeBanner() = with(ui) {
        val text = app.dev.cloudNotice ?: return@with
        val b = banner()
        b.show(text, Tone.WARN)
        chipRow(chip("Dismiss") { app.dev.cloudNotice = null; b.show(""); it.visibility = android.view.View.GONE }.apply { contentDescription = "Dismiss the cloud notice" })
    }

    private fun LinearLayout.sttCard(s0: DevSettings) = with(ui) {
        sectionLabel("SPEECH-TO-TEXT")
        card {
            val on = toggleRow("Cloud speech-to-text", "Sends the audio of your dictations to OpenRouter; off by default", s0.allowCloud)
            val model = navRow(R.drawable.ic_set_mic, "Model") { ModelPickerActivity.open(this@OpenRouterActivity, CloudCatalog.Use.STT) }
            val lang = navRow(R.drawable.ic_set_language, "Language") { editLanguage() }
            val dict = toggleRow("Send dictionary words", "Helps spell names (Whisper on Groq only)", s0.cloud.stt.sendDictionary)
            val consent = infoRow("Consent to send audio")
            on.onChange { v -> if (!quiet) onStt(on, v) }
            dict.onChange { v -> if (!quiet) change { OpenRouterPage.withStt(it) { copy(sendDictionary = v) } } }
            refreshers += { s ->
                quietly { on.isChecked = s.allowCloud; dict.isChecked = s.cloud.stt.sendDictionary }
                model.setSubtitle(CloudCatalog.modelSummary(s.cloud.stt.model, s.cloud.stt.sttPath))
                lang.setSubtitle(s.cloud.stt.language.ifEmpty { "Follow your dictation languages" })
                consent.text = OpenRouterPage.consentLine(s.cloud.stt.consentAtMs, s.cloud.stt.consentOk, "speech-to-text", ::date)
            }
        }
        action("Test speech-to-text") { onTestStt() }
        testResult = result()
        note("Uploads a 3 s public-domain clip to OpenRouter with the model above (costs a fraction of a cent).")
    }

    private fun LinearLayout.cleanupCard(s0: DevSettings) = with(ui) {
        sectionLabel("TEXT CLEANUP")
        card {
            val on = toggleRow(CLEANUP_SWITCH, "Sends the dictated text (not your voice) after each dictation; off by default", s0.allowCloudCleanup)
            val service = navRow(R.drawable.ic_dev_cloud, "Cleanup service") { editProvider() }
            val model = navRow(R.drawable.ic_set_sparkle, "Model") { ModelPickerActivity.open(this@OpenRouterActivity, CloudCatalog.Use.CLEANUP) }
            val sort = navRow(R.drawable.ic_dev_route, "Provider choice") { editSort() }
            val timeout = navRow(R.drawable.ic_set_timer, "Time limit") { editTimeout() }
            val consent = infoRow("Consent to send text")
            val status = infoRow("Status")
            on.onChange { v -> if (!quiet) onCleanup(on, v) }
            refreshers += { s ->
                val custom = s.cloud.customProvider
                quietly { on.isChecked = s.allowCloudCleanup }
                service.setSubtitle(if (custom) s.cloud.cleanupProviderName else OpenRouter.NAME)
                for (row in listOf(model, sort, timeout)) row.view.visibility = if (custom) android.view.View.GONE else android.view.View.VISIBLE
                model.setSubtitle(CloudCatalog.modelSummary(s.cloud.cleanup.model))
                sort.setSubtitle(CloudInput.sortName(s.cloud.cleanup.sort))
                timeout.setSubtitle("${s.cloud.cleanup.timeoutMs} ms, then the text is pasted without it")
                consent.text = OpenRouterPage.cleanupConsentLine(s.cloud, ::date)
                // D52: the router got no cleanup consent for an unusable custom server; say so here.
                status.text = if (s.cloud.customNotSetUp) CustomServer.NOT_SET_UP
                    else app.engine.router()?.let { DevText.cleanupStatus(it.cleanupStatus()) } ?: "Speech engine not loaded"
            }
        }
        customCard()
        sectionLabel("WHAT CLEANUP SENDS")
        card {
            val always = note("")
            val around = toggleRow("Send text around the cursor", "Up to 300 characters before and 150 after, and a selection", s0.cloud.cleanup.sendSurroundingText)
            val screen = toggleRow("Send on-screen words", "Names and words visible in the app", s0.cloud.cleanup.sendScreenWords)
            val appName = toggleRow("Send app name", "The app's package name, not just its kind", s0.cloud.cleanup.sendAppName)
            around.onChange { v -> if (!quiet) change { OpenRouterPage.withCleanup(it) { copy(sendSurroundingText = v) } } }
            screen.onChange { v -> if (!quiet) change { OpenRouterPage.withCleanup(it) { copy(sendScreenWords = v) } } }
            appName.onChange { v -> if (!quiet) change { OpenRouterPage.withCleanup(it) { copy(sendAppName = v) } } }
            refreshers += { s ->
                val c = s.cloud.cleanup
                quietly { around.isChecked = c.sendSurroundingText; screen.isChecked = c.sendScreenWords; appName.isChecked = c.sendAppName }
                always.text = "Sent with each cleanup: ${CloudText.cleanupSends(c)}."
            }
        }
        note("Which engine cleans first (this phone, your server or the cloud) is set in Settings → AI cleanup. About half of dictations need no AI and send nothing.")
    }

    /** D52: the user's own OpenAI-compatible server (LiteLLM proxy, vLLM, Ollama, …) for cleanup only. */
    private fun LinearLayout.customCard() = with(ui) {
        val label = sectionLabel("CUSTOM SERVER")
        val box = card {
            note(CloudText.CUSTOM_STT_NOTE)
            val notSetUp = note(CustomServer.NOT_SET_UP, color = p.danger)
            val url = navRow(R.drawable.ic_dev_route, "Server address") { editCustomUrl() }
            note(CustomServer.URL_HINT)
            val http = note(CloudText.HTTP_WARNING, color = p.danger)
            val keyLine = infoRow("Stored key")
            customKeyField = secretRow(
                "API key (optional)", "", "Only if your server asks for one (e.g. a LiteLLM virtual key). Stored only Keystore-sealed on this phone.",
                "custom server key", onEdit = {}, clearLabel = "Clear", onClear = { confirmClearCustomKey() },
            ).edit
            val model = navRow(R.drawable.ic_set_sparkle, "Model") { editCustomModel() }
            val timeout = navRow(R.drawable.ic_set_timer, "Time limit") { editCustomTimeout() }
            chipRow(
                chip("Save key") { saveCustomKey() }.apply { contentDescription = "Save the custom server key" },
                chip("Load models") { loadCustomModels() }.apply { contentDescription = "Load the model list from your server" },
                chip("Test connection") { testCustom() }.apply { contentDescription = "Test the connection to your server" },
            )
            refreshers += { s ->
                val cloud = s.cloud
                val c = cloud.customCleanup
                notSetUp.visibility = if (cloud.customNotSetUp) android.view.View.VISIBLE else android.view.View.GONE
                url.setSubtitle(c.baseUrl.ifEmpty { "Not set: e.g. http://192.168.1.20:4000" })
                http.visibility = if (c.baseUrl.isNotEmpty() && !c.https) android.view.View.VISIBLE else android.view.View.GONE
                keyLine.text = app.cloudKeys.masked(CustomServer.ID)
                model.setSubtitle(c.model.ifEmpty { "Not set" })
                timeout.setSubtitle("${c.timeoutMs} ms, then the text is pasted without it")
            }
        }
        customResult = result()
        refreshers += { s ->
            val show = s.cloud.customProvider || pickingCustom
            for (v in listOf(label, box)) v.visibility = if (show) android.view.View.VISIBLE else android.view.View.GONE
        }
    }

    // ---- changes ----

    private fun quietly(f: () -> Unit) { quiet = true; try { f() } finally { quiet = false } }

    /** Store + live router, then redraw. */
    private fun change(f: (DevSettings) -> DevSettings) {
        val s = f(app.dev.load())
        app.dev.saveCloud(
            allowCloud = s.allowCloud, cloud = s.cloud, routeOrder = s.routeOrder,
            allowCloudCleanup = s.allowCloudCleanup, cleanupOrder = s.cleanupOrder, cleanupOff = s.cleanupOff,
        )
        app.engine.applyDevSettings(app.dev.load())
        refresh()
    }

    private fun onStt(row: DevUi.ToggleRow, on: Boolean) {
        if (!on) { change(OpenRouterPage::disableStt); return }
        val s = app.dev.load()
        val enable = { change { OpenRouterPage.enableStt(it, System.currentTimeMillis()) }; refreshCatalog(); warnNoKey() }
        if (!OpenRouterPage.sttNeedsConsent(s)) { enable(); return }
        ui.confirm(CloudText.STT_CONSENT_TITLE, CloudText.sttConsentBody(s.cloud.stt), CloudText.STT_CONSENT_BUTTON, onYes = enable,
            onNo = { quietly { row.isChecked = false } })
    }

    private fun onCleanup(row: DevUi.ToggleRow, on: Boolean) {
        if (!on) { change(OpenRouterPage::disableCleanup); return }
        val s = app.dev.load()
        val enable = {
            change { OpenRouterPage.enableCleanup(it, System.currentTimeMillis()) }
            if (!app.dev.load().cloud.customProvider) { refreshCatalog(); warnNoKey() }
        }
        if (!OpenRouterPage.cleanupNeedsConsent(s)) { enable(); return }
        val (title, body, button) = CloudText.cleanupConsent(s.cloud)
        ui.confirm(title, body, button, onYes = enable, onNo = { quietly { row.isChecked = false } })
    }

    /**
     * D52: after the cleanup service or the custom server's host changed while cleanup is on, the
     * new destination's consent is asked; declining turns cleanup off.
     */
    private fun askConsentIfNeeded() {
        val s = app.dev.load()
        if (!s.allowCloudCleanup || !OpenRouterPage.cleanupNeedsConsent(s)) return
        val (title, body, button) = CloudText.cleanupConsent(s.cloud)
        ui.confirm(title, body, button,
            onYes = { change { OpenRouterPage.enableCleanup(it, System.currentTimeMillis()) } },
            onNo = { change(OpenRouterPage::disableCleanup) })
    }

    private fun warnNoKey() {
        if (!app.cloudKeys.has(OpenRouter.ID)) keyResult.show("Add your OpenRouter API key above: nothing is sent without one.", Tone.WARN)
    }

    // ---- custom server (D52) ----

    private fun editProvider() {
        val cur = app.dev.load().cloud.cleanupProvider
        val opts = listOf(CleanupProvider.OPENROUTER to OpenRouter.NAME, CleanupProvider.CUSTOM to "Custom server (LiteLLM / OpenAI-compatible)")
        AlertDialog.Builder(this).setTitle("Cleanup service")
            .setSingleChoiceItems(opts.map { it.second }.toTypedArray(), opts.indexOfFirst { it.first == cur }) { d, i ->
                d.dismiss()
                selectProvider(opts[i].first)
            }
            .setNegativeButton("Cancel", null).show()
    }

    private fun selectProvider(p: CleanupProvider) {
        val next = OpenRouterPage.withProvider(app.dev.load(), p)
        if (next == null) {
            // Not set up yet: show its card; it is selected once the address and model are set.
            pickingCustom = true
            refresh()
            customResult.show((OpenRouterPage.customMissing(app.dev.load().cloud.customCleanup) ?: "Check the time limit") + " below; the custom server is used once its address and model are set.", Tone.WARN)
            return
        }
        pickingCustom = false
        change { next }
        customResult.show("")
        askConsentIfNeeded()
    }

    /** A custom field changed: select the server if it was being picked and is now ready; re-ask a voided consent. */
    private fun customChanged(f: (DevSettings) -> DevSettings) {
        change(f)
        if (pickingCustom && OpenRouterPage.withProvider(app.dev.load(), CleanupProvider.CUSTOM) != null) { selectProvider(CleanupProvider.CUSTOM); return }
        if (app.dev.load().cloud.customProvider) askConsentIfNeeded()
    }

    private fun editCustomUrl() = textDialog(
        "Server address", app.dev.load().cloud.customCleanup.baseUrl, "http://192.168.1.20:4000", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI,
        "The OpenAI-compatible API root. ${CustomServer.URL_HINT} Consent is for this host only.",
        { CloudInput.baseUrl(it).error },
    ) { raw -> val v = CloudInput.baseUrl(raw).value!!; customChanged { OpenRouterPage.withCustom(it) { copy(baseUrl = v) } } }

    private fun editCustomModel() = textDialog(
        "Model", app.dev.load().cloud.customCleanup.model, "gpt-4o-mini", InputType.TYPE_CLASS_TEXT,
        "As your server lists it (Load models shows the list).", { CloudInput.customModel(it).error },
    ) { raw -> val v = CloudInput.customModel(raw).value!!; customChanged { OpenRouterPage.withCustom(it) { copy(model = v) } } }

    private fun editCustomTimeout() = textDialog(
        "Cleanup time limit (ms)", app.dev.load().cloud.customCleanup.timeoutMs.toString(), "4000", InputType.TYPE_CLASS_NUMBER,
        "${CustomServer.TIMEOUT_MIN}-${CustomServer.TIMEOUT_MAX}. After it, the formatted text is pasted and the request is cancelled.",
        { CloudInput.customTimeout(it).error },
    ) { raw -> val v = CloudInput.customTimeout(raw).value!!; customChanged { OpenRouterPage.withCustom(it) { copy(timeoutMs = v) } } }

    private fun saveCustomKey() {
        val k = customKeyField.text.toString().trim()
        if (k.isEmpty()) { customResult.show("Paste the key first.", Tone.BAD); return }
        try {
            app.cloudKeys.set(CustomServer.ID, k)
        } catch (e: Exception) {
            customResult.show("Could not store the key on this phone (${e.javaClass.simpleName}).", Tone.BAD); return
        }
        app.engine.setCloudKey(k, CustomServer.ID)
        customKeyField.setText("")
        customResult.show("Key saved on this phone.", Tone.OK)
        refresh()
    }

    private fun confirmClearCustomKey() {
        if (!app.cloudKeys.has(CustomServer.ID)) { customKeyField.setText(""); return }
        ui.confirm("Delete the server key?", "Removes your custom server's key from this phone. Requests then go without one.", "Delete", onYes = {
            app.cloudKeys.remove(CustomServer.ID)
            app.engine.setCloudKey(null, CustomServer.ID)
            customKeyField.setText("")
            customResult.show("Key deleted from this phone.")
            refresh()
        })
    }

    /**
     * `GET {base}/models` off the main thread with the typed key only (the router lends the stored
     * one just for the consented host); [then] on the UI thread.
     */
    private fun fetchCustomModels(busy: String, then: (dev.murmur.core.CustomModels) -> Unit) {
        val url = app.dev.load().cloud.customCleanup.baseUrl
        if (url.isEmpty()) { customResult.show("Set the server address first.", Tone.BAD); return }
        val typed = dev.murmur.data.CustomModelsKey.typedOnly(customKeyField.text.toString())
        customResult.show(busy)
        thread(name = "custom-models") {
            val m = runCatching { app.engine.customCleanupModels(url, typed) }
                .getOrElse { dev.murmur.core.CustomModels(error = "error", message = it.javaClass.simpleName) }
            m.error?.let { app.debugLog.event("custom_models", "error=$it") }
            runOnUiThread { if (!isFinishing) then(m) }
        }
    }

    private fun testCustom() = fetchCustomModels("Connecting to your server…") { m ->
        customResult.show(CloudText.customModelsLine(m), if (m.error == null) Tone.OK else Tone.BAD)
    }

    private fun loadCustomModels() = fetchCustomModels("Loading models…") { m ->
        if (m.error != null || m.models.isEmpty()) { customResult.show(CloudText.customModelsLine(m), if (m.error == null) Tone.WARN else Tone.BAD); return@fetchCustomModels }
        customResult.show(CloudText.customModelsLine(m), Tone.OK)
        val cur = app.dev.load().cloud.customCleanup.model
        AlertDialog.Builder(this).setTitle("Model")
            .setSingleChoiceItems(m.models.toTypedArray(), m.models.indexOf(cur)) { d, i ->
                d.dismiss()
                val v = m.models[i]
                if (CustomServer.validModelId(v)) customChanged { OpenRouterPage.withCustom(it) { copy(model = v) } }
                else customResult.show("That id has spaces or is too long: type it with Model instead.", Tone.BAD)
            }
            .setNegativeButton("Cancel", null).show()
    }

    /** Turning a privacy guard off asks first; turning it on never does. */
    private fun onPrivacy(row: DevUi.ToggleRow, on: Boolean, isZdr: Boolean) {
        val update = { change { if (isZdr) OpenRouterPage.withPrivacy(it, zdr = on) else OpenRouterPage.withPrivacy(it, denyCollection = on) } }
        if (on) { update(); return }
        ui.confirm(
            if (isZdr) CloudText.ZDR_OFF_TITLE else CloudText.COLLECTION_OFF_TITLE,
            if (isZdr) CloudText.ZDR_OFF_BODY else CloudText.COLLECTION_OFF_BODY,
            "Turn off", onYes = update, onNo = { quietly { row.isChecked = true } },
        )
    }

    // ---- key ----

    private fun saveKey() {
        val k = keyField.text.toString().trim()
        if (k.isEmpty()) { keyResult.show("Paste the key first.", Tone.BAD); return }
        try {
            app.cloudKeys.set(OpenRouter.ID, k)
        } catch (e: Exception) {
            // Keystore unavailable (never the key in the message).
            keyResult.show("Could not store the key on this phone (${e.javaClass.simpleName}).", Tone.BAD); return
        }
        app.engine.setCloudKey(k)
        keyField.setText("")
        keyResult.show("Key saved on this phone.", Tone.OK)
        refresh()
    }

    private fun confirmClearKey() {
        if (!app.cloudKeys.has(OpenRouter.ID)) { keyField.setText(""); return }
        ui.confirm("Delete the API key?", "Removes the OpenRouter key from this phone. Cloud speech-to-text and cleanup stop until you add one.", "Delete", onYes = {
            app.cloudKeys.remove(OpenRouter.ID)
            app.engine.setCloudKey(null)
            keyField.setText("")
            keyResult.show("Key deleted from this phone.")
            refresh()
        })
    }

    private fun checkKey() {
        val r = app.engine.router() ?: run { keyResult.show("The speech engine is not loaded yet; try again in a moment.", Tone.BAD); return }
        if (!app.cloudKeys.has(OpenRouter.ID)) { keyResult.show("No key stored: paste one and tap Save key.", Tone.BAD); return }
        keyResult.show("Checking the key with OpenRouter…")
        thread(name = "or-key-info") {
            val k = runCatching { r.cloudKeyInfo() }.getOrElse { dev.murmur.core.CloudKeyInfo(error = "error") }
            k.limitRemaining?.let { rem -> app.debugLog.event("cloud_key_info", "limit_remaining=$rem usage_monthly=${k.usageMonthly}") }
            runOnUiThread { if (!isFinishing) keyResult.show(DevText.keyInfo(k), if (k.error == null) (if (k.warnNoLimit) Tone.WARN else Tone.OK) else Tone.BAD) }
        }
    }

    // ---- test ----

    private fun onTestStt() {
        val s = app.dev.load()
        val key = app.cloudKeys.get(OpenRouter.ID) ?: run { testResult.show("No API key stored.", Tone.BAD); return }
        testResult.show("Testing ${s.cloud.stt.model}…")
        val json = s.cloud.openRouterJson().toString()
        thread(name = "or-probe") {
            val h = app.engine.router()?.probeCloud(json, key) ?: NativeMurmurCore.probeCloud(json, key)
            if (!h.ok) app.debugLog.event("cloud_probe", DevText.cloudProbe(h))
            runOnUiThread { if (!isFinishing) { testResult.show(DevText.cloudProbe(h), if (h.ok) Tone.OK else Tone.BAD); refresh() } }
        }
    }

    // ---- small dialogs ----

    private fun textDialog(title: String, value: String, hint: String, type: Int, help: String, parse: (String) -> String?, onOk: (String) -> Unit) {
        val box = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(8), dp(20), 0) }
        val edit = EditText(this).apply { setText(value); this.hint = hint; inputType = type; isSingleLine = true; minHeight = dp(48) }
        val err = TextView(this).apply { textSize = 13f; setTextColor(ui.p.danger); text = help }
        box.addView(edit); box.addView(err)
        val d = AlertDialog.Builder(this).setTitle(title).setView(box)
            .setNegativeButton("Cancel", null).setPositiveButton("Save", null).show()
        d.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val e = parse(edit.text.toString())
            if (e != null) { err.text = e; return@setOnClickListener }
            onOk(edit.text.toString()); d.dismiss()
        }
    }

    private fun editBudget() = textDialog(
        "Monthly budget (USD)", CloudInput.budgetText(app.dev.load().cloud.monthlyBudgetUsd), "5",
        InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL, "Empty = none. Speech and cleanup share it.",
        { CloudInput.budget(it).error },
    ) { raw -> val v = CloudInput.budget(raw).value; change { it.copy(cloud = it.cloud.copy(monthlyBudgetUsd = v)) } }

    private fun editTimeout() = textDialog(
        "Cleanup time limit (ms)", app.dev.load().cloud.cleanup.timeoutMs.toString(), "2500", InputType.TYPE_CLASS_NUMBER,
        "${OpenRouter.CLEANUP_TIMEOUT_MIN}-${OpenRouter.CLEANUP_TIMEOUT_MAX}. After it, the formatted text is pasted and the request is cancelled.",
        { CloudInput.timeout(it).error },
    ) { raw -> val v = CloudInput.timeout(raw).value!!; change { OpenRouterPage.withCleanup(it) { copy(timeoutMs = v) } } }

    private fun editLanguage() = textDialog(
        "Speech-to-text language", app.dev.load().cloud.stt.language, "en", InputType.TYPE_CLASS_TEXT,
        "Empty = follow your dictation languages; e.g. en, de, pt-br", { CloudInput.language(it).error },
    ) { raw -> val v = CloudInput.language(raw).value!!; change { OpenRouterPage.withStt(it) { copy(language = v) } } }

    private fun editSort() {
        val cur = app.dev.load().cloud.cleanup.sort
        val opts = CloudInput.SORTS
        AlertDialog.Builder(this).setTitle("Provider choice")
            .setSingleChoiceItems(opts.map { it.second }.toTypedArray(), opts.indexOfFirst { it.first == cur }) { d, i ->
                val v = opts[i].first
                change { OpenRouterPage.withCleanup(it) { copy(sort = v) } }
                d.dismiss()
            }
            .setNegativeButton("Cancel", null).show()
    }

    private fun onResetUsage() {
        ui.confirm("Reset the counters?", "Sets this month's speech and cleanup cost on this phone to $0. The budget then counts from zero.", "Reset", onYes = {
            val r = app.engine.router()
            if (r != null) { r.resetCloudUsage(); app.engine.persistCloudUsage() } else app.dev.cloudUsageJson = null
            refresh()
        })
    }

    /** Background catalogue refresh (24 h cache: usually no network at all). */
    private fun refreshCatalog() {
        thread(name = "or-catalog") {
            runCatching { NativeMurmurCore.setCloudCacheDir(cacheDir.path) }
            val st = runCatching { app.engine.router()?.refreshCloudCatalog(false) ?: NativeMurmurCore.refreshCloudCatalog(false) }.getOrNull()
            st?.let { app.debugLog.event("cloud_catalog_refresh", it) }
        }
    }

    private fun date(ms: Long) = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(ms))
    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    companion object {
        const val TITLE = "Cloud server"
        /** The cleanup switch keeps one title whichever provider is picked; "Cleanup service" names it (D52). */
        const val CLEANUP_SWITCH = "Clean up text with the cloud server"
        fun open(ctx: Context) = ctx.startActivity(Intent(ctx, OpenRouterActivity::class.java))
        private fun String.cap() = replaceFirstChar { it.uppercase() }
    }
}
