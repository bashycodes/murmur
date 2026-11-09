package dev.flowclone.ui.tabs

import android.app.Activity
import android.text.TextUtils
import android.view.Gravity
import android.view.ViewGroup
import android.widget.LinearLayout
import dev.flowclone.core.DictEntry
import dev.flowclone.data.DictionaryPages
import dev.flowclone.data.DictionaryPages.Kind
import dev.flowclone.learn.Suggestion
import dev.flowclone.learn.Suggestions
import dev.flowclone.ui.DevUi
import dev.flowclone.ui.SuggestionDialog
import dev.flowclone.ui.TypeScale

/**
 * Dictionary tab (AndroidDictionary.dc.html): one card of words, "heard →" in grey before a word
 * that replaces what the engine hears. Above it, while any wait, the learn-from-my-edits
 * suggestions (Add / Dismiss; nothing is added without Add). No "learned" marker: entries carry
 * no such flag.
 */
internal class DictionaryTab(activity: Activity, ui: DevUi) : EntryTab(activity, ui, Kind.WORDS) {
    override fun above(col: LinearLayout): Unit = with(ui) {
        val pending = app.suggestions.update { it.pruneKnown(app.dictionary.all()) }.pending
        if (pending.isEmpty()) return@with
        col.sectionLabel("Suggestions")
        col.card {
            for (s in pending) {
                val buttons = LinearLayout(activity).apply {
                    orientation = LinearLayout.HORIZONTAL
                    addView(chip("Add") { addSuggestion(s.word, false) })
                    addView(chip("Dismiss") { dismissSuggestion(s.word) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginStart = dp(8) })
                }
                listRow(
                    s.word, Suggestions.subtitle(s), trailing = buttons,
                    actions = listOf("Add" to { addSuggestion(s.word, false) }, "Dismiss" to { dismissSuggestion(s.word) }),
                ) { suggestionDialog(s) }
            }
        }
        col.sectionLabel("Words")
    }

    override fun rows(col: LinearLayout, entries: List<DictEntry>): Unit = with(ui) {
        col.card {
            for (e in entries) {
                val r = DictionaryPages.row(e)
                addView(LinearLayout(activity).apply {
                    orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                    setPadding(dp(20), dp(8), dp(20), dp(8))
                    background = ripple(null)
                    if (r.heard != null) addView(tv("${r.heard} →", TypeScale.ROW, t.ink2).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.MIDDLE }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginEnd = dp(10) })
                    addView(tv(r.word, TypeScale.ROW, p.text).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
                    entry(e)
                })
            }
        }
    }

    private fun suggestionDialog(s: Suggestion) = SuggestionDialog.show(
        activity, s, "Add “${s.word}”?",
        "You changed “${s.heard}” to “${s.word}” after a dictation. Add the word to teach its spelling, " +
            "or add it as a fix so “${s.heard}” is always replaced. Dismiss = never suggested again.",
    ) { done(it) }

    private fun addSuggestion(word: String, asFix: Boolean) = done(SuggestionDialog.add(app, word, asFix))

    private fun dismissSuggestion(word: String) = done(SuggestionDialog.dismiss(app, word))

    private fun done(announcement: String) {
        ui.announce(announcement)
        fill()
    }
}
