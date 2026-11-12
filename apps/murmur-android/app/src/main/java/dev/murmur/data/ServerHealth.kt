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
    /** A probe result and when it finished (wall clock, like [HealthReport.checkedAtMs]). */
    data class Probe(val report: HealthReport, val atMs: Long)

    private class Entry(val key: String, val probe: Probe)

    @Volatile private var last: Entry? = null

    /** [d] = the effective settings of the server that was probed. */
    fun record(d: DevSettings, h: HealthReport, nowMs: Long) { last = Entry(key(d), Probe(h, nowMs)) }

    /** The recorded probe of the server [d] names, when younger than [maxAgeMs]; else null. */
    fun fresh(d: DevSettings, nowMs: Long, maxAgeMs: Long = MAX_AGE_MS): Probe? =
        last?.takeIf { it.key == key(d) && isFresh(it.probe.atMs, nowMs, maxAgeMs) }?.probe

    companion object {
        /** A probe younger than this is shown as is (else the Settings page probes once). */
        const val MAX_AGE_MS = 60_000L

        /** [a] and [b] reach the same server (the fields a probe depends on). */
        fun sameServer(a: DevSettings, b: DevSettings) = key(a) == key(b)

        fun isFresh(atMs: Long?, nowMs: Long, maxAgeMs: Long = MAX_AGE_MS) = atMs != null && nowMs - atMs in 0..maxAgeMs

        /**
         * The newer of the shared probe ([shared]) and the router's own last health ([router], timed by
         * its [HealthReport.checkedAtMs]), so an old "Connected" never hides a newer failure, or the reverse.
         */
        fun newer(shared: Probe?, router: HealthReport?): Probe? {
            val r = router?.let { Probe(it, it.checkedAtMs ?: Long.MIN_VALUE) }
            return when {
                shared == null -> r
                r == null -> shared
                r.atMs > shared.atMs -> r
                else -> shared
            }
        }

        // The token only as its hash: the key never holds the secret itself.
        private fun key(d: DevSettings) = listOf(d.host, d.port, d.tls, d.tlsPinSha256, d.token.hashCode()).joinToString("|")
    }
}
