package dev.murmur.data

/**
 * "Send on-screen words" (Cloud → AI cleanup): the visible text of the focused app's window,
 * for `FieldContext.screenWords`. Names and terms on screen help the cleanup spell them.
 *
 * Read only when the toggle is on, at dictation start, on the thread that reads the text
 * around the cursor. The walk is bounded ([MAX_DEPTH], [MAX_NODES]) and so is the result
 * ([MAX_CHARS] in all, [MAX_ITEM] per entry). Never read: a protected node (password,
 * sensitive / URI input, accessibility-data-sensitive) and what is under it, the focused
 * field (the text around the cursor covers it), and anything in a banking or denylisted app
 * or while the focused field itself is protected.
 */
object ScreenWords {
    const val MAX_CHARS = 2000
    const val MAX_ITEM = 200
    const val MAX_DEPTH = 25
    const val MAX_NODES = 400

    /** One accessibility node, as much of it as the walk needs. */
    interface Node {
        val text: String?
        val description: String?
        /** Password, sensitive / URI input type, or accessibility-data-sensitive. */
        val isProtected: Boolean
        val isVisible: Boolean
        /** The field the dictation goes into. */
        val isFocusedField: Boolean
        val childCount: Int
        fun child(i: Int): Node?
    }

    /** Whether to read the screen at all: the toggle, then the same rules as the text around the cursor. */
    fun allowed(enabled: Boolean, pkg: String?, fieldProtected: Boolean, userDenylist: Set<String>): Boolean =
        enabled && !fieldProtected && !dev.murmur.debug.DebugRedact.isProtectedApp(pkg, userDenylist)

    /**
     * Visible texts in tree order, whitespace-collapsed, case-insensitively deduped, without
     * [fieldText] (the focused field's own text); stops at [MAX_CHARS].
     */
    fun collect(root: Node?, fieldText: String? = null): List<String> {
        root ?: return emptyList()
        val out = ArrayList<String>()
        val seen = HashSet<String>()
        fieldText?.let(::normalize)?.let { seen += it.lowercase() }
        var total = 0
        var visited = 0
        var full = false

        fun add(raw: String?) {
            val s = raw?.let(::normalize) ?: return
            if (s.none { it.isLetterOrDigit() } || !seen.add(s.lowercase())) return
            if (total + s.length > MAX_CHARS) { full = true; return }
            out += s
            total += s.length
        }

        fun walk(n: Node, depth: Int) {
            if (full || depth > MAX_DEPTH || visited >= MAX_NODES) return
            visited++
            if (n.isProtected || !n.isVisible) return
            if (n.isFocusedField) return
            add(n.text)
            add(n.description)
            for (i in 0 until n.childCount) {
                if (full || visited >= MAX_NODES) return
                n.child(i)?.let { walk(it, depth + 1) }
            }
        }

        walk(root, 0)
        return out
    }

    private fun normalize(s: String): String? =
        s.trim().replace(WHITESPACE, " ").take(MAX_ITEM).trim().ifEmpty { null }

    private val WHITESPACE = Regex("\\s+")
}
