package dev.flowclone.data

import dev.flowclone.core.DictEntry

/**
 * Dictionary & snippets (ui/DictionaryActivity) as a hub + list pages + an edit page, in the
 * Settings look. The list and edit logic reuses [DictionaryEdit] (normalise, validate, apply,
 * filter); this adds the page texts and the edit form's Save rule (same as Settings: Save is
 * enabled only when something changed and the form validates; leaving dirty asks first).
 */
object DictionaryPages {
    enum class Kind { WORDS, SNIPPETS }

    /** Lists this long get a search field. */
    const val SEARCH_FROM = 8

    fun tab(k: Kind) = if (k == Kind.SNIPPETS) DictionaryEdit.Tab.SNIPPETS else DictionaryEdit.Tab.WORDS

    fun of(all: List<DictEntry>, k: Kind) = all.filter { it.isSnippet == (k == Kind.SNIPPETS) }

    fun entries(all: List<DictEntry>, k: Kind, query: String) = DictionaryEdit.filter(all, query, tab(k))

    fun showSearch(count: Int) = count >= SEARCH_FROM

    private fun plural(n: Int, one: String) = if (n == 1) "1 $one" else "$n ${one}s"

    fun hubSubtitle(all: List<DictEntry>, k: Kind): String {
        val list = of(all, k)
        if (list.isEmpty()) return "None yet"
        if (k == Kind.SNIPPETS) return plural(list.size, "snippet")
        val r = list.count { it.replacement != null }
        return plural(list.size, "word") + if (r > 0) " · " + plural(r, "replacement") else ""
    }

    /** Main screen MORE row. */
    fun mainSubtitle(all: List<DictEntry>): String {
        val w = of(all, Kind.WORDS).size
        val sn = of(all, Kind.SNIPPETS).size
        return if (w + sn == 0) "No words or snippets yet" else plural(w, "word") + " · " + plural(sn, "snippet")
    }

    fun rowSubtitle(e: DictEntry): String {
        val r = e.replacement ?: return "Spelling hint for the speech engine"
        val flat = r.replace(Regex("\\s*\\n\\s*"), " ").trim()
        val short = if (flat.length > 80) flat.take(80) + "…" else flat
        return if (e.isSnippet) short else "→ $short"
    }

    data class Empty(val title: String, val hint: String)

    fun empty(k: Kind, query: String, total: Int): Empty {
        val q = query.trim()
        if (total > 0 && q.isNotEmpty()) return Empty("Nothing matches “$q”", "Search looks at both the word and what it becomes.")
        return if (k == Kind.SNIPPETS) Empty("No snippets yet", "Tap Add in the top bar. Say the trigger phrase on its own and its text is inserted.")
        else Empty("No words yet", "Tap Add in the top bar. Names and jargon you add are spelled right; a replacement fixes a word the engine keeps getting wrong.")
    }

    fun title(k: Kind) = if (k == Kind.SNIPPETS) "Snippets" else "Dictionary"
}

/** The edit page: a new entry ([original] null) or an existing one. Fields as typed. */
data class DictionaryDraft(val original: DictEntry?, val word: String, val replacement: String, val snippet: Boolean) {
    enum class Field { WORD, REPLACEMENT }

    fun entry(): DictEntry = DictionaryEdit.normalize(word, replacement, snippet)

    val dirty: Boolean get() = original?.let { entry() != it || snippet != it.isSnippet } ?: (word.isNotBlank() || replacement.isNotBlank())

    /** Both fields checked independently, so both errors can show at once (same rules as [DictionaryEdit.validate]). */
    fun errors(all: List<DictEntry>): Map<Field, String> {
        val e = entry()
        val out = LinkedHashMap<Field, String>()
        DictionaryEdit.validate(all, original?.word, DictEntry(e.word, null, false), wantSnippet = false)?.let { out[Field.WORD] = it }
        DictionaryEdit.validate(emptyList(), null, e.copy(word = "x"), wantSnippet = snippet)?.let { out[Field.REPLACEMENT] = it }
        return out
    }

    /** Errors are not shown on a form nobody has typed into yet. */
    fun visibleErrors(all: List<DictEntry>): Map<Field, String> = if (dirty) errors(all) else emptyMap()

    fun canSave(all: List<DictEntry>) = dirty && errors(all).isEmpty()

    fun save(all: List<DictEntry>): List<DictEntry> = DictionaryEdit.apply(all, original?.word, entry())

    companion object {
        fun new(k: DictionaryPages.Kind) = DictionaryDraft(null, "", "", k == DictionaryPages.Kind.SNIPPETS)
        fun of(e: DictEntry) = DictionaryDraft(e, e.word, e.replacement.orEmpty(), e.isSnippet)
    }
}
