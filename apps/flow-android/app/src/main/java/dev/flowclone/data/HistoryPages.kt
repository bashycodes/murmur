package dev.flowclone.data

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit
import java.util.Locale

/**
 * History (ui/HistoryActivity) in the Settings look: a list grouped by day (one row per
 * dictation: first line, time + target app, engine-outcome badge) and a detail page. Search and
 * raw-vs-formatted text stay in [HistoryView]; this only formats.
 */
object HistoryPages {
    data class Group(val label: String, val rows: List<HistoryRow>)

    /** Lists this long get a search field (same as Dictionary & snippets). */
    const val SEARCH_FROM = DictionaryPages.SEARCH_FROM

    private val WEEKDAY = DateTimeFormatter.ofPattern("EEEE", Locale.ENGLISH)
    private val THIS_YEAR = DateTimeFormatter.ofPattern("EEE, MMM d", Locale.ENGLISH)
    private val OTHER_YEAR = DateTimeFormatter.ofPattern("EEE, MMM d, yyyy", Locale.ENGLISH)

    fun dayLabel(day: LocalDate, today: LocalDate): String {
        val ago = ChronoUnit.DAYS.between(day, today)
        return when {
            ago == 0L -> "Today"
            ago == 1L -> "Yesterday"
            ago in 2..6 -> day.format(WEEKDAY)
            day.year == today.year -> day.format(THIS_YEAR)
            else -> day.format(OTHER_YEAR)
        }
    }

    /** [rows] newest first (as [HistoryStore.recent] returns them); keeps that order. */
    fun groupByDay(rows: List<HistoryRow>, now: Long, zone: ZoneId = ZoneId.systemDefault()): List<Group> {
        val today = Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
        val groups = LinkedHashMap<LocalDate, MutableList<HistoryRow>>()
        for (r in rows) groups.getOrPut(Instant.ofEpochMilli(r.createdAt).atZone(zone).toLocalDate()) { ArrayList() } += r
        return groups.map { (day, list) -> Group(dayLabel(day, today), list) }
    }

    /** Row title: the first non-blank line of what the row shows. */
    fun title(r: HistoryRow, raw: Boolean): String =
        HistoryView.text(r, raw).lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() } ?: "(no speech)"

    fun app(r: HistoryRow): String = r.appId?.takeIf { it.isNotBlank() } ?: "Unknown app"

    /** [time] = the row's time as the phone formats it (12 / 24 h). */
    fun meta(r: HistoryRow, time: String) = "$time · ${app(r)}"

    /** Where it was transcribed, from [HistoryRow.route] (router engine); null when unknown. */
    fun routeLabel(r: HistoryRow): String? = when (r.route) {
        "local" -> "Phone"
        "remote" -> "Server"
        "cloud" -> "Cloud"
        else -> null
    }

    /** The engine outcome badge, prefixed with the route when the row has it: "Server · AI cleaned". */
    fun badge(r: HistoryRow): String = listOfNotNull(routeLabel(r), HistoryView.label(r)).joinToString(" · ")

    /**
     * What Copy puts on the clipboard: the saved text ([CommitResult.historyText] at save time,
     * line breaks included). The "Dictation not inserted" notification copies the same text.
     */
    fun copyText(r: HistoryRow): String = HistoryView.text(r, raw = false)

    private fun hasText(r: HistoryRow) = r.finalText.isNotBlank() || r.asrText.isNotBlank()

    /**
     * Dictated but never reached the field (switched fields, protected target, paste failed):
     * worth recovering. [HistoryRow.inserted] decides (rows saved before v3 have no outcome);
     * [reason] says why from [HistoryRow.insertOutcome].
     */
    fun notInserted(r: HistoryRow) = !r.inserted && hasText(r) &&
        runCatching { dev.flowclone.core.CommitStatus.valueOf(r.status) }.getOrNull().let {
            it == dev.flowclone.core.CommitStatus.FORMATTED || it == dev.flowclone.core.CommitStatus.RAW_TRANSCRIPT
        }

    fun flag(r: HistoryRow): String? = if (notInserted(r)) "Not inserted — tap to copy" else null

    /** Why it was not inserted ([HistoryRow.insertOutcome]); null when inserted or unknown. */
    fun reason(r: HistoryRow): String? = if (r.inserted) null else when (r.insertOutcome) {
        dev.flowclone.debug.InsertOutcome.TARGET_CHANGED -> "You switched apps"
        dev.flowclone.debug.InsertOutcome.TARGET_PROTECTED -> "Protected field"
        dev.flowclone.debug.InsertOutcome.PASTE_FAILED -> "Couldn't paste"
        else -> null
    }

    /** Per-stage timings (INSTALL.md device latency checklist reads them on the detail page). */
    fun timings(r: HistoryRow): String = buildList {
        if (r.audioMs > 0) add("${r.audioMs / 100 / 10.0} s audio")
        if (r.asrMs > 0) add("asr ${r.asrMs} ms")
        if (r.llmMs > 0) add("llm ${r.llmMs} ms")
        if (r.totalMs > 0) add("total ${r.totalMs} ms")
        LlmOutcome.label(r.fallbackReason)?.let { add(it) }
    }.joinToString(" · ")

    fun summary(count: Int): String = when (count) {
        0 -> "No dictations saved"
        1 -> "1 dictation"
        else -> "$count dictations"
    } + " · audio is never stored"

    data class Empty(val title: String, val hint: String)

    fun empty(retention: Retention, query: String, total: Int): Empty {
        val q = query.trim()
        if (total > 0 && q.isNotEmpty()) return Empty("Nothing matches “$q”", "Search looks at the inserted and the raw text.")
        return if (retention == Retention.NEVER) Empty("No dictations yet", "History is set to Never save. Change it under Keep dictations.")
        else Empty("No dictations yet", "Each dictation appears here, newest first. It stays on this phone.")
    }
}
