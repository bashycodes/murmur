package dev.flowclone.ui

import android.app.Activity
import android.os.Bundle
import android.text.InputType
import android.widget.ArrayAdapter
import android.widget.CheckBox
import android.widget.EditText
import android.widget.Spinner
import android.widget.Toast
import dev.flowclone.FlowApp
import dev.flowclone.core.CleanupLevel
import dev.flowclone.data.Retention

class SettingsActivity : Activity() {
    private val app get() = FlowApp.of(this)

    private val languages = listOf(
        dev.flowclone.data.Settings.LANGUAGE_UNSET to "Phone language (${java.util.Locale.getDefault().getDisplayLanguage(java.util.Locale.ENGLISH)}) if the model supports it",
        "auto" to "Auto-detect", "en" to "English", "es" to "Spanish", "fr" to "French", "de" to "German",
        "it" to "Italian", "pt" to "Portuguese", "nl" to "Dutch", "he" to "Hebrew", "ar" to "Arabic",
        "ru" to "Russian", "uk" to "Ukrainian", "pl" to "Polish", "tr" to "Turkish", "hi" to "Hindi",
        "zh" to "Chinese", "ja" to "Japanese", "ko" to "Korean",
    )
    private val levels = listOf(CleanupLevel.LIGHT to "Light (punctuation, fillers)", CleanupLevel.MEDIUM to "Medium", CleanupLevel.HIGH to "High (rewrite for clarity)")
    private val retentions = listOf(Retention.KEEP to "Keep", Retention.DAY to "24 hours", Retention.NEVER to "Never save")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val s = app.settings.load()
        lateinit var lang: Spinner
        var langChoices = languages
        lateinit var cleanup: Spinner
        lateinit var retention: Spinner
        lateinit var cap: EditText
        lateinit var endpoint: EditText
        lateinit var mock: CheckBox
        lateinit var deny: EditText
        lateinit var llmOn: CheckBox
        lateinit var preview: CheckBox
        lateinit var shake: CheckBox
        lateinit var shrink: CheckBox
        lateinit var spoken: CheckBox
        lateinit var decodeEarly: CheckBox
        var parakeet: CheckBox? = null
        val local = app.caps.local
        page {
            heading("Language")
            // A stored code not in the short list (set elsewhere) is shown as an extra entry and kept.
            val langs = if (languages.any { it.first == s.language }) languages
            else languages + (s.language to dev.flowclone.data.LanguageDefault.name(s.language))
            lang = spinner(langs.map { it.second }, langs.indexOfFirst { it.first == s.language }.coerceAtLeast(0))
            langChoices = langs
            spoken = CheckBox(context).apply {
                text = "Auto-format punctuation, fillers and corrections (\"comma\", \"um\", \"scratch that\", \"five dollars\")"
                isChecked = s.spokenCleanup
            }.also(::addView)
            if (local) {
                heading("On-device speech engine")
                parakeet = CheckBox(context).apply {
                    text = "Use Parakeet (English only, more accurate; needs the imported model folder and ~1 GB RAM). Falls back to whisper if it can't load."
                    isChecked = s.asrEngine == dev.flowclone.core.AsrEngine.PARAKEET
                }.also(::addView)
                val pk = app.settings.parakeetPath?.let { java.io.File(it).name }
                body("Parakeet model: ${pk ?: "none imported"}", if (pk != null) OK_COLOR else null)
                button("Parakeet model folder…") { startActivity(ModelImportActivity.parakeetIntent(this@SettingsActivity)) }
            }
            if (local) heading("AI cleanup (local LLM)")
            if (local) body(
                "Off by default. When on, a small language model on this phone rewrites each transcript " +
                    "(punctuation, filler words, \"scratch that\", lists). Needs an imported GGUF model and adds 1–3 s; " +
                    "if it takes longer than 3 s, or changes too much, the plain transcript is inserted. " +
                    "Dictionary replacements and snippets apply either way.",
            )
            llmOn = CheckBox(context).apply { text = "Clean up with local AI"; isChecked = s.llmEnabled && local }
            cleanup = spinner(levels.map { it.second }, levels.indexOfFirst { it.first == s.cleanup }.coerceAtLeast(0)).apply { if (!local) visibility = android.view.View.GONE }
            if (local) {
                removeView(cleanup); addView(llmOn)
                val llmModel = app.settings.llmModelPath?.let { java.io.File(it).name }
                body("Model: ${llmModel ?: "none imported"}", if (llmModel != null) OK_COLOR else null)
                button("AI cleanup model…") { startActivity(ModelImportActivity.llmIntent(this@SettingsActivity)) }
                body("Strength")
                addView(cleanup)
            }
            heading("Bubble")
            preview = CheckBox(context).apply { text = "Live preview of the transcript while you speak (uses more battery)"; isChecked = s.livePreview }.also(::addView)
            shrink = CheckBox(context).apply { text = "Shrink the bubble to a dot after 5 s"; isChecked = s.shrinkBubble }.also(::addView)
            shake = CheckBox(context).apply { text = "Shake the phone to bring a snoozed or hidden bubble back"; isChecked = s.shakeToReveal }.also(::addView)
            body("Drag the bubble onto the target at the bottom of the screen to snooze it for 10 minutes.")
            heading("Hands-free: stop after silence (ms, 300–10000; 0 = only when you tap)")
            body("Tap-to-talk ends by itself after this much silence and inserts the text. Hold-to-talk is not affected.")
            endpoint = EditText(context).apply { inputType = InputType.TYPE_CLASS_NUMBER; setText(s.endpointSilenceMs.toString()) }.also(::addView)
            heading("Speed")
            decodeEarly = CheckBox(context).apply {
                text = "Decode while speaking (long dictations appear sooner after you stop; on-device and own server, not cloud)"
                isChecked = s.decodeWhileSpeaking
            }.also(::addView)
            heading("Session limit (minutes, 1–20)")
            cap = EditText(context).apply { inputType = InputType.TYPE_CLASS_NUMBER; setText(s.capMinutes.toString()) }.also(::addView)
            heading("History retention")
            retention = spinner(retentions.map { it.second }, retentions.indexOfFirst { it.first == s.retention }.coerceAtLeast(0))
            heading("Excluded apps")
            body("Package names, one per line. The bubble stays hidden in these (in addition to the built-in banking list).")
            deny = EditText(context).apply {
                inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE
                minLines = 3; setText(s.userDenylist.sorted().joinToString("\n"))
            }.also(::addView)
            heading("Developer")
            if (app.dev.visible) button("Developer settings (engines, server)…") { startActivity(android.content.Intent(this@SettingsActivity, DeveloperActivity::class.java)) }
            mock = CheckBox(context).apply { text = "Mock engine (inserts a fixed sentence; for testing without a model)"; isChecked = s.mockEngine }.also(::addView)
            button("Save") {
                val n = s.copy(
                    language = langChoices[lang.selectedItemPosition].first,
                    cleanup = levels[cleanup.selectedItemPosition].first,
                    llmEnabled = llmOn.isChecked,
                    livePreview = preview.isChecked,
                    shrinkBubble = shrink.isChecked,
                    shakeToReveal = shake.isChecked,
                    capMinutes = cap.text.toString().toIntOrNull()?.coerceIn(1, 20) ?: 5,
                    endpointSilenceMs = dev.flowclone.data.Settings.parseEndpointMs(endpoint.text.toString()),
                    retention = retentions[retention.selectedItemPosition].first,
                    userDenylist = deny.text.lines().map { it.trim() }.filter { it.isNotEmpty() }.toSet(),
                    mockEngine = mock.isChecked,
                    spokenCleanup = spoken.isChecked,
                    decodeWhileSpeaking = decodeEarly.isChecked,
                    asrEngine = if (parakeet?.isChecked == true) dev.flowclone.core.AsrEngine.PARAKEET else dev.flowclone.core.AsrEngine.WHISPER,
                )
                app.settings.save(n)
                // Review P3: database work off the main thread (the engine reload is non-blocking since C1).
                if (n.retention != s.retention) kotlin.concurrent.thread(name = "history-purge") { runCatching { app.history.purge(n.retention) } }
                if (n.mockEngine != s.mockEngine || n.llmEnabled != s.llmEnabled || n.asrEngine != s.asrEngine) app.engine.reload()
                if (n.llmEnabled && app.settings.llmModelPath == null) {
                    Toast.makeText(this@SettingsActivity, "AI cleanup needs a GGUF model: import one first.", Toast.LENGTH_LONG).show()
                }
                Toast.makeText(this@SettingsActivity, "Saved", Toast.LENGTH_SHORT).show()
                finish()
            }
            // §5 Access: in builds without `remote`, 7 taps here reveal Developer settings.
            val unlock = dev.flowclone.data.DevUnlock({ android.os.SystemClock.elapsedRealtime() })
            body("Version ${dev.flowclone.BuildConfig.VERSION_NAME} (${app.caps.flavor})").apply {
                setPadding(0, dp(24), 0, dp(8))
                setOnClickListener {
                    if (app.dev.visible) return@setOnClickListener
                    if (unlock.tap()) {
                        app.dev.devUnlocked = true
                        Toast.makeText(this@SettingsActivity, "Developer settings unlocked", Toast.LENGTH_SHORT).show()
                        recreate()
                    } else if (unlock.remaining <= 4) {
                        Toast.makeText(this@SettingsActivity, "${unlock.remaining} more taps", Toast.LENGTH_SHORT).show()
                    }
                }
            }
        }
    }

    private fun android.widget.LinearLayout.spinner(items: List<String>, selected: Int) = Spinner(context).apply {
        adapter = ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, items)
        setSelection(selected)
    }.also(::addView)
}
