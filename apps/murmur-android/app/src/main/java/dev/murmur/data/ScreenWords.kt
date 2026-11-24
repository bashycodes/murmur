package dev.murmur.data

import dev.murmur.core.FieldContext

/**
 * "Send on-screen words" (Settings → Cloud): the visible text of the focused app's window,
 * for `FieldContext.screenWords`. The core uses them like the text around the cursor: as
 * spelling hints for on-device speech recognition and casing, for murmur-server, and for
 * cleanup; cloud cleanup gets them only under its consent (the core's prompt scope).
 * Cloud speech-to-text never gets them (it only sends dictionary terms).
 *
 * Read only when the toggle is on, at dictation start, on the call that reads the text around
 * the cursor (before the microphone starts, so the walk is cheap: [BUDGET_MS], [MAX_DEPTH],
 * [MAX_NODES]). The result is bounded too: [MAX_CHARS] in all, [MAX_ENTRY_CHARS] and
 * [MAX_ENTRIES], which keeps it well inside murmur-server's context limits (1000 entries,
 * 64 000 bytes: 2000 UTF-16 chars are at most 6000 UTF-8 bytes).
 *
 * Never read: a protected node (password, sensitive / URI input, accessibility-data-sensitive)
 * and what is under it, hidden nodes, and the focused field (the text around the cursor covers
 * it). Nothing at all is read while the focused field is protected (password, sensitive / URI
 * input), in a banking app or an app on the user's denylist, or when the app is unknown. The
 * text around the cursor follows the field and banking-app rules but not the denylist one.
 */
object ScreenWords {
    const val MAX_CHARS = 2000
    const val MAX_ENTRY_CHARS = 200
    const val MAX_ENTRIES = 200
    const val MAX_DEPTH = 25
    const val MAX_NODES = 400
    const val BUDGET_MS = 30L

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

    /** What decides whether the screen is read, all known at dictation start. */
    data class Gate(
        /** The "Send on-screen words" toggle. */
        val enabled: Boolean,
        /** The focused app, or null when neither the window nor the field names it. */
        val pkg: String?,
        val fieldProtected: Boolean,
        val userDenylist: Set<String>,
    )

    fun allowed(g: Gate): Boolean =
        g.enabled && g.pkg != null && !g.fieldProtected && !dev.murmur.debug.DebugRedact.isProtectedApp(g.pkg, g.userDenylist)

    /**
     * [ctx] with the words on screen when [gate] allows it, else with none. [fieldText] is the
     * focused field's text ([nodeText], else the IME's), left out of the words. [root] is only
     * asked for when allowed.
     */
    fun attach(
        ctx: FieldContext,
        gate: Gate,
        root: () -> Node?,
        nodeText: String?,
        imeText: String?,
        now: () -> Long = ::monotonicMs,
    ): FieldContext {
        if (!allowed(gate)) return ctx.copy(screenWords = emptyList())
        val words = runCatching { collect(root(), nodeText ?: imeText, now() + BUDGET_MS, now) }.getOrDefault(emptyList())
        return ctx.copy(screenWords = words)
    }

    /**
     * Visible texts in tree order, whitespace-collapsed, case-insensitively deduped, without
     * [fieldText]; stops at [MAX_CHARS] / [MAX_ENTRIES], and at [deadline] (by [now]) keeping
     * what it has.
     */
    fun collect(
        root: Node?,
        fieldText: String? = null,
        deadline: Long = monotonicMs() + BUDGET_MS,
        now: () -> Long = ::monotonicMs,
    ): List<String> {
        root ?: return emptyList()
        val out = ArrayList<String>()
        val seen = HashSet<String>()
        fieldText?.let(::normalize)?.let { seen += it.lowercase() }
        var total = 0
        var visited = 0
        var done = false

        fun add(raw: String?) {
            val s = raw?.let(::normalize) ?: return
            if (s.none { it.isLetterOrDigit() } || !seen.add(s.lowercase())) return
            if (total + s.length > MAX_CHARS) { done = true; return }
            out += s
            total += s.length
            if (out.size >= MAX_ENTRIES) done = true
        }

        fun walk(n: Node, depth: Int) {
            if (done || depth > MAX_DEPTH) return
            if (visited >= MAX_NODES || now() >= deadline) { done = true; return }
            visited++
            if (n.isProtected || !n.isVisible || n.isFocusedField) return
            add(n.text)
            add(n.description)
            for (i in 0 until n.childCount) {
                if (done) return
                n.child(i)?.let { walk(it, depth + 1) }
            }
        }

        walk(root, 0)
        return out
    }

    private fun monotonicMs() = System.nanoTime() / 1_000_000

    private fun normalize(s: String): String? =
        s.trim().replace(WHITESPACE, " ").take(MAX_ENTRY_CHARS).trim().ifEmpty { null }

    private val WHITESPACE = Regex("\\s+")
}
