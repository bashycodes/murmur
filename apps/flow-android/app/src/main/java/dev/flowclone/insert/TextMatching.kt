package dev.flowclone.insert

import dev.flowclone.policy.PolicyLists
import java.text.Normalizer

/** Verification / node-write matching rules (spec 06 §4). Pure functions. */
object TextMatching {
    /** IAS:1893 — field contains trim(text); >500 chars also accepts first/last 400. Case-sensitive. */
    fun containsCommittedText(field: String, text: String): Boolean {
        val t = text.trim()
        if (field.contains(t)) return true
        if (t.length > 500) return field.contains(t.takeLast(400).trim()) || field.contains(t.take(400).trim())
        return false
    }

    enum class Compose { AlreadyPresent, RefusedUnreadable, Write, RefusedNonEmpty }

    /** NodeWriteComposer.compose: only ever writes into an empty (or placeholder) field. */
    fun compose(text: String, live: String?): Compose {
        val t = text.trim()
        if (live != null && t.isNotEmpty() && live.contains(t)) return Compose.AlreadyPresent
        if (live == null) return Compose.RefusedUnreadable
        if (live.isEmpty()) return Compose.Write
        return Compose.RefusedNonEmpty
    }

    fun normalizePlaceholder(s: String): String =
        s.trim().trimEnd('…', '.', ':', ',', '!', '?', '‥').trim().lowercase(java.util.Locale.ROOT)

    private fun fuzzy(c: String, declared: String?) =
        !declared.isNullOrBlank() && normalizePlaceholder(c).let { it.isNotEmpty() && it == normalizePlaceholder(declared) }

    /** KnownComposeHints.candidatesFor(pkg, hintAware = true, staleProbed). */
    fun knownComposeHints(pkg: String?, staleProbed: Boolean): Set<String> =
        if (staleProbed && PolicyLists.usesComposePlaceholders(pkg)) PolicyLists.COMPOSE_PLACEHOLDERS else emptySet()

    private fun matchesKnown(s: String, known: Set<String>): Boolean {
        val n = normalizePlaceholder(Normalizer.normalize(s, Normalizer.Form.NFC))
        return n.length in 1..32 && n in known
    }

    /**
     * EditableNodeText.read: the field's text with placeholders read as "" so the
     * node tier may write into them. null = unreadable.
     */
    fun readNodeText(node: NodeSnapshot, editorHint: String?, known: Set<String>): String? {
        if (node.showingHint) return ""
        val s = node.text ?: return null
        if (s.isNotEmpty() && isPlaceholder(s, node, editorHint, known)) return ""
        return s
    }

    private fun isPlaceholder(s: String, node: NodeSnapshot, editorHint: String?, known: Set<String>): Boolean {
        val declared = node.hint
        if (!declared.isNullOrBlank()) return fuzzy(s, declared)
        val noEditorHint = editorHint.isNullOrBlank()
        if (!noEditorHint && fuzzy(s, editorHint)) return true
        if (noEditorHint && matchesKnown(s, known)) return true
        // Review P3: Wispr also read text equal to the contentDescription (≤ 48 chars) as a
        // placeholder, but apps that mirror the field's text into its description then had
        // ACTION_SET_TEXT overwrite real user text. Not a placeholder: the node tier refuses and
        // the text goes to the (sensitive, self-clearing) clipboard instead.
        return false
    }
}
