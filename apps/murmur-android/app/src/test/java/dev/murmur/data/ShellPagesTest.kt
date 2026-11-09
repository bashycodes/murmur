package dev.murmur.data

import dev.murmur.core.CleanupLevel
import dev.murmur.data.DevSettings.CleanupRoute
import dev.murmur.data.ShellPages.Dot
import dev.murmur.data.ShellPages.Extra
import dev.murmur.data.ShellPages.RouteKind
import dev.murmur.data.ShellPages.Tab
import dev.murmur.health.AccessibilityHealth.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.ZoneOffset

/** App shell: bottom-nav model, drawer lines (stats, engines, "N to fix", extras) and Home card lines. */
class ShellPagesTest {
    private companion object { const val NOW = 1_792_540_800_000L }

    @Test fun bottomNavHasTheDesignsFourTabsInOrder() {
        assertEquals(listOf("Home", "Dictionary", "Style", "Snippets"), Tab.entries.map { it.label })
        assertEquals(Tab.STYLE, ShellPages.tabOf("STYLE"))
        assertEquals(Tab.HOME, ShellPages.tabOf(null))
        assertEquals(Tab.HOME, ShellPages.tabOf("gone"))
    }

    @Test fun compactNumbersLikeTheDrawerStats() {
        val cases = mapOf(
            0L to "0", 7L to "7", 999L to "999", 1_000L to "1K", 1_049L to "1K", 1_050L to "1.1K", 9_400L to "9.4K", 9_449L to "9.4K",
            9_960L to "10K", 12_345L to "12K", 999_499L to "999K", 999_950L to "1M", 1_250_000L to "1.3M", 42_000_000L to "42M",
            3_000_000_000L to "3B", -5L to "0",
        )
        for ((n, want) in cases) assertEquals("$n", want, ShellPages.compact(n))
    }

    private fun summary(words: Long, wpm: Double?, streak: Int) = Insights.Summary(
        totalWords = words, weekWords = 0, dictations = 0, totalMs = 0, avgWpm = wpm, currentStreak = streak, longestStreak = streak,
        typingWpm = 40.0, savedMinutes = 0.0, topApps = emptyList(), last7 = emptyList(), since = null,
    )

    @Test fun drawerStatsAreWordsWpmAndStreak() {
        assertEquals(listOf("9.4K" to "words", "121" to "wpm", "6" to "day streak"), ShellPages.stats(summary(9_400, 120.6, 6)))
        assertEquals(listOf("0" to "words", "0" to "wpm", "0" to "day streak"), ShellPages.stats(summary(0, null, 0)))
    }

    private val allOk = SetupChecklist.State(mic = true, overlay = true, a11y = Status.OK, batteryExempt = true, notifications = true)

    @Test fun permissionsBadgeCountsStepsThatCanBeFixed() {
        assertNull(ShellPages.toFix(SetupChecklist.rows(allOk)))
        assertEquals("1 to fix", ShellPages.toFix(SetupChecklist.rows(allOk.copy(a11y = Status.DISABLED))))
        assertEquals("3 to fix", ShellPages.toFix(SetupChecklist.rows(allOk.copy(mic = false, batteryExempt = false, notifications = false))))
        // Connecting has no action: nothing to fix (yet).
        assertNull(ShellPages.toFix(SetupChecklist.rows(allOk.copy(a11y = Status.STARTING))))
    }

    @Test fun permissionsProgressAndFeaturedStep() {
        val rows = SetupChecklist.rows(allOk.copy(a11y = Status.DISABLED, batteryExempt = false))
        assertEquals(0.6f, ShellPages.doneFraction(rows), 0.001f)
        assertEquals(SetupChecklist.Item.ACCESSIBILITY, ShellPages.featured(rows)?.item)
        // Only a recommended step missing: no featured card.
        assertNull(ShellPages.featured(SetupChecklist.rows(allOk.copy(batteryExempt = false))))
        assertEquals(1f, ShellPages.doneFraction(SetupChecklist.rows(allOk)), 0f)
    }

    @Test fun calloutSplitsTheBannerIntoLeadAndText() {
        assertEquals(
            "Dictation can't start yet." to "Allow Accessibility service below.",
            ShellPages.callout("Dictation can't start yet: allow Accessibility service below."),
        )
        assertEquals("" to "Plain text", ShellPages.callout("Plain text"))
    }

    @Test fun engineLinesAndDots() {
        assertEquals(ShellPages.EngineLine("Speech to text", "Server first → Phone", Dot.OK), ShellPages.speechLine("Server first → Phone"))
        assertEquals(Dot.CLOUD, ShellPages.speechLine("Cloud (OpenRouter) first → Phone").dot)
        assertEquals(Dot.BAD, ShellPages.speechLine("Dictation disabled").dot)
        assertEquals("Speech to text: Phone only", ShellPages.speechLine("Phone only").description)

        val base = DevSettings.defaults(BuildCaps.ALL)
        val ai = Settings(llmEnabled = true)
        val cloudFirst = OpenRouterPage.enableCleanup(base, NOW).copy(cleanupOrder = listOf(CleanupRoute.CLOUD, CleanupRoute.LOCAL, CleanupRoute.SERVER))
        val cloud = ShellPages.cleanupLine(ai, cloudFirst, phoneAi = true)
        assertEquals("OpenRouter (llama-3.3-70b-instruct), then this phone" to Dot.CLOUD, cloud.text to cloud.dot)
        // The dot is the first engine's: the phone first, the cloud only as a fallback.
        val phoneFirst = ShellPages.cleanupLine(ai, OpenRouterPage.enableCleanup(base, NOW), phoneAi = true)
        assertEquals("This phone, then OpenRouter (llama-3.3-70b-instruct)" to Dot.OK, phoneFirst.text to phoneFirst.dot)
        assertEquals("This phone" to Dot.OK, ShellPages.cleanupLine(ai, base, phoneAi = true).let { it.text to it.dot })
        // remote builds have no on-device LLM, whatever the stored switch says.
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(ai, base, phoneAi = false).text)
        assertEquals("Off" to Dot.OFF, ShellPages.cleanupLine(Settings(spokenCleanup = false), base, phoneAi = false).let { it.text to it.dot })
        // The phone's model is off for the router when its cleanup switch is off.
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(ai, base.withCleanup(CleanupRoute.LOCAL, false), phoneAi = true).text)
    }

    private val serverOnly = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10").withCleanup(CleanupRoute.LOCAL, false)

    @Test fun serverOnlyCleanupShowsAsOn() {
        // Owner report: AI cleanup only on your server, everything else off -> the drawer said "Off".
        val line = ShellPages.cleanupLine(Settings(llmEnabled = false), serverOnly, phoneAi = false)
        assertEquals("Your server" to Dot.OK, line.text to line.dot)
        assertEquals("Your server", ShellPages.cleanupLine(Settings(), serverOnly, phoneAi = false, serverLlm = true).text)
        // A server that is not set up, or known to have no AI model, cleans nothing.
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(Settings(), serverOnly.copy(host = ""), phoneAi = false).text)
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(Settings(), serverOnly, phoneAi = false, serverLlm = false).text)
    }

    @Test fun storedLevelNoneStillCleansOnTheServer() {
        // NONE is never picked since D27: with an engine off the phone it runs as LIGHT, so the line stays on
        // (the LLM step only skips when the session's level is NONE, which then reads Off).
        val s = Settings(llmEnabled = false, cleanup = CleanupLevel.NONE)
        assertEquals(CleanupLevel.LIGHT, s.cleanupFor(serverOnly.offPhoneCleanup))
        assertEquals("Your server", ShellPages.cleanupLine(s, serverOnly, phoneAi = false).text)
        // phoneAi claimed without the phone's AI on: the session level is NONE, so nothing is named.
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(s, DevSettings.defaults(BuildCaps.ALL), phoneAi = true).text)
    }

    @Test fun customServerOnlyCleanupShowsAsOnAndNamesIt() {
        FakeCustomAddress.install()
        val base = DevSettings.defaults(BuildCaps.ALL)
        val custom = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "gpt-4o-mini")
        val s = OpenRouterPage.enableCleanup(base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom)), NOW)
            .withCleanup(CleanupRoute.LOCAL, false).withCleanup(CleanupRoute.SERVER, false)
        val line = ShellPages.cleanupLine(Settings(), s, phoneAi = false)
        assertEquals("Custom server (10.0.0.5)" to Dot.CLOUD, line.text to line.dot)
        // Not usable (no model): sends nothing, so it is off, never "OpenRouter".
        val broken = OpenRouterPage.withCustom(s) { copy(model = "") }
        assertEquals("Off · formatting rules only", ShellPages.cleanupLine(Settings(), broken, phoneAi = false).text)
    }

    @Test fun drawerExtrasOnlyWhereTheyExist() {
        val all = ShellPages.extras(local = true, developer = true)
        assertEquals(Extra.entries.toList(), all)
        val remote = ShellPages.extras(local = false, developer = false)
        assertEquals(Extra.entries.filter { it != Extra.SPEECH_MODEL && it != Extra.CALLS && it != Extra.DEVELOPER }, remote)
        assertEquals("Murmur 0.6.0", ShellPages.versionLine("0.6.0"))
    }

    private fun row(route: String?, final: String = "Hi", asr: String = "hi") =
        HistoryRow(1, 0, "app", "FORMATTED", asr, final, true, 0, route = route)

    @Test fun homeCardRouteAndText() {
        assertEquals(RouteKind.PHONE, ShellPages.route(row("local")))
        assertEquals(RouteKind.SERVER, ShellPages.route(row("remote")))
        assertEquals(RouteKind.CLOUD, ShellPages.route(row("cloud")))
        assertNull(ShellPages.route(row(null)))
        assertEquals("Hi", ShellPages.cardText(row(null)))
        assertEquals("hi", ShellPages.cardText(row(null, final = "")))
        assertEquals("(no speech)", ShellPages.cardText(row(null, final = "  ", asr = "")))
    }

    @Test fun homeCardTimeIsTheClockTodayAndTheDateBefore() {
        val zone: ZoneId = ZoneOffset.UTC
        fun at(y: Int, mo: Int, d: Int, h: Int) = LocalDateTime.of(y, mo, d, h, 0).atZone(zone).toInstant().toEpochMilli()
        val now = at(2026, 10, 30, 12)
        assertEquals("9:37am", ShellPages.cardTime(at(2026, 10, 30, 9), now, "9:37am", zone))
        assertEquals("Oct 28, 6:55pm", ShellPages.cardTime(at(2026, 10, 28, 18), now, "6:55pm", zone))
        assertEquals("Dec 31, 2025, 18:55", ShellPages.cardTime(at(2025, 12, 31, 18), now, "18:55", zone))
    }
}
