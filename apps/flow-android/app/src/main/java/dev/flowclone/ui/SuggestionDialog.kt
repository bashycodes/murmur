package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import dev.flowclone.FlowApp
import dev.flowclone.core.DictEntry
import dev.flowclone.learn.Suggestion

/**
 * A learned word's Add word / Add as fix / Dismiss dialog: Dictionary → Suggestions, and D68's
 * offer after a History correction. Back (or a tap outside) leaves the suggestion pending.
 * [onDone] gets the TalkBack announcement once the stores are updated.
 */
object SuggestionDialog {
    fun show(activity: Activity, s: Suggestion, title: String, message: String, onDone: (String) -> Unit) {
        val app = FlowApp.of(activity)
        AlertDialog.Builder(activity)
            .setTitle(title)
            .setMessage(message)
            .setPositiveButton("Add word") { _, _ -> onDone(add(app, s.word, false)) }
            .setNeutralButton("Add as fix") { _, _ -> onDone(add(app, s.word, true)) }
            .setNegativeButton("Dismiss") { _, _ -> onDone(dismiss(app, s.word)) }
            .show()
    }

    /** Accept [word] from the pending suggestions into the dictionary; returns the announcement. */
    fun add(app: FlowApp, word: String, asFix: Boolean): String {
        var entry: DictEntry? = null
        app.suggestions.update { cur -> cur.accept(word, asFix).let { (next, e) -> entry = e; next } }
        return entry?.let { app.dictionary.upsert(it); "Added ${it.replacement ?: it.word}" }.orEmpty()
    }

    /** Never suggest [word] again; returns the announcement. */
    fun dismiss(app: FlowApp, word: String): String {
        app.suggestions.update { it.dismiss(word) }
        return "Dismissed $word"
    }
}
