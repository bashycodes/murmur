package dev.flowclone.core

import dev.flowclone.data.BuildCaps
import dev.flowclone.data.CloudPresets
import dev.flowclone.data.CloudSettings
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

    /** D18: the cloud fields serialise to exactly `router_config_cloud.json` (keys are never in it). */
    @Test fun devSettingsSerialiseToTheCloudFixture() {
        val s = DevSettings.defaults(BuildCaps.ALL).copy(
            host = "100.64.0.10", token = "dev-token-0123456789abcdef", allowCloud = true,
            routeOrder = listOf(DevSettings.Route.CLOUD, DevSettings.Route.SERVER, DevSettings.Route.CLIENT),
            cloud = CloudSettings(
                active = "openrouter-1", monthlyBudgetUsd = 5.0,
                providers = listOf(
                    CloudPresets.of("openrouter").newProvider(emptyList()).copy(consentHost = "openrouter.ai", consentAtMs = 1792540800000),
                    CloudPresets.of("groq").newProvider(emptyList()),
                ),
            ),
        )
        assertEquals("openrouter-1", s.cloud.providers[0].id)
        assertEquals("groq-1", s.cloud.providers[1].id)
        assertSameJson(fixture("router_config_cloud.json"), s.toRouterConfigJson())
        assertEquals(s, DevSettings.fromRouterConfigJson(fixture("router_config_cloud.json")))
        assertTrue(s.cloud.activeProvider!!.consentOk)
        assertFalse(s.cloud.provider("groq-1")!!.consentOk)
        // prefer still follows the first of server / client, for routers that ignore `order`.
        assertEquals(DevSettings.Prefer.SERVER, s.prefer)
        // A local build never sends cloud fields at all (C2).
        val local = JSONObject(s.routerConfigFor(BuildCaps.LOCAL))
        assertFalse(local.has("cloud")); assertFalse(local.has("order")); assertFalse(local.has("allow_cloud"))
    }

    @Test fun cloudFixturesDecode() {
        assertEquals(CloudCapabilities(true, listOf("openai_compat", "openrouter", "elevenlabs", "deepgram", "assemblyai")),
            RouterJson.decodeCloudCapabilities(fixture("cloud_capabilities.json")))
        val h = RouterJson.decodeHealth(fixture("health_cloud_ok.json"))
        assertTrue(h.ok); assertEquals("cloud", h.engine); assertEquals("openrouter-1", h.provider)
        assertEquals("And so, my fellow Americans,", h.transcript); assertEquals(820L, h.latencyMs); assertEquals(0.000225, h.costUsd!!, 1e-12)
        assertEquals("OK · heard \"And so, my fellow Americans,\" · 820 ms · $0.0002 · openai/gpt-transcribe", DevText.probe(h))
        val st = RouterJson.decodeRouteStatus(fixture("route_status_cloud.json"))
        assertEquals(CircuitStatus(open = true, reason = "auth_failed", untilConfigChange = true), st.cloudCircuit)
        assertEquals("auth_failed", st.last!!.fallback)
        assertNull(RouterJson.decodeRouteStatus(fixture("route_status.json")).cloudCircuit)
        val c = CoreJson.decodeCommit(fixture("commit_result_cloud.json"))
        val r = c.route!!
        assertEquals("cloud", r.engine); assertEquals("openrouter-1", r.provider); assertEquals("openai/gpt-transcribe", r.model)
        assertEquals(1000L, r.billedAudioMs); assertEquals(0.000075, r.costUsd!!, 1e-12); assertEquals(false, r.costEstimated)
        assertEquals("C", DevText.badge(r))
        assertEquals("C", DevText.bubbleBadge(r, showEngineBadge = false))
        assertNull(DevText.bubbleBadge(CoreJson.decodeCommit(fixture("commit_result_remote.json")).route, showEngineBadge = false))
        assertTrue(DevText.lastDictation(c) { "OpenRouter" }, DevText.lastDictation(c) { "OpenRouter" }.startsWith("cloud · OpenRouter · openai/gpt-transcribe · $0.0001"))
        val u = dev.flowclone.data.CloudUsage.decode(fixture("cloud_usage.json"))
        assertEquals("2026-10", u.month)
        assertEquals(dev.flowclone.data.CloudUsage.ProviderUsage("openrouter-1", 42, 312000, 0.0234, false), u.providers.single())
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

    // ---- 2026-10-22 live route + hands-free endpointing (crates/flow-router/fixtures) ----

    @Test fun decodesStartInfoAndPollFixtures() {
        assertEquals(
            StartInfo(queued = false, route = SessionRoute("remote", server = "100.64.0.10:7313")),
            RouterJson.decodeStartInfo(fixture("start_info_remote.json")),
        )
        assertEquals(
            CorePoll("ask not what", false, 1200, 300, SessionRoute("cloud", provider = "openrouter-1", model = "openai/gpt-transcribe")),
            RouterJson.decodePoll(fixture("poll_cloud.json")),
        )
        assertEquals(
            CorePoll(null, true, 2400, 2000, SessionRoute("local", fallback = "disconnected")),
            RouterJson.decodePoll(fixture("poll_fallback.json")),
        )
        assertEquals(CorePoll(), RouterJson.decodePoll(fixture("poll_idle.json")))
        assertEquals(CorePoll(), RouterJson.decodePoll("not json"))
        assertEquals(StartInfo(), RouterJson.decodeStartInfo("{}"))
    }

    @Test fun lastDictationShowsClientMs() {
        val r = CommitResult(CommitStatus.RAW_TRANSCRIPT, timings = Timings(asrMs = 400, totalMs = 520), route = RouteInfo("remote", server = "h:1", clientMs = 610))
        val s = DevText.lastDictation(r)
        assertTrue(s, s.contains("client 610 ms"))
        assertFalse(DevText.lastDictation(r.copy(route = r.route!!.copy(clientMs = null))).contains("client"))
    }
}
