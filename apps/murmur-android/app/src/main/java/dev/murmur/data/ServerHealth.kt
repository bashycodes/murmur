package dev.murmur.data

import dev.murmur.core.HealthReport

/**
 * The process's last server health probe (one per app, [dev.murmur.MurmurApp.serverHealth]):
 * Server → Test connection and the probe after Save record it, the Settings pages read it, so
 * a test that just passed shows as "Connected · N ms" on return instead of "Not checked yet".
 *
 * A probe counts only for the server it reached: host, port, TLS, pin and token must match the
 * settings asked about, so testing unsaved values never colours the saved server's row.
 */
class ServerHealth {
    private class Entry(val key: String, val report: HealthReport, val atMs: Long)

    @Volatile private var last: Entry? = null

    /** [d] = the effective settings the probe used. */
    fun record(d: DevSettings, h: HealthReport, nowMs: Long) { last = Entry(key(d), h, nowMs) }

    /** The recorded probe of the server [d] names, when younger than [maxAgeMs]; else null. */
    fun fresh(d: DevSettings, nowMs: Long, maxAgeMs: Long = MAX_AGE_MS): HealthReport? =
        last?.takeIf { it.key == key(d) && nowMs - it.atMs in 0..maxAgeMs }?.report

    companion object {
        /** A probe younger than this is shown as is (else the Settings page probes once). */
        const val MAX_AGE_MS = 60_000L

        // The token only as its hash: the key never holds the secret itself.
        private fun key(d: DevSettings) = listOf(d.host, d.port, d.tls, d.tlsPinSha256, d.token.hashCode()).joinToString("|")
    }
}
