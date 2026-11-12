package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
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
import dev.murmur.data.SettingsSummary
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * D78 "Cloud": the one page for online models (D28 replaced the D18 provider list), in the app
 * design's look (top bar, cards; desktop SettingsOpenRouter.dc.html "Settings · Cloud" adapted to
 * the phone). Reached from Settings → Connections "Cloud" and from Developer settings.
 *   PROVIDER         OpenRouter | LiteLLM switch (one provider at a time, [OpenRouterPage.withProvider]),
 *                    what each one is and offers
 *   API KEY          OpenRouter: SecretField (never in the a11y tree), Paste / Clear, Save, Check key
 *   PRIVACY          OpenRouter: zero-retention providers only, deny data collection (confirm to turn off)
 *   YOUR LITELLM PROXY  LiteLLM (D52 server): address, optional key, Test connection (fills the
 *                    model list), model, time limit
 *   SPEECH-TO-TEXT   opt-in + its consent dialog, model → picker, language, dictionary; Test.
 *                    "Not available" with LiteLLM (cleanup only)
 *   TEXT CLEANUP     opt-in + the provider's own consent dialog, model → picker, provider choice,
 *                    time limit, what is sent (PromptScope), live status
 *   USAGE            this month's cost (total / speech / cleanup), requests, budget, reset; the proxy's line
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
    private lateinit var providerResult: DevUi.Result
    private lateinit var testResult: DevUi.Result
    private lateinit var customKeyField: EditText
    private lateinit var customResult: DevUi.Result
    /** The proxy's models from the last Test connection (the Model row picks from them). */
    private var proxyModels: List<String> = emptyList()

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

    /** The page under the design's top bar (back, "Cloud"). */
    private fun page(build: LinearLayout.() -> Unit) = with(ui) {
        screen(header = { topBar(TITLE, DevUi.TopNav.BACK, onNav = ::finish) }, build = build)
    }

    private fun refresh() { val s = app.dev.load(); for (r in refreshers) r(s) }

    private fun renderNotInBuild() = with(ui) {
        page {
            card {
                toggleRow(STT_SWITCH, CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                toggleRow(CLEANUP_SWITCH, CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                note("This build has no cloud client: nothing leaves the phone except to your own server.")
            }
        }
    }

    /** [views] shown only while [provider] is the Cloud provider. */
    private fun onlyFor(provider: CleanupProvider, vararg views: View) {
        refreshers += { s -> val v = if (s.cloud.cleanupProvider == provider) View.VISIBLE else View.GONE; for (x in views) x.visibility = v }
    }

    private fun render() = with(ui) {
        val s0 = app.dev.load()
        page {
            noticeBanner()
            note(CloudText.INTRO)

            providerCard(s0)

            val keyLabel = sectionLabel("API KEY")
            val keyCard = card {
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

            val privacyLabel = sectionLabel("PRIVACY")
            val privacyCard = card {
                val zdr = toggleRow("Zero-retention providers only", "Providers that keep no copy of your audio or text", s0.cloud.privacy.zdr)
                val deny = toggleRow("Deny data collection", "Never use providers that may store or train on your data", s0.cloud.privacy.denyDataCollection)
                note(CloudText.PRIVACY_NOTE)
                zdr.onChange { on -> if (!quiet) onPrivacy(zdr, on, isZdr = true) }
                deny.onChange { on -> if (!quiet) onPrivacy(deny, on, isZdr = false) }
                refreshers += { s -> quietly { zdr.isChecked = s.cloud.privacy.zdr; deny.isChecked = s.cloud.privacy.denyDataCollection } }
            }
            onlyFor(CleanupProvider.OPENROUTER, keyLabel, keyCard, privacyLabel, privacyCard)

            customCard()
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
                    budget.setSubtitle(s.cloud.monthlyBudgetUsd?.let { "${CloudText.usd(it)} per month for OpenRouter speech and cleanup together; both pause once it is reached" } ?: "None")
                }
            }
            if (app.dev.visible) note("The cloud's place among your server and the phone: Developer settings → Routing & engines.")
        }
        refreshers += { _ -> keyLine.text = app.cloudKeys.masked(OpenRouter.ID) }
    }

    /** One-time notice (D28 migration, D78 one provider) until dismissed. */
    private fun LinearLayout.noticeBanner() = with(ui) {
        val text = app.dev.cloudNotice ?: return@with
        val b = banner()
        b.show(text, Tone.WARN)
        chipRow(chip("Dismiss") { app.dev.cloudNotice = null; b.show(""); it.visibility = View.GONE }.apply { contentDescription = "Dismiss the cloud notice" })
    }

    /** D78: the OpenRouter | LiteLLM switch. */
    private fun LinearLayout.providerCard(s0: DevSettings) = with(ui) {
        sectionLabel("PROVIDER")
        card {
            val seg = segmented(PROVIDERS.map { it.second }, PROVIDERS.indexOfFirst { it.first == s0.cloud.cleanupProvider }) { i -> selectProvider(PROVIDERS[i].first) }
            seg.view.contentDescription = "Cloud provider"
            val desc = note("")
            val offers = note("")
            refreshers += { s ->
                val custom = s.cloud.customProvider
                seg.select(if (custom) 1 else 0)
                desc.text = if (custom) CloudText.LITELLM_DESC else CloudText.OPENROUTER_DESC
                offers.text = if (custom) CloudText.LITELLM_OFFERS else CloudText.OPENROUTER_OFFERS
            }
        }
        providerResult = result()
    }

    private fun LinearLayout.sttCard(s0: DevSettings) = with(ui) {
        sectionLabel("SPEECH-TO-TEXT")
        card {
            val on = toggleRow(STT_SWITCH, STT_DESC, s0.allowCloud)
            val notAvailable = note(CloudText.CUSTOM_STT_NOTE)
            val model = navRow(R.drawable.ic_set_mic, "Model") { ModelPickerActivity.open(this@OpenRouterActivity, CloudCatalog.Use.STT) }
            val lang = navRow(R.drawable.ic_set_language, "Language") { editLanguage() }
            val dict = toggleRow("Send dictionary words", "Helps spell names (Whisper on Groq only)", s0.cloud.stt.sendDictionary)
            val consent = infoRow("Consent to send audio")
            on.onChange { v -> if (!quiet) onStt(on, v) }
            dict.onChange { v -> if (!quiet) change { OpenRouterPage.withStt(it) { copy(sendDictionary = v) } } }
            refreshers += { s ->
                // D78: LiteLLM does cleanup only; the switch stays off and says why.
                val available = OpenRouterPage.sttAvailable(s)
                quietly { on.isChecked = s.allowCloud && available; dict.isChecked = s.cloud.stt.sendDictionary }
                on.isEnabled = available
                on.setDescription(if (available) STT_DESC else CloudText.STT_NOT_AVAILABLE)
                notAvailable.visibility = if (available) View.GONE else View.VISIBLE
                for (row in listOf(model.view, lang.view, dict.row, consent.parent as View)) row.visibility = if (available) View.VISIBLE else View.GONE
                model.setSubtitle(CloudCatalog.modelSummary(s.cloud.stt.model, s.cloud.stt.sttPath))
                lang.setSubtitle(s.cloud.stt.language.ifEmpty { "Follow your dictation languages" })
                consent.text = OpenRouterPage.consentLine(s.cloud.stt.consentAtMs, s.cloud.stt.consentOk, "speech-to-text", ::date)
            }
        }
        val test = action("Test speech-to-text") { onTestStt() }
        testResult = result()
        val testNote = note("Uploads a 3 s public-domain clip to OpenRouter with the model above (costs a fraction of a cent).")
        onlyFor(CleanupProvider.OPENROUTER, test, testNote)
    }

    private fun LinearLayout.cleanupCard(s0: DevSettings) = with(ui) {
        sectionLabel("TEXT CLEANUP")
        card {
            val on = toggleRow(CLEANUP_SWITCH, "Sends the dictated text (not your voice) after each dictation; off by default", s0.allowCloudCleanup)
            val model = navRow(R.drawable.ic_set_sparkle, "Model") { ModelPickerActivity.open(this@OpenRouterActivity, CloudCatalog.Use.CLEANUP) }
            val sort = navRow(R.drawable.ic_dev_route, "Provider choice") { editSort() }
            val timeout = navRow(R.drawable.ic_set_timer, "Time limit") { editTimeout() }
            val consent = infoRow("Consent to send text")
            val status = infoRow("Status")
            on.onChange { v -> if (!quiet) onCleanup(on, v) }
            refreshers += { s ->
                val custom = s.cloud.customProvider
                quietly { on.isChecked = s.allowCloudCleanup }
                // LiteLLM's model and time limit are in its own card above.
                for (row in listOf(model, sort, timeout)) row.view.visibility = if (custom) View.GONE else View.VISIBLE
                model.setSubtitle(CloudCatalog.modelSummary(s.cloud.cleanup.model))
                sort.setSubtitle(CloudInput.sortName(s.cloud.cleanup.sort))
                timeout.setSubtitle("${s.cloud.cleanup.timeoutMs} ms, then the text is pasted without it")
                consent.text = OpenRouterPage.cleanupConsentLine(s.cloud, ::date)
                // D52: the router got no cleanup consent for an unusable proxy; say so here.
                status.text = if (s.cloud.customNotSetUp) CustomServer.NOT_SET_UP
                    else app.engine.router()?.let { DevText.cleanupStatus(it.cleanupStatus()) } ?: "Speech engine not loaded"
            }
        }
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

    /** D52/D78: the user's LiteLLM proxy (or vLLM, Ollama, … any OpenAI-compatible server), for cleanup only. */
    private fun LinearLayout.customCard() = with(ui) {
        val label = sectionLabel("YOUR LITELLM PROXY")
        val box = card {
            val notSetUp = note(CustomServer.NOT_SET_UP, color = p.danger)
            val url = navRow(R.drawable.ic_dev_route, "Address") { editCustomUrl() }
            note("Any OpenAI-compatible server works. ${CustomServer.URL_HINT}")
            val http = note(CloudText.HTTP_WARNING, color = p.danger)
            val keyLine = infoRow("Stored key")
            customKeyField = secretRow(
                "API key (optional)", "", "Only if your proxy asks for one (e.g. a LiteLLM virtual key). Stored only Keystore-sealed on this phone, sent only to this address.",
                "proxy key", onEdit = {}, clearLabel = "Clear", onClear = { confirmClearCustomKey() },
            ).edit
            chipRow(
                chip("Save key") { saveCustomKey() }.apply { contentDescription = "Save the proxy key" },
                chip("Test connection") { testCustom() }.apply { contentDescription = "Test the connection to your proxy and load its models" },
            )
            val model = navRow(R.drawable.ic_set_sparkle, "Model") { pickCustomModel() }
            val timeout = navRow(R.drawable.ic_set_timer, "Time limit") { editCustomTimeout() }
            refreshers += { s ->
                val cloud = s.cloud
                val c = cloud.customCleanup
                notSetUp.visibility = if (cloud.customNotSetUp) View.VISIBLE else View.GONE
                url.setSubtitle(c.baseUrl.ifEmpty { "Not set: e.g. http://192.168.1.20:4000" })
                http.visibility = if (c.baseUrl.isNotEmpty() && !c.https) View.VISIBLE else View.GONE
                keyLine.text = app.cloudKeys.masked(CustomServer.ID)
                model.setSubtitle(c.model.ifEmpty { if (proxyModels.isEmpty()) "Not set: Test connection loads the list" else "Not set: pick one of ${proxyModels.size}" })
                timeout.setSubtitle("${c.timeoutMs} ms, then the text is pasted without it")
            }
        }
        customResult = result()
        onlyFor(CleanupProvider.CUSTOM, label, box)
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
        // D78: the switch is disabled with LiteLLM; a stale tap still says why instead of turning it on.
        if (!OpenRouterPage.sttAvailable(s)) { quietly { row.isChecked = false }; providerResult.show(CloudText.LITELLM_NO_STT, Tone.WARN); return }
        val enable = { change { OpenRouterPage.enableStt(it, System.currentTimeMillis()) }; refreshCatalog(); warnNoKey() }
        if (!OpenRouterPage.sttNeedsConsent(s)) { enable(); return }
        ui.confirm(CloudText.STT_CONSENT_TITLE, CloudText.sttConsentBody(s.cloud.stt), CloudText.STT_CONSENT_BUTTON, onYes = enable,
            onNo = { quietly { row.isChecked = false } })
    }

    private fun onCleanup(row: DevUi.ToggleRow, on: Boolean) {
        if (!on) { change(OpenRouterPage::disableCleanup); return }
        val s = app.dev.load()
        if (s.cloud.customNotSetUp) {
            // Nothing to consent to yet: the proxy's host is not known.
            quietly { row.isChecked = false }
            customResult.show((OpenRouterPage.customMissing(s.cloud.customCleanup) ?: "Check the time limit") + " above, then turn cleanup on.", Tone.WARN)
            return
        }
        val enable = {
            change { OpenRouterPage.enableCleanup(it, System.currentTimeMillis()) }
            if (!app.dev.load().cloud.customProvider) { refreshCatalog(); warnNoKey() }
        }
        if (!OpenRouterPage.cleanupNeedsConsent(s)) { enable(); return }
        val (title, body, button) = CloudText.cleanupConsent(s.cloud)
        ui.confirm(title, body, button, onYes = enable, onNo = { quietly { row.isChecked = false } })
    }

    /**
     * D52: after the provider or the proxy's host changed while cleanup is on, the new
     * destination's consent is asked; declining turns cleanup off. A proxy that is not set up has
     * no host to ask for yet.
     */
    private fun askConsentIfNeeded() {
        val s = app.dev.load()
        if (!s.allowCloudCleanup || s.cloud.customNotSetUp || !OpenRouterPage.cleanupNeedsConsent(s)) return
        val (title, body, button) = CloudText.cleanupConsent(s.cloud)
        ui.confirm(title, body, button,
            onYes = { change { OpenRouterPage.enableCleanup(it, System.currentTimeMillis()) } },
            onNo = { change(OpenRouterPage::disableCleanup) })
    }

    private fun warnNoKey() {
        if (!app.cloudKeys.has(OpenRouter.ID)) keyResult.show("Add your OpenRouter API key above: nothing is sent without one.", Tone.WARN)
    }

    // ---- provider (D78) and the LiteLLM proxy (D52) ----

    private fun selectProvider(p: CleanupProvider) {
        val cur = app.dev.load()
        if (cur.cloud.cleanupProvider == p) return
        val next = OpenRouterPage.withProvider(cur, p)
        change { next }
        providerResult.show(if (cur.allowCloud && !next.allowCloud) CloudText.LITELLM_STT_OFF_NOTICE else "", Tone.WARN)
        customResult.show(
            if (next.cloud.customNotSetUp) (OpenRouterPage.customMissing(next.cloud.customCleanup) ?: "Check the time limit") + " below; cleanup uses the proxy once its address and model are set."
            else "",
            Tone.WARN,
        )
        askConsentIfNeeded()
    }

    /** A proxy field changed: re-ask a voided consent. */
    private fun customChanged(f: (DevSettings) -> DevSettings) {
        change(f)
        if (!app.dev.load().cloud.customNotSetUp) customResult.show("")
        if (app.dev.load().cloud.customProvider) askConsentIfNeeded()
    }

    private fun editCustomUrl() = textDialog(
        "Proxy address", app.dev.load().cloud.customCleanup.baseUrl, "http://192.168.1.20:4000", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI,
        "The OpenAI-compatible API root. ${CustomServer.URL_HINT} Consent is for this host only.",
        { CloudInput.baseUrl(it).error },
    ) { raw ->
        val v = CloudInput.baseUrl(raw).value!!
        if (v != app.dev.load().cloud.customCleanup.baseUrl) proxyModels = emptyList()
        customChanged { OpenRouterPage.withCustom(it) { copy(baseUrl = v) } }
    }

    private fun editCustomModel() = textDialog(
        "Model", app.dev.load().cloud.customCleanup.model, "gpt-4o-mini", InputType.TYPE_CLASS_TEXT,
        "As your proxy lists it (Test connection loads the list).", { CloudInput.customModel(it).error },
    ) { raw -> val v = CloudInput.customModel(raw).value!!; customChanged { OpenRouterPage.withCustom(it) { copy(model = v) } } }

    /** The Model row: the list from the last Test connection (and typing one), else typing. */
    private fun pickCustomModel() {
        val models = proxyModels
        if (models.isEmpty()) { editCustomModel(); return }
        val cur = app.dev.load().cloud.customCleanup.model
        val items = models + TYPE_MODEL
        AlertDialog.Builder(this).setTitle("Model")
            .setSingleChoiceItems(items.toTypedArray(), models.indexOf(cur)) { d, i ->
                d.dismiss()
                if (i == models.size) { editCustomModel(); return@setSingleChoiceItems }
                val v = models[i]
                if (CustomServer.validModelId(v)) customChanged { OpenRouterPage.withCustom(it) { copy(model = v) } }
                else customResult.show("That id has spaces or is too long: type it with $TYPE_MODEL instead.", Tone.BAD)
            }
            .setNegativeButton("Cancel", null).show()
    }

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
        ui.confirm("Delete the proxy key?", "Removes your LiteLLM proxy's key from this phone. Requests then go without one.", "Delete", onYes = {
            app.cloudKeys.remove(CustomServer.ID)
            app.engine.setCloudKey(null, CustomServer.ID)
            customKeyField.setText("")
            customResult.show("Key deleted from this phone.")
            refresh()
        })
    }

    /**
     * Test connection: `GET {base}/models` off the main thread with the typed key only (the router
     * lends the stored one just for the consented host). The list it returns fills the Model row;
     * with no model picked yet, the list opens right away.
     */
    private fun testCustom() {
        val url = app.dev.load().cloud.customCleanup.baseUrl
        if (url.isEmpty()) { customResult.show("Set the proxy address first.", Tone.BAD); return }
        val typed = dev.murmur.data.CustomModelsKey.typedOnly(customKeyField.text.toString())
        customResult.show("Connecting to your proxy…")
        thread(name = "custom-models") {
            val m = runCatching { app.engine.customCleanupModels(url, typed) }
                .getOrElse { dev.murmur.core.CustomModels(error = "error", message = it.javaClass.simpleName) }
            m.error?.let { app.debugLog.event("custom_models", "error=$it") }
            runOnUiThread {
                if (isFinishing) return@runOnUiThread
                customResult.show(CloudText.customModelsLine(m), if (m.error != null) Tone.BAD else if (m.models.isEmpty()) Tone.WARN else Tone.OK)
                if (m.error != null) return@runOnUiThread
                proxyModels = m.models
                refresh()
                if (m.models.isNotEmpty() && app.dev.load().cloud.customCleanup.model.isEmpty()) pickCustomModel()
            }
        }
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
        InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_DECIMAL, "Empty = none. OpenRouter speech and cleanup share it; your proxy is not counted.",
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
        /** D78: the umbrella name; the provider is named only in the switch and its fields. */
        const val TITLE = SettingsSummary.CLOUD
        const val STT_SWITCH = "Use the cloud for speech"
        /** The cleanup switch keeps one title whichever provider is picked. */
        const val CLEANUP_SWITCH = "Use the cloud for cleanup"
        private const val STT_DESC = "Sends the audio of your dictations to OpenRouter; off by default"
        private const val TYPE_MODEL = "Type a model id…"
        /** The switch's segments, in order. */
        val PROVIDERS = listOf(CleanupProvider.OPENROUTER to OpenRouter.NAME, CleanupProvider.CUSTOM to CustomServer.NAME)
        fun open(ctx: Context) = ctx.startActivity(Intent(ctx, OpenRouterActivity::class.java))
        private fun String.cap() = replaceFirstChar { it.uppercase() }
    }
}
