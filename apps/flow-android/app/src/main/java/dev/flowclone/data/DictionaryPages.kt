package dev.flowclone.data

import dev.flowclone.core.DictEntry

/**
 * Dictionary & snippets: the Dictionary and Snippets tabs (ui/tabs, AndroidDictionary /
 * AndroidSnippets.dc.html: search, card list, FAB) and the add / edit page (ui/DictionaryActivity).
 * The list and edit logic reuses [DictionaryEdit] (normalise, validate, apply, filter); this adds
 * the page texts and the edit form's Save rule (same as Settings: Save is enabled only when
 * something changed and the form validates; leaving dirty asks first).
 */
object DictionaryPages {
    enum class Kind { WORDS, SNIPPETS }

    /** Lists this long get a search field (History; the dictionary tabs always show theirs). */
    const val SEARCH_FROM = 8

    fun tab(k: Kind) = if (k == Kind.SNIPPETS) DictionaryEdit.Tab.SNIPPETS else DictionaryEdit.Tab.WORDS

    fun of(all: List<DictEntry>, k: Kind) = all.filter { it.isSnippet == (k == Kind.SNIPPETS) }

    /** One kind, sorted by word, filtered by [query] (word or what it becomes). */
    fun entries(all: List<DictEntry>, k: Kind, query: String) = DictionaryEdit.filter(all, query, tab(k))

    /**
     * A tab row. Dictionary: "[heard] → [word]" when the entry replaces what the engine hears
     * (heard null for a plain spelling word). Snippets: [word] is what you say, [detail] the text
     * it inserts on one line.
     */
    data class Row(val heard: String?, val word: String, val detail: String, val description: String)

    fun row(e: DictEntry): Row {
        val r = e.replacement
        return when {
            e.isSnippet -> flat(r.orEmpty()).let { Row(null, e.word, it, "${e.word}, inserts $it") }
            r != null -> Row(e.word, r, "", "${e.word}, replaced with $r")
            else -> Row(null, e.word, "", e.word)
        }
    }

    /** Line breaks folded to spaces, cut at 80 characters. */
    fun flat(text: String): String {
        val f = text.replace(Regex("\\s*\\n\\s*"), " ").trim()
        return if (f.length > 80) f.take(80) + "…" else f
    }

    data class Empty(val title: String, val hint: String)

    fun empty(k: Kind, query: String, total: Int): Empty {
        val q = query.trim()
        if (total > 0 && q.isNotEmpty()) return Empty("Nothing matches “$q”", "Search looks at both the word and what it becomes.")
        return if (k == Kind.SNIPPETS) Empty("No snippets yet", "Tap + to add one. Say the trigger phrase on its own and its text is inserted.")
        else Empty("No words yet", "Tap + to add one. Names and jargon you add are spelled right; a replacement fixes a word the engine keeps getting wrong.")
    }

    fun title(k: Kind) = if (k == Kind.SNIPPETS) "Snippets" else "Dictionary"

    /** Search box's TalkBack label (the placeholder is just "Search"). */
    fun searchLabel(k: Kind) = if (k == Kind.SNIPPETS) "Search snippets" else "Search dictionary"

    fun addLabel(k: Kind) = if (k == Kind.SNIPPETS) "Add a snippet" else "Add a word"

    /** Edit page title: "Add word", "Edit snippet", … */
    fun editTitle(isNew: Boolean, snippet: Boolean) = (if (isNew) "Add " else "Edit ") + if (snippet) "snippet" else "word"
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
