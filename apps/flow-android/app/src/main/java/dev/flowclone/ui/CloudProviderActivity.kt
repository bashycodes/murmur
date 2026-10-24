package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.R
import dev.flowclone.core.DevText
import dev.flowclone.core.NativeFlowCore
import dev.flowclone.data.CloudPresets
import dev.flowclone.data.CloudProvider
import dev.flowclone.data.CloudProviderForm
import dev.flowclone.data.CloudKeyPolicy
import dev.flowclone.data.CloudRules
import dev.flowclone.data.CloudText
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * D18 (docs/cloud-stt.md §8.3): add / edit one cloud STT provider: model, language, base URL
 * (custom only), API key (masked; stored only Keystore-sealed), dictionary / screen-word /
 * zero-retention toggles, consent naming the host, Test provider, delete, in the Settings look
 * (ui/DevUi; the key field is [DevUi.secretRow]: never in the a11y tree). Saves go straight to
 * the store and the live router; the Developer page's toggles decide whether the cloud is used.
 */
class CloudProviderActivity : Activity() {
    private val app get() = FlowApp.of(this)

    private lateinit var base: CloudProvider
    private var isNew = false
    private lateinit var ui: DevUi
    private lateinit var label: EditText
    private lateinit var baseUrl: EditText
    private lateinit var model: EditText
    private lateinit var language: EditText
    private lateinit var timeout: EditText
    private lateinit var key: EditText
    private lateinit var keyLine: TextView
    private lateinit var sendDict: DevUi.ToggleRow
    private lateinit var sendScreen: DevUi.ToggleRow
    private lateinit var zdr: DevUi.ToggleRow
    private lateinit var error: DevUi.Result
    private lateinit var consentLine: TextView
    private lateinit var testLine: DevUi.Result
    private lateinit var save: Button

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!app.caps.cloud) { finish(); return }
        val cloud = app.dev.load().cloud
        val id = intent.getStringExtra(EXTRA_ID)
        val existing = id?.let(cloud::provider)
        if (existing != null) {
            base = existing
        } else {
            if (cloud.providers.size >= CloudRules.MAX_PROVIDERS) {
                Toast.makeText(this, "At most ${CloudRules.MAX_PROVIDERS} providers", Toast.LENGTH_LONG).show(); finish(); return
            }
            base = CloudPresets.of(intent.getStringExtra(EXTRA_PRESET) ?: "custom").newProvider(cloud.providers.map { it.id })
            isNew = true
        }
        title = if (isNew) "Add cloud provider" else base.displayName
        ui = DevUi(this)
        actionBar?.setDisplayHomeAsUpEnabled(true)
        render()
    }

    private fun render(): Unit = with(ui) {
        val preset = CloudPresets.of(base.preset)
        screen {
            note(CloudText.banner(base))
            error = banner()
            sectionLabel("PROVIDER")
            card {
                infoRow("Provider", "${preset.label} · adapter ${base.adapter} · id ${base.id}")
                label = field("Name shown in the app", base.label)
                baseUrl = field("Base URL", base.baseUrl, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI, "https://…; http:// only for LAN / Tailscale hosts")
                baseUrl.isEnabled = preset.editableBaseUrl || base.adapter == "openai_compat"
                model = field("Model", base.model)
                language = field("Language", base.language, help = "Empty = follow your dictation languages; e.g. en, de")
                timeout = field("Timeout base (ms)", base.timeoutBaseMs.toString(), InputType.TYPE_CLASS_NUMBER, "1000–60000")
            }

            sectionLabel("API KEY")
            card {
                keyLine = infoRow("Stored key")
                key = secretRow(
                    "New key", "", "Paste a new key to replace the stored one (${preset.keyHint}). Stored only Keystore-sealed.", "API key",
                    onEdit = { if (::save.isInitialized) validate() }, clearLabel = "Clear stored key", onClear = { clearKey() },
                ).edit
            }

            sectionLabel("WHAT IS SENT")
            card {
                sendDict = toggleRow("Send dictionary words", "Improves spelling of names", base.sendDictionary).also { it.onChange { validate() } }
                sendScreen = toggleRow("Send on-screen words", "Text around the cursor; off by default", base.sendScreenWords).also { it.onChange { validate() } }
                zdr = toggleRow(
                    "Zero retention",
                    if (preset.zeroRetention == null) "Not a request option for this provider" else "",
                    base.zeroRetention && preset.zeroRetention != null,
                    enabled = preset.zeroRetention != null,
                ).also { it.onChange { validate() } }
            }
            note(preset.retention)

            sectionLabel("CONSENT")
            card {
                consentLine = infoRow("Audio may be sent")
                navRow(R.drawable.ic_dev_lock, "Give consent to send audio to this host…") { askConsent() }
            }

            save = action(if (isNew) "Add provider" else "Save", DevUi.Style.PRIMARY) { onSave() }
            action("Test provider") { onTest() }
            note("Uploads a 3 s public-domain clip with these settings.")
            testLine = result()
            if (!isNew) {
                action("Use this provider for the cloud route") { setActive() }
                action("Delete provider (and its key)", DevUi.Style.DANGER) { confirmDelete() }
            }
        }
        refresh()
    }

    private fun LinearLayout.field(labelText: String, value: String, type: Int = InputType.TYPE_CLASS_TEXT, help: String = ""): EditText =
        with(ui) { fieldRow(labelText, value, type, help) { if (::save.isInitialized) validate() }.edit }

    override fun onOptionsItemSelected(item: android.view.MenuItem): Boolean =
        if (item.itemId == android.R.id.home) { finish(); true } else super.onOptionsItemSelected(item)

    private fun input() = CloudProviderForm.Input(
        base, label.text.toString(), baseUrl.text.toString(), model.text.toString(), language.text.toString(),
        sendDict.isChecked, sendScreen.isChecked, zdr.isChecked, timeout.text.toString(),
    )

    private fun validate(): CloudProviderForm.Result {
        val r = CloudProviderForm.validate(input())
        val msg = r.error ?: (if (r.consentVoided) "The host or adapter changed: consent will be cleared on save." else "")
        if (error.text.toString() != msg) error.show(msg, if (r.error != null) Tone.BAD else Tone.WARN)
        save.isEnabled = r.provider != null
        save.alpha = if (save.isEnabled) 1f else 0.5f
        r.provider?.let { p ->
            consentLine.text = if (p.consentOk) "Given for ${p.consentHost} on ${DateFormat.getDateTimeInstance().format(Date(p.consentAtMs))}"
            else "Not given for ${p.host.ifEmpty { "this host" }}: the cloud route skips this provider (cloud_not_configured)."
        }
        return r
    }

    private fun refresh() {
        keyLine.text = app.cloudKeys.masked(base.id)
        validate()
    }

    private fun persist(p: CloudProvider, toast: String) {
        val s = app.dev.load()
        // [p] already carries the right consent (form: CloudProvider.edited; dialog: withConsent).
        val cloud = (if (s.cloud.provider(p.id) == null) s.cloud.upsert(p) else s.cloud).replace(p)
        app.dev.saveCloud(cloud = cloud)
        app.engine.applyDevSettings(app.dev.load())
        base = cloud.provider(p.id) ?: p
        isNew = false
        Toast.makeText(this, toast, Toast.LENGTH_SHORT).show()
    }

    private fun onSave() {
        val r = validate()
        val p = r.provider ?: return
        val saved = if (isNew) null else app.dev.load().cloud.provider(base.id)
        // S4: a key is bound to the saved host + adapter; a changed one deletes it (never re-sent to the new host).
        val keys = CloudKeyPolicy.onSave(saved, p, key.text.toString(), app.cloudKeys.has(p.id))
        if (keys.deleteStored) {
            app.cloudKeys.remove(p.id)
            app.engine.setCloudKey(p.id, null)
        }
        persist(
            p,
            when {
                keys.askAgain -> "Saved. Host or adapter changed: the old key was deleted, enter the key for ${p.host}."
                r.consentVoided -> "Saved. Host changed: give consent again before it is used."
                else -> "Saved: applies to the next dictation"
            },
        )
        keys.set?.let { k ->
            app.cloudKeys.set(p.id, k)
            // After persist: the router now has this provider, so the key binds to its host + adapter.
            app.engine.setCloudKeyFor(base, k)
            key.setText("")
        }
        title = base.displayName
        refresh()
    }

    private fun askConsent() {
        val p = validate().provider ?: run { Toast.makeText(this, "Fix the highlighted field first", Toast.LENGTH_SHORT).show(); return }
        consentDialog(this, p) {
            persist(p.withConsent(System.currentTimeMillis()), "Consent recorded for ${p.host}")
            refresh()
        }
    }

    private fun onTest() {
        val p = validate().provider ?: run { testLine.show("Fix the highlighted field first.", Tone.BAD); return }
        val saved = if (isNew) null else app.dev.load().cloud.provider(base.id)
        // S1: the stored key only goes to the saved host + adapter; an edited form needs the typed key.
        val k = when (val t = CloudKeyPolicy.testKey(saved, p, key.text.toString(), app.cloudKeys.get(p.id))) {
            is CloudKeyPolicy.TestKey.Missing -> { testLine.show(t.message, Tone.BAD); return }
            is CloudKeyPolicy.TestKey.Use -> t.key
            CloudKeyPolicy.TestKey.Keyless -> null // the router sends no Authorization header
        }
        testLine.show("Testing ${p.displayName} (${p.host})…")
        val json = p.toJson().toString()
        thread(name = "cloud-probe") {
            // Through the live router when there is one (a success for the active provider closes its
            // cloud circuit), else a temporary native router. The key goes in explicitly either way.
            val h = app.engine.router()?.probeCloud(json, k) ?: NativeFlowCore.probeCloud(json, k)
            runOnUiThread {
                testLine.show(DevText.cloudProbe(h), if (h.ok) Tone.OK else Tone.BAD)
            }
        }
    }

    private fun clearKey() {
        app.cloudKeys.remove(base.id)
        app.engine.setCloudKey(base.id, null)
        key.setText("")
        refresh()
        Toast.makeText(this, "Key removed from this device", Toast.LENGTH_SHORT).show()
    }

    private fun setActive() {
        val s = app.dev.load()
        app.dev.saveCloud(cloud = s.cloud.copy(active = base.id))
        app.engine.applyDevSettings(app.dev.load())
        Toast.makeText(this, "${base.displayName} is now the cloud provider", Toast.LENGTH_SHORT).show()
    }

    private fun confirmDelete() {
        AlertDialog.Builder(this)
            .setTitle("Delete ${base.displayName}?")
            .setMessage("Removes the provider and its API key from this device.")
            .setNegativeButton("Cancel", null)
            .setPositiveButton("Delete") { _, _ ->
                val s = app.dev.load()
                app.cloudKeys.remove(base.id)
                app.engine.setCloudKey(base.id, null)
                app.dev.saveCloud(cloud = s.cloud.remove(base.id))
                app.engine.applyDevSettings(app.dev.load())
                finish()
            }.show()
    }

    companion object {
        const val EXTRA_ID = "provider_id"
        const val EXTRA_PRESET = "preset"

        fun edit(ctx: Context, id: String) = ctx.startActivity(Intent(ctx, CloudProviderActivity::class.java).putExtra(EXTRA_ID, id))
        fun add(ctx: Context, preset: String) = ctx.startActivity(Intent(ctx, CloudProviderActivity::class.java).putExtra(EXTRA_PRESET, preset))

        /** §8.3 consent dialog: names the provider and its host; only the explicit button records consent. */
        fun consentDialog(ctx: Context, p: CloudProvider, onCancel: () -> Unit = {}, onAccept: () -> Unit) {
            AlertDialog.Builder(ctx)
                .setTitle(CloudText.consentTitle(p))
                .setMessage(CloudText.consentBody(p))
                .setNegativeButton("Cancel") { _, _ -> onCancel() }
                .setOnCancelListener { onCancel() }
                .setPositiveButton(CloudText.consentButton(p)) { _, _ -> onAccept() }
                .show()
        }
    }
}
