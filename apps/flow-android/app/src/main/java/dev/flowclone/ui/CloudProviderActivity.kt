package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
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
 * zero-retention toggles, consent naming the host, Test provider, delete. Saves go straight to
 * the store and the live router; the Developer page's toggles decide whether the cloud is used.
 */
class CloudProviderActivity : Activity() {
    private val app get() = FlowApp.of(this)

    private lateinit var base: CloudProvider
    private var isNew = false
    private lateinit var label: EditText
    private lateinit var baseUrl: EditText
    private lateinit var model: EditText
    private lateinit var language: EditText
    private lateinit var timeout: EditText
    private lateinit var key: EditText
    private lateinit var keyLine: TextView
    private lateinit var sendDict: CheckBox
    private lateinit var sendScreen: CheckBox
    private lateinit var zdr: CheckBox
    private lateinit var error: TextView
    private lateinit var consentLine: TextView
    private lateinit var testLine: TextView
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
        render()
    }

    private fun render() {
        val preset = CloudPresets.of(base.preset)
        page {
            body("Provider: ${preset.label} · adapter ${base.adapter} · id ${base.id}")
            body(CloudText.banner(base))
            label = field("Name shown in the app", base.label)
            baseUrl = field("Base URL (https://…; http:// only for LAN / Tailscale hosts)", base.baseUrl, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
            baseUrl.isEnabled = preset.editableBaseUrl || base.adapter == "openai_compat"
            model = field("Model", base.model)
            language = field("Language (empty = follow your dictation languages; e.g. en, de)", base.language)
            timeout = field("Timeout base (ms, 1000-60000)", base.timeoutBaseMs.toString(), InputType.TYPE_CLASS_NUMBER)

            heading("API key")
            keyLine = body("")
            key = field("Paste a new key to replace the stored one (${preset.keyHint})", "", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
            row(
                Button(context).apply { text = "Show"; isAllCaps = false; setOnClickListener { toggleKeyVisible(this) } },
                Button(context).apply { text = "Paste"; isAllCaps = false; setOnClickListener { pasteKey() } },
                Button(context).apply { text = "Clear stored key"; isAllCaps = false; setOnClickListener { clearKey() } },
            )

            heading("What is sent")
            sendDict = check("Send dictionary words (improves spelling of names)", base.sendDictionary)
            sendScreen = check("Send on-screen words (text around the cursor; off by default)", base.sendScreenWords)
            zdr = check(if (preset.zeroRetention == null) "Zero retention — not a request option for this provider" else "Zero retention", base.zeroRetention && preset.zeroRetention != null)
            zdr.isEnabled = preset.zeroRetention != null
            body(preset.retention)

            heading("Consent")
            consentLine = body("")
            button("Give consent to send audio to this host…") { askConsent() }

            error = body("", BAD_COLOR).apply { visibility = View.GONE }
            save = button(if (isNew) "Add provider" else "Save") { onSave() }
            button("Test provider (uploads a 3 s public-domain clip with these settings)") { onTest() }
            testLine = body("")
            if (!isNew) {
                button("Use this provider for the cloud route") { setActive() }
                button("Delete provider (and its key)") { confirmDelete() }
            }
        }
        refresh()
    }

    private fun LinearLayout.field(labelText: String, value: String, type: Int = InputType.TYPE_CLASS_TEXT): EditText {
        body(labelText)
        return EditText(context).apply {
            inputType = type; setText(value); isSingleLine = true
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun afterTextChanged(s: Editable?) { if (::save.isInitialized) validate() }
            })
        }.also(::addView)
    }

    private fun LinearLayout.check(text: String, checked: Boolean) = CheckBox(context).apply {
        this.text = text; isChecked = checked
    }.also(::addView)

    private fun input() = CloudProviderForm.Input(
        base, label.text.toString(), baseUrl.text.toString(), model.text.toString(), language.text.toString(),
        sendDict.isChecked, sendScreen.isChecked, zdr.isChecked, timeout.text.toString(),
    )

    private fun validate(): CloudProviderForm.Result {
        val r = CloudProviderForm.validate(input())
        error.text = r.error ?: (if (r.consentVoided) "The host or adapter changed: consent will be cleared on save." else "")
        error.visibility = if (error.text.isEmpty()) View.GONE else View.VISIBLE
        save.isEnabled = r.provider != null
        r.provider?.let { p ->
            consentLine.text = if (p.consentOk) "Given for ${p.consentHost} on ${DateFormat.getDateTimeInstance().format(Date(p.consentAtMs))}"
            else "Not given for ${p.host.ifEmpty { "this host" }}: the cloud route skips this provider (cloud_not_configured)."
        }
        return r
    }

    private fun refresh() {
        keyLine.text = "Stored key: " + app.cloudKeys.masked(base.id)
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
        val p = validate().provider ?: run { testLine.text = "Fix the highlighted field first."; return }
        val saved = if (isNew) null else app.dev.load().cloud.provider(base.id)
        // S1: the stored key only goes to the saved host + adapter; an edited form needs the typed key.
        val k = when (val t = CloudKeyPolicy.testKey(saved, p, key.text.toString(), app.cloudKeys.get(p.id))) {
            is CloudKeyPolicy.TestKey.Missing -> { testLine.text = t.message; return }
            is CloudKeyPolicy.TestKey.Use -> t.key
            CloudKeyPolicy.TestKey.Keyless -> null // the router sends no Authorization header
        }
        testLine.text = "Testing ${p.displayName} (${p.host})…"
        testLine.setTextColor(currentTextColor())
        val json = p.toJson().toString()
        thread(name = "cloud-probe") {
            // Through the live router when there is one (a success for the active provider closes its
            // cloud circuit), else a temporary native router. The key goes in explicitly either way.
            val h = app.engine.router()?.probeCloud(json, k) ?: NativeFlowCore.probeCloud(json, k)
            runOnUiThread {
                testLine.text = DevText.cloudProbe(h)
                testLine.setTextColor(if (h.ok) OK_COLOR else BAD_COLOR)
            }
        }
    }

    private fun currentTextColor() = keyLine.currentTextColor

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

    private fun toggleKeyVisible(b: Button) {
        val hidden = key.inputType and InputType.TYPE_TEXT_VARIATION_PASSWORD == InputType.TYPE_TEXT_VARIATION_PASSWORD
        key.inputType = InputType.TYPE_CLASS_TEXT or if (hidden) InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD else InputType.TYPE_TEXT_VARIATION_PASSWORD
        b.text = if (hidden) "Hide" else "Show"
    }

    private fun pasteKey() {
        val cm = getSystemService(android.content.ClipboardManager::class.java)
        val t = cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(this)?.toString()?.trim()
        if (t.isNullOrEmpty()) Toast.makeText(this, "Clipboard is empty", Toast.LENGTH_SHORT).show() else key.setText(t)
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
