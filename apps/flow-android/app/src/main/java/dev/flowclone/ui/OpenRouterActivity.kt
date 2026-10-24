package dev.flowclone.ui

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
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.DevText
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.data.CloudCatalog
import dev.flowclone.data.CloudInput
import dev.flowclone.data.CloudText
import dev.flowclone.data.CloudUsage
import dev.flowclone.data.DevSettings
import dev.flowclone.data.OpenRouter
import dev.flowclone.data.OpenRouterPage
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * D28: the one cloud page (docs/proposals/openrouter-only-cloud.md; replaces the D18 provider
 * list). Reached from Developer settings and from Settings → DICTATION "Cloud (OpenRouter)".
 *   API KEY          SecretField (never in the a11y tree), Paste / Clear, Save, Check key
 *   PRIVACY          zero-retention providers only, deny data collection (confirm to turn off)
 *   SPEECH-TO-TEXT   opt-in + its consent dialog, model → picker, language, dictionary; Test
 *   TEXT CLEANUP     opt-in + its own consent dialog, model → picker, order, provider choice,
 *                    time limit, what is sent (PromptScope), live status
 *   USAGE            speech vs cleanup cost this month, budget, reset
 * Every change applies at once (store + live router), like the old consent dialog did.
 */
class OpenRouterActivity : Activity() {
    private val app get() = FlowApp.of(this)
    private val caps get() = app.caps
    private lateinit var ui: DevUi
    private val refreshers = ArrayList<() -> Unit>()
    /** Set while a switch is flipped back in code (a cancelled dialog), so its listener ignores it. */
    private var quiet = false

    private lateinit var keyLine: TextView
    private lateinit var keyField: EditText
    private lateinit var keyResult: DevUi.Result
    private lateinit var testResult: DevUi.Result

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = TITLE
        actionBar?.setDisplayHomeAsUpEnabled(true)
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

    override fun onOptionsItemSelected(item: android.view.MenuItem): Boolean =
        if (item.itemId == android.R.id.home) { finish(); true } else super.onOptionsItemSelected(item)

    private fun refresh() { for (r in refreshers) r() }

    private fun renderNotInBuild() = with(ui) {
        screen {
            card {
                toggleRow("Cloud speech-to-text", CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                toggleRow("Clean up text with OpenRouter", CloudText.NOT_IN_BUILD.cap(), checked = false, enabled = false)
                note("This build has no cloud client: nothing leaves the phone except to your own server.")
            }
        }
    }

    private fun render() = with(ui) {
        val s0 = app.dev.load()
        screen {
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
                refreshers += { val s = app.dev.load(); quietly { zdr.isChecked = s.cloud.privacy.zdr; deny.isChecked = s.cloud.privacy.denyDataCollection } }
            }

            sttCard(s0)
            cleanupCard(s0)

            sectionLabel("USAGE")
            card {
                val usage = infoRow("This month (this phone only, never sent anywhere)")
                val budget = navRow(R.drawable.ic_dev_cloud, "Monthly budget") { editBudget() }
                chipRow(chip("Reset counters") { onResetUsage() }.apply { contentDescription = "Reset cloud usage counters" })
                refreshers += {
                    val s = app.dev.load()
                    usage.text = CloudText.usageLines(CloudUsage.decode(app.engine.router()?.cloudUsageJson() ?: app.dev.cloudUsageJson), s.cloud).joinToString("\n")
                    budget.setSubtitle(s.cloud.monthlyBudgetUsd?.let { "${CloudText.usd(it)} per month for speech and cleanup together; both pause once it is reached" } ?: "None")
                }
            }
            if (app.dev.visible) note("The cloud's place among your server and the phone: Developer settings → Routing & engines.")
        }
        refreshers += { keyLine.text = app.cloudKeys.masked(OpenRouter.ID) }
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
            refreshers += {
                val s = app.dev.load()
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
            val on = toggleRow("Clean up text with OpenRouter", "Sends the dictated text (not your voice) after each dictation; off by default", s0.allowCloudCleanup)
            val model = navRow(R.drawable.ic_set_sparkle, "Model") { ModelPickerActivity.open(this@OpenRouterActivity, CloudCatalog.Use.CLEANUP) }
            val order = if (caps.local) choiceRows(
                listOf(
                    DevUi.Choice(true, "OpenRouter first", "The phone's AI is the backup when OpenRouter is unreachable"),
                    DevUi.Choice(false, "Phone AI first", "OpenRouter only when no AI model is loaded on the phone"),
                ),
                OpenRouterPage.cleanupCloudFirst(s0),
            ) { first -> change { OpenRouterPage.withCleanupOrder(it, first) } } else null
            val sort = navRow(R.drawable.ic_dev_route, "Provider choice") { editSort() }
            val timeout = navRow(R.drawable.ic_set_timer, "Time limit") { editTimeout() }
            val consent = infoRow("Consent to send text")
            val status = infoRow("Status")
            on.onChange { v -> if (!quiet) onCleanup(on, v) }
            refreshers += {
                val s = app.dev.load()
                quietly { on.isChecked = s.allowCloudCleanup; order?.select(OpenRouterPage.cleanupCloudFirst(s)) }
                model.setSubtitle(CloudCatalog.modelSummary(s.cloud.cleanup.model))
                sort.setSubtitle(CloudInput.sortName(s.cloud.cleanup.sort))
                timeout.setSubtitle("${s.cloud.cleanup.timeoutMs} ms, then the text is pasted without it")
                consent.text = OpenRouterPage.consentLine(s.cloud.cleanup.consentAtMs, s.cloud.cleanup.consentOk, "cleanup", ::date)
                status.text = app.engine.router()?.let { DevText.cleanupStatus(it.cleanupStatus()) } ?: "Speech engine not loaded"
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
            refreshers += {
                val c = app.dev.load().cloud.cleanup
                quietly { around.isChecked = c.sendSurroundingText; screen.isChecked = c.sendScreenWords; appName.isChecked = c.sendAppName }
                always.text = "Sent with each cleanup: ${CloudText.cleanupSends(c)}."
            }
        }
        note("Cleanup is used for on-device and cloud dictations. Your server uses its own AI. About half of dictations need no AI and send nothing.")
    }

    // ---- changes ----

    private fun quietly(f: () -> Unit) { quiet = true; try { f() } finally { quiet = false } }

    /** Store + live router, then redraw. */
    private fun change(f: (DevSettings) -> DevSettings) {
        val s = f(app.dev.load())
        app.dev.saveCloud(
            allowCloud = s.allowCloud, cloud = s.cloud, routeOrder = s.routeOrder,
            allowCloudCleanup = s.allowCloudCleanup, cleanupOrder = s.cleanupOrder,
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
        val enable = { change { OpenRouterPage.enableCleanup(it, System.currentTimeMillis(), caps) }; refreshCatalog(); warnNoKey() }
        if (!OpenRouterPage.cleanupNeedsConsent(s)) { enable(); return }
        ui.confirm(CloudText.CLEANUP_CONSENT_TITLE, CloudText.cleanupConsentBody(s.cloud.cleanup), CloudText.CLEANUP_CONSENT_BUTTON, onYes = enable,
            onNo = { quietly { row.isChecked = false } })
    }

    private fun warnNoKey() {
        if (!app.cloudKeys.has(OpenRouter.ID)) keyResult.show("Add your OpenRouter API key above: nothing is sent without one.", Tone.WARN)
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
            val k = runCatching { r.cloudKeyInfo() }.getOrElse { dev.flowclone.core.CloudKeyInfo(error = "error") }
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
            val h = app.engine.router()?.probeCloud(json, key) ?: NativeFlowCore.probeCloud(json, key)
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
            runCatching { NativeFlowCore.setCloudCacheDir(cacheDir.path) }
            val st = runCatching { app.engine.router()?.refreshCloudCatalog(false) ?: NativeFlowCore.refreshCloudCatalog(false) }.getOrNull()
            st?.let { app.debugLog.event("cloud_catalog_refresh", it) }
        }
    }

    private fun date(ms: Long) = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(ms))
    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    companion object {
        const val TITLE = "OpenRouter (cloud)"
        fun open(ctx: Context) = ctx.startActivity(Intent(ctx, OpenRouterActivity::class.java))
        private fun String.cap() = replaceFirstChar { it.uppercase() }
    }
}
