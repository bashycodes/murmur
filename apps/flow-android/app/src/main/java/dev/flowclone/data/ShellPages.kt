package dev.flowclone.data

import dev.flowclone.core.CleanupLevel
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToLong

/**
 * The app shell (ui/MainActivity, docs/design/app-design/AndroidHome / AndroidNav / AndroidDrawer):
 * bottom-navigation model, the Home tab's card lines and the drawer's lines. Formatting only; the
 * activity reads the stores and hands the values in.
 */
object ShellPages {
    /** Bottom navigation, in AndroidNav.dc.html order. */
    enum class Tab(val label: String) { HOME("Home"), DICTIONARY("Dictionary"), STYLE("Style"), SNIPPETS("Snippets") }

    /** The app's name: Home top bar, drawer title. */
    const val APP_NAME = "Murmur"

    /** Help in the drawer opens the project page. */
    const val HELP_URL = "https://github.com/bashycodes/murmur"

    /** Home shows this many recent dictations; History (drawer) has them all. */
    const val HOME_LIMIT = 100

    fun tabOf(name: String?): Tab = Tab.entries.firstOrNull { it.name == name } ?: Tab.HOME

    // ---- numbers ----

    /** Drawer stat numbers: 950, 1K, 9.4K, 12K, 1.3M (one decimal below 10, none above). */
    fun compact(n: Long): String {
        if (n < 1_000) return n.coerceAtLeast(0).toString()
        val units = listOf(1_000L to "K", 1_000_000L to "M", 1_000_000_000L to "B")
        for ((i, u) in units.withIndex()) {
            val (div, suffix) = u
            val v = n.toDouble() / div
            val text = if (v < 9.95) {
                val tenth = (v * 10).roundToLong()
                if (tenth % 10 == 0L) "${tenth / 10}" else "${tenth / 10}.${tenth % 10}"
            } else v.roundToLong().toString()
            // 999 950 rounds to "1000K": that is the next unit.
            val rolled = text.toDoubleOrNull()?.let { it >= 1_000 } == true && i < units.size - 1
            if (!rolled) return text + suffix
        }
        return n.toString()
    }

    /** The drawer's three stats (words / wpm / day streak), as the design's stat card shows them. */
    fun stats(s: Insights.Summary): List<Pair<String, String>> = listOf(
        compact(s.totalWords) to "words",
        (s.avgWpm?.roundToLong() ?: 0L).toString() to "wpm",
        s.currentStreak.toString() to "day streak",
    )

    // ---- drawer ----

    /** Permissions row badge: "2 to fix" (missing steps that have an action), null when all is done. */
    fun toFix(rows: List<SetupChecklist.Row>): String? {
        val n = rows.count { !it.ok && it.action != null }
        return if (n == 0) null else "$n to fix"
    }

    /** Permissions progress bar: share of rows done (0..1). */
    fun doneFraction(rows: List<SetupChecklist.Row>): Float = if (rows.isEmpty()) 1f else rows.count { it.ok }.toFloat() / rows.size

    /** The step the Permissions page puts first, in its own card with a filled button: the first missing required one. */
    fun featured(rows: List<SetupChecklist.Row>): SetupChecklist.Row? =
        rows.firstOrNull { !it.ok && it.action != null && it.need == SetupChecklist.Need.REQUIRED }

    fun versionLine(version: String) = "$APP_NAME $version"

    /** Status dot of an engine line: green = on the phone / your server, glow = cloud, grey = off, red = disabled. */
    enum class Dot { OK, CLOUD, OFF, BAD }

    data class EngineLine(val label: String, val text: String, val dot: Dot) {
        /** One TalkBack phrase. */
        val description: String get() = "$label: $text"
    }

    /** [routing] = [DevSummary.routing] ("Server first → Phone", "Dictation disabled"). */
    fun speechLine(routing: String) = EngineLine(
        "Speech to text", routing,
        when {
            routing == "Dictation disabled" -> Dot.BAD
            routing.contains("Cloud") -> Dot.CLOUD
            else -> Dot.OK
        },
    )

    /**
     * The engines AI cleanup will use ([CleanupEngines.planned]; [d] = the effective settings,
     * [phoneAi] = the on-device AI is on in a build that has it, [serverLlm] from `cleanup_status`),
     * "Off" when the dictation's LLM step does not run; then the formatting rules or off.
     */
    fun cleanupLine(s: Settings, d: DevSettings, phoneAi: Boolean, serverLlm: Boolean? = null): EngineLine {
        val plan = CleanupEngines.planned(d, phoneAi, serverLlm)
        val runs = s.cleanupFor(d.offPhoneCleanup) != CleanupLevel.NONE
        val engines = if (runs) CleanupEngines.engineLine(d, plan) else null
        val cloudFirst = plan.firstOrNull() == DevSettings.CleanupRoute.CLOUD
        return when {
            engines != null -> EngineLine("AI cleanup", engines, if (cloudFirst) Dot.CLOUD else Dot.OK)
            s.spokenCleanup -> EngineLine("AI cleanup", "Off · formatting rules only", Dot.OFF)
            else -> EngineLine("AI cleanup", "Off", Dot.OFF)
        }
    }

    /**
     * Drawer features the design has no slot for (they lived on the old main screen), in the
     * second card. [SPEECH_MODEL] and [CALLS] (D46) only where the build transcribes on the phone, [DEVELOPER] only
     * once Developer settings are unlocked.
     */
    enum class Extra(val title: String, val subtitle: String) {
        NOTES("Notes", "Saved notes to dictate into"),
        QUICK_NOTE("Quick note", "One note to dictate into and copy out"),
        INSIGHTS("Insights", "Words, speed, streaks and top apps"),
        HISTORY("History", "Every saved dictation"),
        CALLS("Calls", "Transcribe a call recording"),
        START_BUBBLE("Start / show bubble", "Shows the dictation bubble over other apps"),
        STOP_BUBBLE("Stop bubble", "Hides it until you start it again"),
        SPEECH_MODEL("Speech model", "Import an on-device model"),
        DEVELOPER("Developer settings", "Engines, server"),
    }

    fun extras(local: Boolean, developer: Boolean): List<Extra> = Extra.entries.filter {
        when (it) {
            Extra.SPEECH_MODEL, Extra.CALLS -> local
            Extra.DEVELOPER -> developer
            else -> true
        }
    }

    // ---- Home ----

    /** Where a dictation ran, for its route tag ([HistoryRow.route]); null = unknown (no tag). */
    enum class RouteKind { PHONE, SERVER, CLOUD }

    fun route(r: HistoryRow): RouteKind? = when (r.route) {
        "local" -> RouteKind.PHONE
        "remote" -> RouteKind.SERVER
        "cloud" -> RouteKind.CLOUD
        else -> null
    }

    /** The card's text: what was inserted (formatted), or "(no speech)". */
    fun cardText(r: HistoryRow): String = HistoryView.text(r, raw = false).trim().ifEmpty { "(no speech)" }

    private val MONTH_DAY = DateTimeFormatter.ofPattern("MMM d", Locale.ENGLISH)
    private val MONTH_DAY_YEAR = DateTimeFormatter.ofPattern("MMM d, yyyy", Locale.ENGLISH)

    /** Card time: "9:37am" today, "Oct 28, 6:55pm" before; [time] is the clock time as the phone formats it. */
    fun cardTime(createdAt: Long, now: Long, time: String, zone: ZoneId = ZoneId.systemDefault()): String {
        val day = Instant.ofEpochMilli(createdAt).atZone(zone).toLocalDate()
        val today = Instant.ofEpochMilli(now).atZone(zone).toLocalDate()
        return when {
            day == today -> time
            day.year == today.year -> "${day.format(MONTH_DAY)}, $time"
            else -> "${day.format(MONTH_DAY_YEAR)}, $time"
        }
    }

    /** Peach callout from a [SetupChecklist.Banner] text: bold lead sentence + the rest ("Dictation can't start yet." / "Allow …"). */
    fun callout(text: String): Pair<String, String> {
        val i = text.indexOf(": ")
        if (i < 0) return "" to text
        return text.substring(0, i) + "." to text.substring(i + 2).replaceFirstChar { it.uppercase() }
    }
}
