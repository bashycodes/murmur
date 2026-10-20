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
        "auto" to "Auto-detect", "en" to "English", "es" to "Spanish", "fr" to "French", "de" to "German",
        "it" to "Italian", "pt" to "Portuguese", "nl" to "Dutch", "he" to "Hebrew", "ar" to "Arabic",
        "ru" to "Russian", "uk" to "Ukrainian", "pl" to "Polish", "tr" to "Turkish", "hi" to "Hindi",
        "zh" to "Chinese", "ja" to "Japanese", "ko" to "Korean",
    )
    private val retentions = listOf(Retention.KEEP to "Keep", Retention.DAY to "24 hours", Retention.NEVER to "Never save")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val s = app.settings.load()
        lateinit var lang: Spinner
        lateinit var cleanup: Spinner
        lateinit var retention: Spinner
        lateinit var cap: EditText
        lateinit var mock: CheckBox
        lateinit var deny: EditText
        page {
            heading("Language")
            lang = spinner(languages.map { it.second }, languages.indexOfFirst { it.first == s.language }.coerceAtLeast(0))
            heading("AI cleanup")
            body("Local LLM cleanup isn't bundled yet; \"none\" = deterministic post-processing only.")
            cleanup = spinner(CleanupLevel.entries.map { it.wire }, s.cleanup.ordinal)
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
            mock = CheckBox(context).apply { text = "Mock engine (inserts a fixed sentence; for testing without a model)"; isChecked = s.mockEngine }.also(::addView)
            button("Save") {
                val n = s.copy(
                    language = languages[lang.selectedItemPosition].first,
                    cleanup = CleanupLevel.entries[cleanup.selectedItemPosition],
                    capMinutes = cap.text.toString().toIntOrNull()?.coerceIn(1, 20) ?: 5,
                    retention = retentions[retention.selectedItemPosition].first,
                    userDenylist = deny.text.lines().map { it.trim() }.filter { it.isNotEmpty() }.toSet(),
                    mockEngine = mock.isChecked,
                )
                app.settings.save(n)
                if (n.retention != s.retention) app.history.purge(n.retention)
                if (n.mockEngine != s.mockEngine) app.engine.reload()
                Toast.makeText(this@SettingsActivity, "Saved", Toast.LENGTH_SHORT).show()
                finish()
            }
        }
    }

    private fun android.widget.LinearLayout.spinner(items: List<String>, selected: Int) = Spinner(context).apply {
        adapter = ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, items)
        setSelection(selected)
    }.also(::addView)
}
