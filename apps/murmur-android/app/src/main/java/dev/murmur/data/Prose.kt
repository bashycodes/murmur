package dev.murmur.data

/** Small wording helpers shared by the summaries and banners. */
object Prose {
    /** "a", "a and b", "a, b and c" ([conjunction] "or" for alternatives). */
    fun list(items: List<String>, conjunction: String = "and"): String = when (items.size) {
        0 -> ""
        1 -> items[0]
        else -> items.dropLast(1).joinToString(", ") + " $conjunction " + items.last()
    }
}
