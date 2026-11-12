package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.TextView
import android.widget.Toast
import dev.murmur.MurmurApp
import dev.murmur.R
import dev.murmur.core.NativeMurmurCore
import dev.murmur.data.CloudCatalog
import dev.murmur.data.CloudCatalog.Use
import dev.murmur.data.DevSettings
import dev.murmur.data.OpenRouter
import dev.murmur.data.CloudPage
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

/**
 * D28 model picker (spec §2.2): every OpenRouter model that can do the job, from the Rust
 * catalogue (`nativeCloudCatalog`: memory, 24 h cache or the baked list, never the network).
 * Opening the page refreshes it in the background (`nativeRefreshCloudCatalog`, cache-first);
 * "Refresh" forces it. Search by id / name, filter chips, a price per hour of audio (speech) or
 * per dictation (cleanup), badges, and any model id typed by hand.
 */
class ModelPickerActivity : Activity() {
    private val app get() = MurmurApp.of(this)
    private lateinit var ui: DevUi
    private lateinit var use: Use
    private var query = ""
    private var filters = CloudCatalog.Filters()
    private var status = CloudCatalog.Status()
    private lateinit var listBox: LinearLayout
    private lateinit var statusLine: TextView
    private lateinit var current: TextView
    private val chips = ArrayList<Pair<Button, () -> Boolean>>()
    @Volatile private var refreshing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        use = intent.getStringExtra(EXTRA_USE)?.let { w -> Use.entries.firstOrNull { it.wire == w } } ?: Use.STT
        title = if (use == Use.STT) "Speech-to-text model" else "Cleanup model"
        ui = DevUi(this)
        if (!app.caps.cloud) { finish(); return }
        runCatching { NativeMurmurCore.setCloudCacheDir(cacheDir.path) }
        render()
        reload()
        refresh(force = false)
    }

    private fun render() = with(ui) {
        screen {
            topBar(title.toString(), DevUi.TopNav.BACK, onNav = ::finish)
            card {
                current = infoRow("In use")
                statusLine = infoRow("Model list")
                chipRow(chip("Refresh") { refresh(force = true) }.apply { contentDescription = "Refresh the model list from OpenRouter" })
            }
            searchField("Search models by name or id") { q -> query = q.trim(); reload() }
            sectionLabel("FILTERS")
            card {
                val rows = ArrayList<Button>()
                if (use == Use.STT) {
                    rows += toggleChip("Transcription", { filters.sttPath == "transcriptions" }) {
                        filters = filters.copy(sttPath = if (filters.sttPath == "transcriptions") null else "transcriptions")
                    }
                    rows += toggleChip("Chat audio", { filters.sttPath == "chat_audio" }) {
                        filters = filters.copy(sttPath = if (filters.sttPath == "chat_audio") null else "chat_audio")
                    }
                } else {
                    rows += toggleChip("Text", { true }) { }
                }
                chipRow(*rows.toTypedArray())
                chipRow(
                    toggleChip("Zero-retention only", { filters.zdrOnly }) { filters = filters.copy(zdrOnly = !filters.zdrOnly) },
                    toggleChip("Fast providers", { filters.fastOnly }) { filters = filters.copy(fastOnly = !filters.fastOnly) },
                    toggleChip("Free", { filters.freeOnly }) { filters = filters.copy(freeOnly = !filters.freeOnly) },
                )
            }
            sectionLabel("MODELS")
            card {
                navRow(R.drawable.ic_dev_add, "Use custom model id…", "Any OpenRouter model id, e.g. a :nitro variant", chevron = false) { customId() }
                listBox = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }.also(::addView)
            }
            note(
                if (use == Use.STT) "Prices are per hour of audio, as OpenRouter lists them; \"≈\" marks a chat model's estimate (32 audio tokens a second, plus its output). " +
                    "Your real cost per dictation is in Usage."
                else "Prices are an estimate per dictation (about 1,000 tokens in, a short answer out). Your real cost is in Usage.",
            )
        }
        syncChips()
    }

    /** A filter chip that reads its state from [on]; TalkBack hears "selected" / "not selected". */
    private fun toggleChip(label: String, on: () -> Boolean, flip: () -> Unit): Button =
        ui.chip(label) { flip(); syncChips(); reload() }.also { chips += it to on }

    private fun syncChips() {
        for ((b, on) in chips) {
            val sel = on()
            val label = b.text.toString().removePrefix("✓ ")
            b.text = if (sel) "✓ $label" else label
            b.isSelected = sel
            b.stateDescription = if (sel) "selected" else "not selected"
            b.contentDescription = "Filter $label"
        }
    }

    private fun stored(): DevSettings = app.dev.load()

    /** Re-reads the rows (no network) and redraws the list. */
    private fun reload() {
        val rows = CloudCatalog.decodeRows(NativeMurmurCore.cloudCatalog(use.wire, query, filters.toJson()))
        val inUse = CloudPage.model(stored(), use)
        current.text = CloudCatalog.modelSummary(inUse, if (use == Use.STT) stored().cloud.stt.sttPath else null)
        statusLine.text = CloudCatalog.sourceLine(status, System.currentTimeMillis()) { DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(it)) } +
            if (refreshing) " · refreshing…" else ""
        listBox.removeAllViews()
        with(ui) {
            if (rows.isEmpty()) {
                listBox.emptyState("No model matches", if (filters.zdrOnly) "Try another search, or turn off \"Zero-retention only\"." else "Try another search or fewer filters.")
                return
            }
            for (r in rows.take(MAX_ROWS)) {
                val title = r.name + if (r.recommended) "  ★" else ""
                val row = listBox.listRow(
                    title, CloudCatalog.meta(r, use), badge = if (r.id == inUse) "In use" else "",
                    flag = CloudCatalog.badgeLine(r).ifEmpty { null }, flagTone = if (r.chatAudio || r.allowedByAccount == false) Tone.WARN else Tone.OK,
                ) { pick(r) }
                row.view.contentDescription = listOfNotNull(
                    r.name, if (r.recommended) "recommended" else null, r.id, r.priceLabel.ifEmpty { null },
                    CloudCatalog.badgeLine(r).ifEmpty { null }, if (r.id == inUse) "in use" else null,
                ).joinToString(", ")
            }
            if (rows.size > MAX_ROWS) listBox.note("${rows.size - MAX_ROWS} more: refine the search.")
        }
    }

    private fun refresh(force: Boolean) {
        if (refreshing) return
        refreshing = true
        reload()
        thread(name = "or-catalog") {
            val json = runCatching { app.engine.router()?.refreshCloudCatalog(force) ?: NativeMurmurCore.refreshCloudCatalog(force) }.getOrNull()
            json?.let { app.debugLog.event("cloud_catalog_refresh", it) }
            runOnUiThread {
                refreshing = false
                status = CloudCatalog.decodeStatus(json)
                if (!isFinishing) reload()
            }
        }
    }

    private fun pick(r: CloudCatalog.Row) {
        val save = { save(r.id, "auto") }
        if (use == Use.STT && r.chatAudio) {
            ui.confirm("Use a chat model for speech-to-text?", CloudCatalog.CHAT_AUDIO_WARNING, "Use it", onYes = save)
            return
        }
        save()
    }

    private fun save(id: String, sttPath: String) {
        val m = id.trim()
        OpenRouter.modelIdError(m)?.let { Toast.makeText(this, it, Toast.LENGTH_LONG).show(); return }
        val next = CloudPage.withModel(stored(), use, m, sttPath)
        app.dev.saveCloud(cloud = next.cloud)
        app.engine.applyDevSettings(app.dev.load())
        // The router checks the id too (same rule); a refusal there is logged, the stored choice stays.
        app.engine.router()?.setCloudModel(use.wire, m)?.takeIf { !it }?.let { app.debugLog.event("cloud_model", "router refused the model id") }
        Toast.makeText(this, "Model set: $m", Toast.LENGTH_SHORT).show()
        reload()
    }

    /** Free-text id (spec §2.2), validated as typed; for speech, the user may pick the API path. */
    private fun customId() {
        val box = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(8), dp(20), 0) }
        val edit = EditText(this).apply {
            hint = if (use == Use.STT) OpenRouter.DEFAULT_STT_MODEL else OpenRouter.DEFAULT_CLEANUP_MODEL
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            isSingleLine = true; minHeight = dp(48)
        }
        val err = TextView(this).apply { textSize = 13f; setTextColor(ui.p.danger); visibility = View.GONE }
        box.addView(edit); box.addView(err)
        val paths = listOf("auto" to "Automatic (from the model list)", "transcriptions" to "Transcription model", "chat_audio" to "Chat model with audio input (may paraphrase)")
        val group = RadioGroup(this).apply { orientation = RadioGroup.VERTICAL }
        if (use == Use.STT) {
            for ((i, p) in paths.withIndex()) group.addView(RadioButton(this).apply { id = 1000 + i; text = p.second; minHeight = dp(48); isChecked = i == 0 })
            box.addView(group)
        }
        edit.addTextChangedListener(object : android.text.TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
            override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
            override fun afterTextChanged(s: android.text.Editable?) {
                val e = OpenRouter.modelIdError(s?.toString().orEmpty())
                err.text = e.orEmpty(); err.visibility = if (e == null) View.GONE else View.VISIBLE
            }
        })
        val d = AlertDialog.Builder(this).setTitle("Custom model id").setView(box)
            .setNegativeButton("Cancel", null).setPositiveButton("Use", null).show()
        d.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val id = edit.text.toString().trim()
            val e = OpenRouter.modelIdError(id)
            if (e != null) { err.text = e; err.visibility = View.VISIBLE; return@setOnClickListener }
            val path = if (use == Use.STT) paths.getOrNull(group.checkedRadioButtonId - 1000)?.first ?: "auto" else "auto"
            save(id, path); d.dismiss()
        }
    }

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    companion object {
        const val EXTRA_USE = "use"
        private const val MAX_ROWS = 150

        fun open(ctx: Context, use: Use) = ctx.startActivity(Intent(ctx, ModelPickerActivity::class.java).putExtra(EXTRA_USE, use.wire))
    }
}
