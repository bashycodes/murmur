package dev.flowclone.data

import dev.flowclone.core.CleanupLevel
import dev.flowclone.core.DictEntry
import dev.flowclone.core.FieldContext
import dev.flowclone.core.SessionConfig
import dev.flowclone.policy.AppCategorizer
import dev.flowclone.policy.BubbleVisibilityPolicy
import dev.flowclone.policy.EditorInputType
import dev.flowclone.session.SessionCap

/** History retention pref (10-clone-spec-android §4.8: keep / 24 h / never). */
enum class Retention(val key: String) {
    KEEP("keep"), DAY("day"), NEVER("never");

    val stores: Boolean get() = this != NEVER

    /** Rows older than this (epoch ms) are purged; null = keep everything. */
    fun cutoff(now: Long): Long? = when (this) {
        KEEP -> null
        DAY -> now - 86_400_000L
        NEVER -> Long.MAX_VALUE
    }

    companion object {
        fun fromKey(k: String?): Retention = entries.firstOrNull { it.key == k } ?: KEEP
    }
}

object DictionaryMerge {
    /** Import semantics: an incoming row replaces any existing row with the same word (case-insensitive). */
    fun merge(existing: List<DictEntry>, incoming: List<DictEntry>): List<DictEntry> {
        val inc = LinkedHashMap<String, DictEntry>()
        for (e in incoming) { val k = e.word.lowercase(); inc.remove(k); inc[k] = e }
        return existing.filter { it.word.lowercase() !in inc } + inc.values
    }
}

data class Settings(
    val language: String = "auto",
    val cleanup: CleanupLevel = CleanupLevel.NONE,
    val retention: Retention = Retention.KEEP,
    val capMinutes: Int = 5,
    val mockEngine: Boolean = false,
    val userDenylist: Set<String> = emptySet(),
) {
    fun toSessionConfig(dictionary: List<DictEntry>) = SessionConfig(
        languages = if (language == "auto" || language.isBlank()) emptyList() else listOf(language),
        dictionary = dictionary,
        cleanupLevel = cleanup,
        rawMode = cleanup == CleanupLevel.NONE,
        maxSamples = sessionCap().maxSamples,
    )

    fun sessionCap() = SessionCap(capMinutes.coerceIn(1, 20) * 60_000L)
}

/**
 * captureFieldSnapshot → core `FieldContext` (06 §1): ±500 chars around the selection
 * from `getSurroundingText(500, 500, 0)`; blank for context-extraction-excluded fields
 * (sensitive / URI inputType) and banking apps.
 */
object FieldContextBuilder {
    const val WINDOW = 500

    fun build(pkg: String?, inputType: Int, surrounding: String?, selStart: Int, selEnd: Int, browserHost: String? = null): FieldContext {
        val category = AppCategorizer.categorize(pkg, browserHost).coreName
        val base = FieldContext(appId = pkg.orEmpty(), urlHost = browserHost.orEmpty(), appCategory = category)
        if (surrounding == null || EditorInputType.isContentExtractionExcluded(inputType) || BubbleVisibilityPolicy.isBanking(pkg)) return base
        val len = surrounding.length
        val a = minOf(selStart, selEnd).coerceIn(0, len)
        val b = maxOf(selStart, selEnd).coerceIn(0, len)
        return base.copy(
            before = surrounding.substring(0, a).takeLast(WINDOW),
            selected = surrounding.substring(a, b),
            after = surrounding.substring(b).take(WINDOW),
        )
    }
}
