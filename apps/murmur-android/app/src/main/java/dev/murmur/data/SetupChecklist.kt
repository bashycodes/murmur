package dev.murmur.data

import dev.murmur.health.AccessibilityHealth

/**
 * Permissions page (ui/PermissionsActivity; drawer → Permissions), in onboarding order:
 * mic → overlay → accessibility (prominent disclosure, ghost check) → battery → notifications.
 * Each row shows its state ("Granted") or the action that fixes it ("Grant"); [banner] is the
 * problem line (Permissions callout, Home notice) while a step is missing.
 */
object SetupChecklist {
    enum class Item { MIC, OVERLAY, ACCESSIBILITY, BATTERY, NOTIFICATIONS }

    /** REQUIRED blocks dictation; RECOMMENDED keeps it alive; OPTIONAL is nice to have. */
    enum class Need { REQUIRED, RECOMMENDED, OPTIONAL }

    data class State(
        val mic: Boolean,
        val overlay: Boolean,
        val a11y: AccessibilityHealth.Status,
        val batteryExempt: Boolean,
        val notifications: Boolean,
        /** Developer → Keep last dictation audio: the mic text must not say "never stored". */
        val keepDebugAudio: Boolean = false,
    )

    /** [action] null = nothing to do (done, or connecting). */
    data class Row(val item: Item, val title: String, val subtitle: String, val state: String, val ok: Boolean, val action: String?, val need: Need)

    data class Banner(val text: String, val blocking: Boolean)

    const val A11Y_DISCLOSURE =
        "Murmur uses Accessibility to notice when your keyboard is open, show the bubble, read up to 500 " +
            "characters around the cursor so the transcript fits, and type the transcript into the focused field. " +
            "It never reads or types into password, PIN, number or phone fields, and stays hidden in banking apps. " +
            "Nothing it sees leaves the phone.\n\nIf the switch is greyed out: Settings → Apps → Murmur → ⋮ → Allow restricted settings."

    const val A11Y_GHOST =
        "Switched on but not responding. Android or the phone's battery manager stopped it without turning " +
            "the switch off. Turn Murmur OFF and then ON again in Accessibility settings. Then allow Battery " +
            "optimisation and follow Keep it running below so it stays alive."

    fun rows(s: State): List<Row> {
        val audio = "Audio is processed on the phone and never stored" +
            if (s.keepDebugAudio) " (except: Developer → Keep last dictation audio is on)." else "."
        return listOf(
            if (s.mic) Row(Item.MIC, "Microphone", "Hears you. $audio", "Granted", true, null, Need.REQUIRED)
            else Row(Item.MIC, "Microphone", "Needed to hear you. $audio", "Not granted", false, "Grant", Need.REQUIRED),
            if (s.overlay) Row(Item.OVERLAY, "Display over other apps", "Shows the dictation bubble", "Allowed", true, null, Need.REQUIRED)
            else Row(Item.OVERLAY, "Display over other apps", "Needed for the dictation bubble.", "Not allowed", false, "Allow", Need.REQUIRED),
            when (s.a11y) {
                AccessibilityHealth.Status.OK -> Row(Item.ACCESSIBILITY, "Accessibility service", "Types the transcript into the focused field", "On", true, null, Need.REQUIRED)
                AccessibilityHealth.Status.STARTING -> Row(Item.ACCESSIBILITY, "Accessibility service", "Turned on, connecting…", "Connecting…", false, null, Need.REQUIRED)
                AccessibilityHealth.Status.DISABLED -> Row(Item.ACCESSIBILITY, "Accessibility service", A11Y_DISCLOSURE, "Off", false, "Turn on", Need.REQUIRED)
                AccessibilityHealth.Status.GHOST -> Row(Item.ACCESSIBILITY, "Accessibility service", A11Y_GHOST, "Not responding", false, "Fix", Need.REQUIRED)
            },
            if (s.batteryExempt) Row(Item.BATTERY, "Battery optimisation", "Android won't stop Murmur in the background", "Unrestricted", true, null, Need.RECOMMENDED)
            else Row(
                Item.BATTERY, "Battery optimisation",
                "Recommended. Without it Android may stop Murmur in the background, and the bubble disappears until you reopen the app.",
                "Optimised", false, "Allow", Need.RECOMMENDED,
            ),
            if (s.notifications) Row(Item.NOTIFICATIONS, "Notifications", "Status, session limit and Accessibility alerts", "Allowed", true, null, Need.OPTIONAL)
            else Row(Item.NOTIFICATIONS, "Notifications", "Optional: status, session limit, and the alert when Accessibility needs fixing.", "Off", false, "Allow", Need.OPTIONAL),
        )
    }

    /** Missing steps that block dictation (a connecting service is not one). */
    private fun blocking(s: State) = rows(s).filter { !it.ok && it.need == Need.REQUIRED && it.action != null }

    /** Card header counter: "3 of 5 done" / "All 5 done". */
    fun progress(s: State): String {
        val r = rows(s)
        val done = r.count { it.ok }
        return if (done == r.size) "All ${r.size} done" else "$done of ${r.size} done"
    }

    /** While a required step is missing the SETUP card comes first, right under the banner. */
    fun setupFirst(s: State) = blocking(s).isNotEmpty()

    fun banner(s: State): Banner? {
        val b = blocking(s)
        if (b.isNotEmpty()) {
            if (b.size == 1 && s.a11y == AccessibilityHealth.Status.GHOST) {
                return Banner("Dictation stopped: the Accessibility service stopped responding. Fix it below.", true)
            }
            val names = b.map { it.title }
            val list = if (names.size == 1) names[0] else names.dropLast(1).joinToString(", ") + " and " + names.last()
            return Banner("Dictation can't start yet: allow $list below.", true)
        }
        val rec = rows(s).filter { !it.ok && it.need == Need.RECOMMENDED }
        if (rec.isNotEmpty()) return Banner("Recommended: ${rec.joinToString { it.title }}, so the bubble keeps working in the background.", false)
        return null
    }

    /** TalkBack announcements after returning from a system screen: "Microphone granted". */
    fun newlyDone(before: State?, now: State): List<String> {
        if (before == null) return emptyList()
        val was = rows(before).associateBy { it.item }
        return rows(now).filter { it.ok && was[it.item]?.ok == false }.map { "${it.title} ${it.state.lowercase()}" }
    }
}
