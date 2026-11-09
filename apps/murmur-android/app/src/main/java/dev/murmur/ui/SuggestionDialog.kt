package dev.murmur.ui

import android.app.Activity
import android.app.AlertDialog
import dev.murmur.MurmurApp
import dev.murmur.core.DictEntry
import dev.murmur.learn.Suggestion

/**
 * A learned word's dialog: Dictionary → Suggestions (Add word / Add as fix / Dismiss), and D68's
 * offer after a History correction ([offerName]: Add word / Add as fix / Not now). Back (or a
 * tap outside) leaves the suggestion pending. [show]'s `onDone` gets the TalkBack announcement
 * once the stores are updated.
 */
object SuggestionDialog {
    /** D68: what [offerName] says. */
    fun offerTitle(s: Suggestion) = "Add “${s.word}” to your dictionary?"
    const val OFFER_TEXT = "Speech recognition then listens for it, and cleanup spells it this way. " +
        "Not now keeps it in the Dictionary's suggestions."
    const val NOT_NOW = "Not now"

    /** The Dictionary's suggestion dialog: its negative button dismisses [s] for good. */
    fun show(activity: Activity, s: Suggestion, title: String, message: String, onDone: (String) -> Unit) =
        build(activity, s, title, message, onDone)
            .setNegativeButton("Dismiss") { _, _ -> onDone(dismiss(MurmurApp.of(activity), s.word)) }
            .show()

    /** D68: offer a corrected name; Not now leaves it pending in the Dictionary's suggestions. */
    fun offerName(activity: Activity, s: Suggestion, onDone: (String) -> Unit) =
        build(activity, s, offerTitle(s), OFFER_TEXT, onDone)
            .setNegativeButton(NOT_NOW) { _, _ -> onDone("") }
            .show()

    private fun build(activity: Activity, s: Suggestion, title: String, message: String, onDone: (String) -> Unit): AlertDialog.Builder {
        val app = MurmurApp.of(activity)
        return AlertDialog.Builder(activity)
            .setTitle(title)
            .setMessage(message)
            .setPositiveButton("Add word") { _, _ -> onDone(add(app, s.word, false)) }
            .setNeutralButton("Add as fix") { _, _ -> onDone(add(app, s.word, true)) }
    }

    /** Accept [word] from the pending suggestions into the dictionary; returns the announcement. */
    fun add(app: MurmurApp, word: String, asFix: Boolean): String {
        var entry: DictEntry? = null
        app.suggestions.update { cur -> cur.accept(word, asFix).let { (next, e) -> entry = e; next } }
        return entry?.let { app.dictionary.upsert(it); "Added ${it.replacement ?: it.word}" }.orEmpty()
    }

    /** Never suggest [word] again; returns the announcement. */
    fun dismiss(app: MurmurApp, word: String): String {
        app.suggestions.update { it.dismiss(word) }
        return "Dismissed $word"
    }
}
