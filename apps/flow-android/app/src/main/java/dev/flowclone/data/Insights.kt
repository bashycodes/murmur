package dev.flowclone.data

import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.TimeZone

/**
 * Insights page math: a Kotlin mirror of `crates/flow-core/src/insights.rs` (the Windows
 * Insights tab uses the Rust original). Same formulas, same JSON file shape; flow-core's golden
 * vectors (`crates/flow-core/fixtures/insights/insights_vectors.json`) are checked against both
 * implementations (InsightsVectorsTest here, `tests/insights_vectors.rs` there).
 *
 * Mirrored rather than called through JNI because `libflow_ffi.so` is optional at build and run
 * time (built separately, missing on a fresh checkout, absent from the JVM unit tests) while
 * these stats must always work; the math is ~100 pure lines.
 *
 * Days are local calendar dates as epoch days (days since 1970-01-01), keyed `YYYY-MM-DD`.
 */
object Insights {
    /** Typing speed the "time saved" estimate compares against (words/minute). */
    const val DEFAULT_TYPING_WPM = 40.0
    const val TOP_APPS = 5
    const val CHART_DAYS = 7
    private const val MS_PER_DAY = 86_400_000L

    /** Local epoch day of Unix time [tsMs] in a zone [offsetMin] minutes ahead of UTC. */
    fun dayOf(tsMs: Long, offsetMin: Int): Long = Math.floorDiv(tsMs + offsetMin * 60_000L, MS_PER_DAY)

    fun key(day: Long): String = java.time.LocalDate.ofEpochDay(day).toString()

    /** `YYYY-MM-DD` → epoch day; null for junk or impossible dates (2026-02-30). */
    fun parse(s: String): Long? = runCatching { java.time.LocalDate.parse(s).toEpochDay() }.getOrNull()

    /** Whitespace-separated words, as the Windows shell counts them (Rust `split_whitespace`). */
    fun words(text: String): Int {
        var n = 0
        var inWord = false
        for (c in text) {
            if (c.isWhitespace()) inWord = false else if (!inWord) { inWord = true; n++ }
        }
        return n
    }

    data class DayStats(val words: Long = 0, val ms: Long = 0, val count: Long = 0, val apps: Map<String, Long> = emptyMap())

    /** The persisted aggregate: `YYYY-MM-DD` (local) → [DayStats]. Counts only, never text. */
    data class Aggregates(
        /** History was folded in once (or the stats were reset, which must not bring it back). */
        val backfilled: Boolean = false,
        val days: Map<String, DayStats> = emptyMap(),
    ) {
        /** One committed dictation on [day]; an empty transcript is not a dictation. */
        fun add(day: Long, words: Long, ms: Long, app: String): Aggregates {
            if (words <= 0) return this
            val k = key(day)
            val d = days[k] ?: DayStats()
            val apps = d.apps.toMutableMap().apply { merge(app, words, Long::plus) }
            val nd = DayStats(d.words + words, d.ms + ms.coerceAtLeast(0), d.count + 1, apps.toSortedMap())
            return copy(days = (days + (k to nd)).toSortedMap())
        }

        /** Forget every count; stays [backfilled] so History is not re-added. */
        fun reset(): Aggregates = Aggregates(backfilled = true)

        private fun activeDays(): List<Long> = days.entries.filter { it.value.count > 0 }.mapNotNull { parse(it.key) }.sorted()

        fun summary(today: Long, typingWpm: Double): Summary {
            var totalWords = 0L; var totalMs = 0L; var dictations = 0L
            val apps = HashMap<String, Long>()
            for (s in days.values) {
                totalWords += s.words; totalMs += s.ms; dictations += s.count
                for ((a, w) in s.apps) apps.merge(a, w, Long::plus)
            }
            val top = apps.filter { it.value > 0 }.map { AppWords(it.key, it.value) }
                .sortedWith(compareByDescending<AppWords> { it.words }.thenBy { it.app })
                .take(TOP_APPS)
            val last7 = (CHART_DAYS - 1 downTo 0).map { back ->
                val k = key(today - back)
                DayWords(k, days[k]?.words ?: 0)
            }
            val active = activeDays()
            val (current, longest) = streaks(active, today)
            val minutes = totalMs / 60_000.0
            val wpm = if (typingWpm > 0.0) typingWpm else DEFAULT_TYPING_WPM
            return Summary(
                totalWords = totalWords,
                weekWords = last7.sumOf { it.words },
                dictations = dictations,
                totalMs = totalMs,
                avgWpm = if (totalMs > 0) totalWords / minutes else null,
                currentStreak = current,
                longestStreak = longest,
                typingWpm = wpm,
                savedMinutes = (totalWords / wpm - minutes).coerceAtLeast(0.0),
                topApps = top,
                last7 = last7,
                since = active.firstOrNull()?.let(::key),
            )
        }

        fun toJson(): JSONObject = JSONObject().put("backfilled", backfilled).put(
            "days",
            JSONObject().apply {
                for ((k, d) in days) put(
                    k,
                    JSONObject().put("words", d.words).put("ms", d.ms).put("count", d.count)
                        .put("apps", JSONObject().apply { for ((a, w) in d.apps) put(a, w) }),
                )
            },
        )

        companion object {
            /** Tolerant like the Rust serde shape: missing fields default; throws on malformed JSON. */
            fun fromJson(json: String): Aggregates {
                val o = JSONObject(json)
                val days = sortedMapOf<String, DayStats>()
                o.optJSONObject("days")?.let { ds ->
                    for (k in ds.keys()) {
                        val d = ds.getJSONObject(k)
                        val apps = sortedMapOf<String, Long>()
                        d.optJSONObject("apps")?.let { a -> for (n in a.keys()) apps[n] = a.getLong(n) }
                        days[k] = DayStats(d.optLong("words"), d.optLong("ms"), d.optLong("count"), apps)
                    }
                }
                return Aggregates(o.optBoolean("backfilled", false), days)
            }
        }
    }

    data class AppWords(val app: String, val words: Long)
    data class DayWords(val date: String, val words: Long)

    /** What the Insights page shows (field names as the Rust `Summary`). */
    data class Summary(
        val totalWords: Long,
        /** Today and the 6 days before it. */
        val weekWords: Long,
        val dictations: Long,
        val totalMs: Long,
        /** Words per minute of recorded audio; null before any audio. */
        val avgWpm: Double?,
        /** Consecutive days ending today, or yesterday while today has nothing yet. */
        val currentStreak: Int,
        val longestStreak: Int,
        val typingWpm: Double,
        /** Typing the same words at [typingWpm] minus the time spent speaking (never negative). */
        val savedMinutes: Double,
        val topApps: List<AppWords>,
        /** Oldest first, ending today. */
        val last7: List<DayWords>,
        /** First day with data. */
        val since: String?,
    ) {
        /** The Rust serde shape (snake_case), for the shared vectors. */
        fun toJson(): JSONObject = JSONObject()
            .put("total_words", totalWords).put("week_words", weekWords).put("dictations", dictations).put("total_ms", totalMs)
            .put("avg_wpm", avgWpm ?: JSONObject.NULL)
            .put("current_streak", currentStreak).put("longest_streak", longestStreak)
            .put("typing_wpm", typingWpm).put("saved_minutes", savedMinutes)
            .put("top_apps", JSONArray().apply { topApps.forEach { put(JSONObject().put("app", it.app).put("words", it.words)) } })
            .put("last7", JSONArray().apply { last7.forEach { put(JSONObject().put("date", it.date).put("words", it.words)) } })
            .put("since", since ?: JSONObject.NULL)
    }

    /**
     * `(current, longest)` run of consecutive [days] (any order, duplicates allowed). The current
     * run ends [today], or yesterday when today has nothing yet; days after today (clock moved
     * back) don't extend it.
     */
    fun streaks(days: Collection<Long>, today: Long): Pair<Int, Int> {
        val d = days.toSortedSet()
        var longest = 0; var run = 0
        var prev: Long? = null
        for (day in d) {
            run = if (prev != null && prev + 1 == day) run + 1 else 1
            longest = maxOf(longest, run)
            prev = day
        }
        var end = if (today in d) today else today - 1
        var current = 0
        while (end in d) { current++; end-- }
        return current to longest
    }
}

/**
 * Insights counts on disk (`files/insights.json`, the same shape as the Windows file): per local
 * day, words / recorded ms / dictations / words per app. Never text. Kept apart from
 * [HistoryStore], so deleting History or retention "Never" doesn't touch it.
 *
 * The first use folds the dictations already in History in, exactly once: [Aggregates.backfilled]
 * is set even when reading History fails, since a retry could count rows recorded live since.
 * Every operation runs that check first under the store lock, so a dictation recorded before the
 * page was ever opened can't be counted twice (the shell records here before adding the History row).
 */
class InsightsStore(
    private val file: File,
    private val zone: () -> TimeZone = TimeZone::getDefault,
    private val historyRows: () -> List<Row>,
) {
    /** What the backfill needs from one History row. */
    data class Row(val createdAt: Long, val appId: String?, val text: String, val audioMs: Long)

    private var cache: Insights.Aggregates? = null

    /** Local epoch day of [tsMs] in the device time zone (DST as it was at that instant). */
    fun localDay(tsMs: Long): Long = Insights.dayOf(tsMs, zone().getOffset(tsMs) / 60_000)

    @Synchronized
    fun load(): Insights.Aggregates {
        cache?.let { return it }
        var a = runCatching { file.takeIf { it.exists() }?.readText()?.let(Insights.Aggregates::fromJson) }.getOrNull()
            ?: Insights.Aggregates()
        if (!a.backfilled) {
            val rows = runCatching { historyRows() }.getOrDefault(emptyList())
            for (r in rows) a = a.add(localDay(r.createdAt), Insights.words(r.text).toLong(), r.audioMs, r.appId.orEmpty())
            a = a.copy(backfilled = true)
            cache = a
            runCatching { write(a) }
        }
        cache = a
        return a
    }

    /** Count one committed dictation; false when it had no words. */
    @Synchronized
    fun record(atMs: Long, text: String, audioMs: Long, appId: String?): Boolean {
        val words = Insights.words(text).toLong()
        val before = load()
        if (words <= 0) return false
        set(before.add(localDay(atMs), words, audioMs, appId.orEmpty()))
        return true
    }

    @Synchronized
    fun reset() { set(load().reset()) }

    /** Backup restore: these counts replace the stored ones (marked backfilled, so History is not folded in again). */
    @Synchronized
    fun replace(a: Insights.Aggregates) { set(a.copy(backfilled = true)) }

    fun summary(nowMs: Long = System.currentTimeMillis()): Insights.Summary =
        load().summary(localDay(nowMs), Insights.DEFAULT_TYPING_WPM)

    private fun set(a: Insights.Aggregates) {
        cache = a
        write(a)
    }

    private fun write(a: Insights.Aggregates) {
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, file.name + ".tmp")
        tmp.writeText(a.toJson().toString())
        if (!tmp.renameTo(file)) error("couldn't save ${file.name}")
    }
}
