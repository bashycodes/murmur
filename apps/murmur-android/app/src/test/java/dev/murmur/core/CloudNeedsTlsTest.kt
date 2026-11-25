package dev.murmur.core

import org.junit.Assert.assertEquals
import org.junit.Test

/** The router's cloud rule (http:// base URL at a public address) reads as "use https://" on Test provider. */
class CloudNeedsTlsTest {
    @Test fun cloudTestProviderLine() {
        val h = HealthReport(ok = false, engine = "cloud", error = "cloud_not_configured", message = "public base URL: https:// is required")
        assertEquals("cloud_not_configured: ${RouterErrors.CLOUD_NEEDS_TLS}", DevText.probe(h))
    }

    @Test fun serverErrorsAreShownAsIs() {
        assertEquals("Server unreachable (connect_failed)", RouterErrors.userMessage(RouterErrors.SERVER_UNAVAILABLE, "server_unavailable: connect_failed"))
    }
}
