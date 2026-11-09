package dev.murmur.health

/** `Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES` parsing (colon-separated flattened ComponentNames). */
object A11ySetting {
    fun isEnabled(enabledServices: String?, pkg: String, cls: String): Boolean {
        if (enabledServices.isNullOrBlank()) return false
        val short = if (cls.startsWith("$pkg.")) cls.removePrefix(pkg) else null
        return enabledServices.split(':').any { entry ->
            val parts = entry.trim().split('/')
            if (parts.size != 2 || !parts[0].equals(pkg, ignoreCase = true)) return@any false
            parts[1].equals(cls, ignoreCase = true) || (short != null && parts[1].equals(short, ignoreCase = true))
        }
    }
}

/**
 * Ghost-service detection: the toggle
 * says ON but the binding is dead, which OEM task killers cause. Two shapes:
 * - enabled in Settings, but `onServiceConnected` never ran within [CONNECT_GRACE_MS] of
 *   process start;
 * - connected, but the service's own 15 s health tick (a `getWindows()` probe) has not
 *   succeeded for [HEARTBEAT_STALE_MS] while the screen is on, **and** a live probe made
 *   now ([Input.liveProbe]) fails too.
 * The heartbeat alone is not proof: its timer runs on `uptimeMillis` (stops while the CPU
 * sleeps) but is compared with `elapsedRealtime`, so right after a wake the beat looks
 * minutes old although the service is bound and idle (seen on a Galaxy S25+: GHOST, then OK
 * 0.2-1 s later when the overdue tick fired). [Monitor] then debounces what is left.
 * The fix offered to the user: turn the service off and on again.
 */
object AccessibilityHealth {
    const val CONNECT_GRACE_MS = 15_000L
    const val HEARTBEAT_STALE_MS = 60_000L
    const val HEARTBEAT_INTERVAL_MS = 15_000L
    /** A ghost is reported only after two consecutive ghost readings at least this far apart. */
    const val GHOST_CONFIRM_MS = 10_000L

    enum class Status { OK, DISABLED, STARTING, GHOST }

    data class Input(
        val enabledInSettings: Boolean,
        val connected: Boolean,
        val processStartAt: Long,
        val connectedAt: Long?,
        val lastHeartbeatAt: Long?,
        val screenOn: Boolean,
        val now: Long,
        /** Asks the bound service directly (binder round trip); called only when the heartbeat is stale. Throwing = failed. */
        val liveProbe: (() -> Boolean)? = null,
    )

    fun evaluate(i: Input): Status {
        if (!i.enabledInSettings) return Status.DISABLED
        if (!i.connected) return if (i.now - i.processStartAt < CONNECT_GRACE_MS) Status.STARTING else Status.GHOST
        if (!i.screenOn) return Status.OK
        val last = i.lastHeartbeatAt ?: i.connectedAt ?: i.processStartAt
        if (i.now - last <= HEARTBEAT_STALE_MS) return Status.OK
        val alive = i.liveProbe?.let { p -> runCatching(p).getOrDefault(false) } ?: false
        return if (alive) Status.OK else Status.GHOST
    }

    /**
     * Debounce over [evaluate]: a single GHOST reading is never surfaced (no "not responding"
     * flash, no notification re-post, no Repair bubble). GHOST is reported once two
     * consecutive readings, the first and the last at least [confirmMs] apart, both say so;
     * anything else is reported at once. An unconfirmed ghost reads as the last reported status
     * ([Status.STARTING] before any).
     */
    class Monitor(private val confirmMs: Long = GHOST_CONFIRM_MS) {
        private var reported: Status? = null
        private var ghostSince: Long? = null
        private var ghostReadings = 0

        @Synchronized fun update(raw: Status, now: Long): Status {
            if (raw != Status.GHOST) {
                ghostSince = null; ghostReadings = 0; reported = raw
                return raw
            }
            val since = ghostSince ?: now.also { ghostSince = it }
            ghostReadings++
            if (ghostReadings >= 2 && now - since >= confirmMs) reported = Status.GHOST
            return reported ?: Status.STARTING
        }
    }

    /** MurmurService: re-post the notification / Repair state only when the status changed. */
    class ChangeGate(private var last: Status = Status.OK) {
        fun changed(s: Status): Boolean = (s != last).also { last = s }
    }
}

/** `MainServiceWatchdog`: respawn MurmurService on an unexpected stop, at most every 5 s. */
class ServiceWatchdog(private val minSpacingMs: Long = 5_000L) {
    private var lastRestartAt = Long.MIN_VALUE / 2

    fun shouldRestart(wanted: Boolean, running: Boolean, now: Long): Boolean {
        if (!wanted || running) return false
        if (now - lastRestartAt < minSpacingMs) return false
        lastRestartAt = now
        return true
    }
}

/**
 * OEM background-kill guidance (Xiaomi, Samsung, Huawei …).
 * [intents] are best-effort deep links into the vendor's autostart / battery screens;
 * callers try each and fall back to the app's details page.
 */
data class OemGuidance(val title: String, val steps: String, val intents: List<Component>, val extraWarning: String?) {
    data class Component(val packageName: String, val className: String)

    companion object {
        private val GENERIC = OemGuidance(
            "Android",
            "Keep battery optimisation off for Murmur (above). Nothing else is needed on this phone.",
            emptyList(), null,
        )

        fun forManufacturer(manufacturer: String?): OemGuidance = when (manufacturer?.trim()?.lowercase()) {
            "xiaomi", "redmi", "poco" -> OemGuidance(
                "Xiaomi / Redmi / POCO (MIUI, HyperOS)",
                "Security app → Permissions → Autostart → turn on Murmur. Then Settings → Apps → Murmur → " +
                    "Battery saver → No restrictions. In Recents, pull Murmur down to lock it.",
                listOf(
                    Component("com.miui.securitycenter", "com.miui.permcenter.autostart.AutoStartManagementActivity"),
                    Component("com.miui.powerkeeper", "com.miui.powerkeeper.ui.HiddenAppsConfigActivity"),
                ),
                "MIUI/HyperOS turns accessibility services off when it kills an app without Autostart.",
            )
            "samsung" -> OemGuidance(
                "Samsung (One UI)",
                "Settings → Battery → Background usage limits → Never sleeping apps → add Murmur. " +
                    "Also make sure Murmur is not in Sleeping apps or Deep sleeping apps.",
                listOf(Component("com.samsung.android.lool", "com.samsung.android.sm.battery.ui.BatteryActivity")),
                null,
            )
            "huawei", "honor" -> OemGuidance(
                "Huawei / Honor (EMUI, MagicOS)",
                "Settings → Battery → App launch → Murmur → switch off \"Manage automatically\" and turn on " +
                    "Auto-launch, Secondary launch and Run in background.",
                listOf(
                    Component("com.huawei.systemmanager", "com.huawei.systemmanager.startupmgr.ui.StartupNormalAppListActivity"),
                    Component("com.huawei.systemmanager", "com.huawei.systemmanager.optimize.process.ProtectActivity"),
                ),
                "EMUI closes background apps aggressively; without App launch the bubble disappears after the screen locks.",
            )
            "oppo", "realme", "oneplus" -> OemGuidance(
                "OPPO / realme / OnePlus (ColorOS, OxygenOS)",
                "Settings → Apps → Murmur → Battery usage → allow Auto launch and Background activity. " +
                    "In Recents, lock Murmur.",
                listOf(
                    Component("com.coloros.safecenter", "com.coloros.safecenter.permission.startup.StartupAppListActivity"),
                    Component("com.coloros.safecenter", "com.coloros.safecenter.startupapp.StartupAppListActivity"),
                    Component("com.oplus.battery", "com.oplus.powermanager.fuelgaue.PowerUsageModelActivity"),
                ),
                null,
            )
            "vivo", "iqoo" -> OemGuidance(
                "vivo / iQOO (Funtouch, OriginOS)",
                "i Manager → App manager → Autostart manager → turn on Murmur. Settings → Battery → " +
                    "Background power consumption → allow Murmur.",
                listOf(
                    Component("com.vivo.permissionmanager", "com.vivo.permissionmanager.activity.BgStartUpManagerActivity"),
                    Component("com.iqoo.secure", "com.iqoo.secure.ui.phoneoptimize.BgStartUpManager"),
                ),
                null,
            )
            else -> GENERIC
        }
    }
}
