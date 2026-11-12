package dev.murmur.data

import dev.murmur.core.HealthReport
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ServerHealthTest {
    private val saved = DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = "tok_test")
    private val ok = HealthReport(ok = true)

    @Test fun aTestConnectionShowsOnTheSavedServerAtOnce() {
        // Owner report: right after a passing Test connection, Settings → Your server said "Not checked yet".
        val cache = ServerHealth()
        assertNull(cache.fresh(saved, 1_000))
        cache.record(saved, ok, 1_000)
        assertEquals(ServerHealth.Probe(ok, 1_000), cache.fresh(saved, 1_500))
        // Fields that do not change which server is reached keep the result (Save raises the connect timeout).
        assertEquals(ok, cache.fresh(saved.copy(connectTimeoutMs = saved.connectTimeoutMs * 2), 1_500)?.report)
    }

    @Test fun anotherServerOrAnOldProbeDoesNotCount() {
        val cache = ServerHealth()
        cache.record(saved, ok, 1_000)
        assertNull(cache.fresh(saved.copy(host = "100.64.0.11"), 1_500))
        assertNull(cache.fresh(saved.copy(port = saved.port + 1), 1_500))
        assertNull(cache.fresh(saved.copy(token = "tok_other"), 1_500))
        assertNull(cache.fresh(saved, 1_000 + ServerHealth.MAX_AGE_MS + 1))
        // A clock that went backwards is not "fresh".
        assertNull(cache.fresh(saved, 500))
    }

    @Test fun aFailedProbeIsSharedToo() {
        val cache = ServerHealth()
        val down = HealthReport(ok = false, error = "connect_failed")
        cache.record(saved, down, 1_000)
        assertEquals("Not reachable", SettingsSummary.server(saved, BuildCaps.ALL, cache.fresh(saved, 1_200)?.report))
        cache.record(saved, ok.copy(rttMs = null), 2_000)
        assertEquals("Connected", SettingsSummary.server(saved, BuildCaps.ALL, cache.fresh(saved, 2_100)?.report))
    }

    @Test fun theNewerOfTheSharedAndTheRouterProbeWins() {
        // Review: a 60 s old "Connected" must not hide the router's newer failure, nor the reverse.
        val shared = ServerHealth.Probe(ok, 1_000)
        val routerDown = HealthReport(ok = false, error = "connect_failed", checkedAtMs = 2_000)
        assertEquals(ServerHealth.Probe(routerDown, 2_000), ServerHealth.newer(shared, routerDown))
        val routerOld = routerDown.copy(checkedAtMs = 500)
        assertEquals(shared, ServerHealth.newer(shared, routerOld))
        assertEquals(shared, ServerHealth.newer(shared, routerDown.copy(checkedAtMs = null)))
        assertEquals(ServerHealth.Probe(routerDown, 2_000), ServerHealth.newer(null, routerDown))
        assertNull(ServerHealth.newer(null, null))
        assertEquals(true, ServerHealth.isFresh(2_000, 2_500))
        assertEquals(false, ServerHealth.isFresh(null, 2_500))
    }

    @Test fun sameServerIgnoresTimeouts() {
        assertEquals(true, ServerHealth.sameServer(saved, saved.copy(healthTtlS = saved.healthTtlS + 5)))
        assertEquals(false, ServerHealth.sameServer(saved, saved.copy(tls = !saved.tls)))
    }
}
