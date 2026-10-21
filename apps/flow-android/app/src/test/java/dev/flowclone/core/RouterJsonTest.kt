package dev.flowclone.core

import dev.flowclone.data.BuildCaps
import dev.flowclone.data.DevSettings
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Kotlin mirror types parse the router's own golden fixtures
 * (crates/flow-router/fixtures, on the test classpath via sourceSets — no copies), and
 * DevSettings serialises to exactly the `router_config_*.json` the router expects.
 */
class RouterJsonTest {
    private fun fixture(name: String): String =
        requireNotNull(javaClass.classLoader!!.getResource(name)) { "fixture $name not on the test classpath" }.readText()

    /** Key-for-key, value-for-value (numbers by value: Int 800 == Long 800). */
    private fun assertSameJson(expected: String, actual: JSONObject) =
        assertTrue("expected $expected\nactual   $actual", same(JSONObject(expected), actual))

    private fun same(a: Any?, b: Any?): Boolean = when {
        a is JSONObject && b is JSONObject ->
            a.keys().asSequence().toSet() == b.keys().asSequence().toSet() && a.keys().asSequence().all { same(a.opt(it), b.opt(it)) }
        a is org.json.JSONArray && b is org.json.JSONArray -> a.length() == b.length() && (0 until a.length()).all { same(a.opt(it), b.opt(it)) }
        a is Number && b is Number -> a.toDouble() == b.toDouble()
        else -> a == b
    }

    @Test fun devSettingsSerialiseToEveryRouterConfigFixture() {
        val token = "dev-token-0123456789abcdef"
        val cases = mapOf(
            "router_config_default.json" to DevSettings.defaults(BuildCaps.ALL),
            "router_config_hybrid.json" to DevSettings.defaults(BuildCaps.ALL).copy(host = "100.64.0.10", token = token),
            "router_config_local_only.json" to DevSettings(allowServer = false, allowClient = true, routeOrder = DevSettings.orderFor(DevSettings.Prefer.CLIENT)),
            "router_config_remote_only.json" to DevSettings(
                allowServer = true, allowClient = false, host = "flow.example.com", port = 443, tls = true,
                tlsPinSha256 = "3f2a9c0b7e4d1f6a8b5c2e9d0a7f4b1c6e3d8a5f2b9c0e7d4a1f6b3c8e5d2a9f", token = token,
                connectTimeoutMs = 1500, keepLocalWarm = false,
            ),
        )
        for ((file, s) in cases) {
            assertSameJson(fixture(file), s.toRouterConfigJson())
            // …and parse back to the same settings (the UI-only badge flag is not part of RouterConfig).
            assertEquals(file, s, DevSettings.fromRouterConfigJson(fixture(file)))
        }
    }

    @Test fun capabilitiesFixturesMatchTheFlavours() {
        val local = RouterJson.decodeCapabilities(fixture("capabilities_local.json"))
        assertTrue(local.local); assertFalse(local.remote); assertEquals(listOf("whisper"), local.asrEngines)
        val hybrid = RouterJson.decodeCapabilities(fixture("capabilities_hybrid.json"))
        assertTrue(hybrid.local && hybrid.remote); assertTrue("parakeet" in hybrid.asrEngines)
        val remote = RouterJson.decodeCapabilities(fixture("capabilities_remote.json"))
        assertFalse(remote.local); assertTrue(remote.remote); assertTrue(remote.asrEngines.isEmpty()); assertFalse(remote.llm)
        assertEquals(1, remote.proto); assertEquals("0.1.0", remote.coreVersion)
    }

    @Test fun healthOkAndAuthFailed() {
        val ok = RouterJson.decodeHealth(fixture("health_ok.json"))
        assertTrue(ok.ok)
        assertEquals(Rtt(3, 4, 6), ok.rttMs)
        assertEquals("flow-server 0.1.0", ok.serverVersion)
        assertEquals("parakeet", ok.asrBackend)
        assertEquals(true, ok.postprocessMatch)
        assertEquals(1, ok.workers); assertEquals(0, ok.busy)
        assertEquals("OK · median 4 ms (3-6) · flow-server 0.1.0 · parakeet", DevText.probe(ok))
        val bad = RouterJson.decodeHealth(fixture("health_auth_failed.json"))
        assertFalse(bad.ok)
        assertEquals("auth_failed", bad.error)
        assertNull(bad.rttMs); assertNull(bad.workers)
        assertTrue(DevText.probe(bad), DevText.probe(bad).startsWith("auth_failed: token rejected"))
    }

    @Test fun routeStatusFixture() {
        val s = RouterJson.decodeRouteStatus(fixture("route_status.json"))
        assertEquals(RouteInfo("local", "connect_failed", "100.64.0.10:7313", null, null, null), s.last)
        assertTrue(s.circuit.open); assertEquals("connect_failed", s.circuit.reason); assertEquals(4200L, s.circuit.retryInMs)
        assertFalse(s.circuit.untilConfigChange)
        assertTrue(s.health!!.ok)
        assertEquals("open until 12:00:04 (connect_failed)", DevText.circuit(s.circuit, 0) { "12:00:0${it / 1000}" })
    }

    @Test fun commitResultFixturesCarryRoute() {
        val remote = CoreJson.decodeCommit(fixture("commit_result_remote.json"))
        assertEquals(CommitStatus.RAW_TRANSCRIPT, remote.status)
        assertEquals("remote", remote.route!!.engine); assertEquals(4L, remote.route!!.rttMs); assertEquals("0.1.0", remote.route!!.serverCore)
        assertEquals(listOf(PasteSegment.Text("ask not what your country can do ")), remote.pastePlan())
        assertEquals("S", DevText.badge(remote.route))
        val fb = CoreJson.decodeCommit(fixture("commit_result_fallback.json"))
        assertEquals("connect_failed", fb.route!!.fallback)
        assertEquals("L•", DevText.badge(fb.route))
        assertTrue(DevText.lastDictation(fb), DevText.lastDictation(fb).startsWith("Client (on-device) · fallback: connect_failed"))
    }
}
