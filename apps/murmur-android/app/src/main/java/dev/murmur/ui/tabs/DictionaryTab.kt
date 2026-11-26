package dev.murmur.ui.tabs

import android.app.Activity
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import dev.murmur.core.DictEntry
import dev.murmur.data.DictionaryPages
import dev.murmur.data.DictionaryPages.Kind
import dev.murmur.learn.Suggestion
import dev.murmur.learn.Suggestions
import dev.murmur.ui.DevUi
import dev.murmur.ui.SuggestionDialog
import dev.murmur.ui.TypeScale

/**
 * Dictionary tab (AndroidDictionary.dc.html): one card of words, "heard →" in grey before a word
 * that replaces what the engine hears. Above it, while any wait, the learn-from-my-edits
 * suggestions (Add / Dismiss; nothing is added without Add). No "learned" marker: entries carry
 * no such flag. Below it, with a server that has one, the team dictionary: read-only (admins
 * edit it in Users & keys); a personal entry wins on the same word.
 */
internal class DictionaryTab(activity: Activity, ui: DevUi) : EntryTab(activity, ui, Kind.WORDS) {
    /** A team refresh landed: redraw while the tab shows. */
    private val teamChanged: () -> Unit = { activity.runOnUiThread { if (!activity.isDestroyed) fill() } }

    override fun onShow() {
        app.team.addListener(teamChanged)
        super.onShow()
    }

    override fun onHide() {
        app.team.removeListener(teamChanged)
        super.onHide()
    }

    override fun below(col: LinearLayout, query: String): Unit = with(ui) {
        // Empty without a server: the store drops the list when the server goes (Server page save, Settings).
        val team = app.team.entries
        if (team.isEmpty()) return@with
        col.sectionLabel(TEAM_LABEL)
        col.note(TEAM_NOTE)
        val shown = DictionaryPages.entries(team, Kind.WORDS, query)
        if (shown.isEmpty()) { col.card { emptyState("Nothing matches “${query.trim()}”", "Search looks at both the word and what it becomes.") }; return@with }
        col.card {
            for (e in shown) addView(wordRow(e).apply {
                isFocusable = true; contentDescription = DictionaryPages.row(e).description
                for (i in 0 until childCount) getChildAt(i).importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            })
        }
    }
    override fun above(col: LinearLayout): Unit = with(ui) {
        val pending = app.suggestions.update { it.pruneKnown(app.dictationDictionary()) }.pending
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
        col.card { for (e in entries) addView(wordRow(e).apply { background = ripple(null); entry(e) }) }
    }

    /** "heard →" in grey, then the word. */
    private fun wordRow(e: DictEntry): LinearLayout = with(ui) {
        val r = DictionaryPages.row(e)
        LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(20), dp(8), dp(20), dp(8))
            minimumHeight = dp(58)
            if (r.heard != null) addView(tv("${r.heard} →", TypeScale.ROW, t.ink2).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.MIDDLE }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { marginEnd = dp(10) })
            addView(tv(r.word, TypeScale.ROW, p.text).apply { maxLines = 2; ellipsize = TextUtils.TruncateAt.END }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
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

    companion object {
        const val TEAM_LABEL = "Team"
        const val TEAM_NOTE = "From your server. Your own entries win."
    }
}
