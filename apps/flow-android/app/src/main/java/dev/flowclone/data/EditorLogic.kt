package dev.flowclone.data

import dev.flowclone.core.CommitStatus
import dev.flowclone.core.DictEntry

/** Dictionary / snippet editor rules (03: words and snippet triggers ≤ 60 chars). */
object DictionaryEdit {
    const val MAX_WORD = 60
    /** Clone limit for snippet expansions (Wispr's ≤ 60 is too tight for signatures). */
    const val MAX_EXPANSION = 2_000

    enum class Tab { ALL, WORDS, SNIPPETS }

    fun normalize(word: String, replacement: String, snippet: Boolean): DictEntry {
        val w = word.trim().replace(Regex("\\s+"), " ")
        val r = replacement.takeIf { it.isNotBlank() }?.let { if (snippet) it.trimEnd() else it.trim() }
        return DictEntry(w, r, snippet && r != null)
    }

    /** Null when [e] may be saved (as a new entry, or replacing [original]). */
    fun validate(all: List<DictEntry>, original: String?, e: DictEntry, wantSnippet: Boolean = e.isSnippet): String? {
        val w = e.word.trim()
        if (w.isEmpty()) return "Enter a word or trigger phrase."
        if (w.length > MAX_WORD) return "Keep it to $MAX_WORD characters or fewer."
        if (wantSnippet && e.replacement.isNullOrBlank()) return "A snippet needs the text it expands to."
        if ((e.replacement?.length ?: 0) > MAX_EXPANSION) return "Expansion is limited to $MAX_EXPANSION characters."
        val clash = all.any { it.word.equals(w, ignoreCase = true) && !it.word.equals(original, ignoreCase = true) }
        if (clash) return "\"$w\" is already in the dictionary."
        return null
    }

    /** Replace [original] in place (keeps order), or append. */
    fun apply(all: List<DictEntry>, original: String?, e: DictEntry): List<DictEntry> {
        val i = if (original == null) -1 else all.indexOfFirst { it.word.equals(original, ignoreCase = true) }
        if (i < 0) return all.filterNot { it.word.equals(e.word, ignoreCase = true) } + e
        return all.toMutableList().also { it[i] = e }
    }

    fun filter(all: List<DictEntry>, query: String, tab: Tab): List<DictEntry> {
        val q = query.trim()
        return all.asSequence()
            .filter { when (tab) { Tab.ALL -> true; Tab.WORDS -> !it.isSnippet; Tab.SNIPPETS -> it.isSnippet } }
            .filter { q.isEmpty() || it.word.contains(q, true) || it.replacement?.contains(q, true) == true }
            .sortedBy { it.word.lowercase() }
            .toList()
    }
}

/** History screen presentation: search, raw vs formatted text, status labels. */
object HistoryView {
    fun filter(rows: List<HistoryRow>, query: String): List<HistoryRow> {
        val q = query.trim()
        if (q.isEmpty()) return rows
        return rows.filter { it.finalText.contains(q, true) || it.asrText.contains(q, true) }
    }

    fun text(r: HistoryRow, raw: Boolean): String {
        val t = if (raw) r.asrText.ifEmpty { r.finalText } else r.finalText.ifEmpty { r.asrText }
        return t.ifEmpty { "(no speech)" }
    }

    fun label(r: HistoryRow): String = when (runCatching { CommitStatus.valueOf(r.status) }.getOrNull()) {
        CommitStatus.FORMATTED -> "AI cleaned"
        CommitStatus.RAW_TRANSCRIPT -> "Transcript"
        CommitStatus.EMPTY -> "No speech"
        CommitStatus.ERROR, null -> "Error"
    }

    /** True when the raw ASR text differs from what was inserted (worth a "show raw" toggle). */
    fun hasRawVariant(r: HistoryRow) = r.asrText.isNotEmpty() && r.asrText != r.finalText
}
