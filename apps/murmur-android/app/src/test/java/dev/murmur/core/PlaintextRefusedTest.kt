package dev.murmur.core

import dev.murmur.session.DictationController
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Review S6 wording: the router's "plaintext refused" reads as "server needs TLS" everywhere the user sees it. */
class PlaintextRefusedTest {
    private val msg = "connect_failed: plaintext refused: 203.0.113.7 is not private or tailnet (enable TLS)"

    @Test fun testConnectionLine() {
        val h = HealthReport(ok = false, error = "connect_failed", message = msg)
        assertEquals("connect_failed: ${RouterErrors.NEEDS_TLS}", DevText.probe(h))
        assertTrue(DevText.serverInfo(h).contains("Server needs TLS"))
    }

    @Test fun cloudTestProviderLine() {
        val h = HealthReport(ok = false, engine = "cloud", error = "connect_failed", message = "http://x: plaintext refused (use https)")
        assertEquals("connect_failed: ${RouterErrors.CLOUD_NEEDS_TLS}", DevText.probe(h))
    }

    @Test fun bubbleErrorsAtStartAndCommit() {
        assertEquals(RouterErrors.NEEDS_TLS, RouterErrors.userMessage(RouterErrors.SERVER_UNAVAILABLE, "server_unavailable: $msg"))
        assertEquals(RouterErrors.NEEDS_TLS, DictationController.routerFailure(CommitResult(CommitStatus.ERROR, error = msg)))
        assertEquals("unrelated errors unchanged", "Server unreachable (connect_failed)", RouterErrors.userMessage(RouterErrors.SERVER_UNAVAILABLE, "server_unavailable: connect_failed"))
    }
}
