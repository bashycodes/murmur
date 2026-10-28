package dev.flowclone.data

import java.text.NumberFormat
import java.time.LocalDate
import java.time.format.TextStyle
import java.util.Locale
import kotlin.math.roundToLong

/** Wording of the Insights page (ui/InsightsActivity), kept pure for the JVM tests. */
object InsightsPage {
    fun number(n: Long, locale: Locale = Locale.getDefault()): String = NumberFormat.getIntegerInstance(locale).format(n)

    fun days(n: Int) = if (n == 1) "1 day" else "$n days"

    fun dictations(n: Long, locale: Locale = Locale.getDefault()) = if (n == 1L) "1 dictation" else "${number(n, locale)} dictations"

    fun wpm(avg: Double?): String = avg?.let { "${it.roundToLong()} words/min" } ?: "No recordings yet"

    fun saved(minutes: Double): String {
        if (minutes <= 0.0) return "None yet"
        if (minutes < 1.0) return "Under a minute"
        val m = minutes.toLong()
        return when {
            m < 60 -> "$m min"
            m % 60 == 0L -> "${m / 60} h"
            else -> "${m / 60} h ${m % 60} min"
        }
    }

    fun savedHint(typingWpm: Double) = "Compared with typing at ${typingWpm.roundToLong()} words/min"

    /** Short weekday per bar, the last one "Today". */
    fun chartLabels(last7: List<Insights.DayWords>, locale: Locale = Locale.getDefault()): List<String> =
        last7.mapIndexed { i, d ->
            if (i == last7.lastIndex) "Today"
            else runCatching { LocalDate.parse(d.date).dayOfWeek.getDisplayName(TextStyle.SHORT, locale) }.getOrDefault(d.date)
        }

    fun chartDescription(last7: List<Insights.DayWords>, locale: Locale = Locale.getDefault()): String =
        "Words per day, last 7 days: " + chartLabels(last7, locale).zip(last7).joinToString(", ") { (l, d) -> "$l ${number(d.words, locale)}" }

    /** [label] = the installed app's name (null when unknown / not visible to us). */
    fun appName(app: String, label: (String) -> String?): String =
        if (app.isBlank()) "Unknown app" else label(app)?.takeIf { it.isNotBlank() } ?: app
}
