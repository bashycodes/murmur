package dev.flowclone.learn

import dev.flowclone.core.DictEntry
import dev.flowclone.core.LearnCandidate
import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject

/**
 * Dictionary suggestions from learn-from-edits, waiting for the user (Windows
 * `apps/flow-windows/src/suggestions.rs`). The clone never adds learned words by itself:
 * a candidate lands in [pending], the Dictionary screen lists it, and only an explicit Add turns
 * it into a dictionary entry. Dismissed words are remembered so they are not suggested again.
 * Stored as words only (no transcripts).
 */
data class Suggestion(
    /** Proposed dictionary word (what the user typed). */
    val word: String,
    /** What the dictation produced instead. */
    val heard: String,
    /** Package the correction was seen in. */
    val app: String,
    /** Last seen, epoch ms. */
    val ts: Long,
    /** Times the same correction was seen. */
    val count: Int = 1,
)

data class Suggestions(val pending: List<Suggestion> = emptyList(), val dismissed: List<String> = emptyList()) {
    data class Added(val next: Suggestions, val added: List<Suggestion>)

    /**
     * Add candidates. Words already in the dictionary or dismissed are skipped; a repeat bumps
     * its count and moves it to the top. Newest first, capped at [MAX_PENDING].
     */
    fun add(cands: List<LearnCandidate>, app: String, ts: Long, dict: List<DictEntry>): Added {
        val list = pending.toMutableList()
        val added = ArrayList<Suggestion>()
        for (c in cands) {
            val word = c.corrected.trim()
            if (word.isEmpty() || inDictionary(word, dict) || key(word) in dismissed) continue
            val i = list.indexOfFirst { key(it.word) == key(word) }
            if (i >= 0) {
                val s = list.removeAt(i)
                list.add(0, s.copy(count = s.count + 1, ts = ts, heard = c.original))
                continue
            }
            val s = Suggestion(word, c.original, app, ts)
            list.add(0, s)
            added += s
        }
        return Added(copy(pending = list.take(MAX_PENDING)), added)
    }

    /**
     * Remove [word] from pending and return the entry to add: the word itself (biases speech
     * recognition and cleanup), or `heard → word` when [asReplacement].
     */
    fun accept(word: String, asReplacement: Boolean = false): Pair<Suggestions, DictEntry?> {
        val s = pending.firstOrNull { key(it.word) == key(word) } ?: return this to null
        val entry = if (asReplacement && s.heard.isNotBlank()) DictEntry(s.heard, s.word) else DictEntry(s.word)
        return copy(pending = pending - s) to entry
    }

    /** Drop [word] from pending and never suggest it again. */
    fun dismiss(word: String): Suggestions {
        val k = key(word)
        val d = if (k.isEmpty() || k in dismissed) dismissed else (dismissed + k).takeLast(MAX_DISMISSED)
        return Suggestions(pending.filterNot { key(it.word) == k }, d)
    }

    /** Drop pending words that have since been added by hand. */
    fun pruneKnown(dict: List<DictEntry>) = copy(pending = pending.filterNot { inDictionary(it.word, dict) })

    companion object {
        /** A Suggestions row's subtitle: what was heard, and how often the fix was seen. */
        fun subtitle(s: Suggestion): String =
            "Heard \u201c${s.heard}\u201d" + if (s.count > 1) " \u00b7 fixed ${s.count} times" else ""

        const val MAX_PENDING = 50
        const val MAX_DISMISSED = 500

        private fun key(w: String) = w.trim().lowercase()

        private fun inDictionary(word: String, dict: List<DictEntry>): Boolean {
            val k = key(word)
            return dict.any { key(it.word) == k || it.replacement?.let(::key) == k }
        }

        fun encode(s: Suggestions): String = JSONObject()
            .put("pending", JSONArray().apply {
                for (p in s.pending) put(JSONObject().put("word", p.word).put("heard", p.heard).put("app", p.app).put("ts", p.ts).put("count", p.count))
            })
            .put("dismissed", JSONArray(s.dismissed))
            .toString()

        /** A missing or unreadable file = empty. */
        fun decode(json: String?): Suggestions {
            if (json.isNullOrBlank()) return Suggestions()
            return try {
                val o = JSONObject(json)
                val p = o.optJSONArray("pending") ?: JSONArray()
                val d = o.optJSONArray("dismissed") ?: JSONArray()
                Suggestions(
                    (0 until p.length()).mapNotNull { p.optJSONObject(it) }.mapNotNull { j ->
                        val w = j.optString("word").trim()
                        if (w.isEmpty()) null else Suggestion(w, j.optString("heard"), j.optString("app"), j.optLong("ts"), j.optInt("count", 1).coerceAtLeast(1))
                    },
                    (0 until d.length()).map { d.optString(it) }.filter { it.isNotEmpty() },
                )
            } catch (_: JSONException) {
                Suggestions()
            }
        }
    }
}

/**
 * D68 name learning: the user corrected a dictation's text in History. Every learn-from-edits
 * candidate becomes a pending [Suggestion] (the Dictionary screen lists it, as for field
 * edits); the ones that read as a name ([LearnCandidate.looksLikeName]: a capitalised correction, not
 * sentence case) come back in [Learned.names] so History offers them at once.
 */
object HistoryCorrection {
    data class Learned(val next: Suggestions, val names: List<Suggestion>)

    fun learn(
        before: String, after: String, current: Suggestions, dict: List<DictEntry>, app: String, ts: Long,
        learnEdit: (pasted: String, edited: String) -> List<LearnCandidate>,
    ): Learned {
        val b = EditText.normalize(before)
        val a = EditText.normalize(after)
        if (a.isEmpty() || a == b) return Learned(current, emptyList())
        val cands = runCatching { learnEdit(b, a) }.getOrDefault(emptyList())
        val next = current.add(cands, app, ts, dict).next
        val nameKeys = cands.filter { it.looksLikeName }.map { it.corrected.trim().lowercase() }.toSet()
        return Learned(next, next.pending.filter { it.word.trim().lowercase() in nameKeys })
    }
}

/**
 * What happens when a watch ends: bound the edit, ask the core for candidates, fold them into
 * [current]. Returns null when nothing changed.
 */
object EditLearning {
    /**
     * Watch the field after an insert? Not when the setting is off, nothing was inserted, the
     * plan pressed Enter (a sent message clears the field) or no field was captured at start.
     */
    fun shouldWatch(enabled: Boolean, pasted: String, sentKey: Boolean, hasContext: Boolean): Boolean =
        enabled && !sentKey && hasContext && pasted.isNotBlank()

    fun learn(
        pasted: String, outcome: Outcome, current: Suggestions, dict: List<DictEntry>, app: String, ts: Long,
        learnEdit: (pasted: String, edited: String) -> List<LearnCandidate>,
    ): Suggestions.Added? {
        val edited = outcome.edited ?: return null
        val bounded = EditText.boundEdit(pasted, edited)
        if (bounded.isEmpty()) return null
        val cands = runCatching { learnEdit(EditText.normalize(pasted), bounded) }.getOrDefault(emptyList())
        if (cands.isEmpty()) return null
        return current.add(cands, app, ts, dict).takeIf { it.next != current }
    }
}
