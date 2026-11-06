package dev.flowclone.core

import dev.flowclone.data.BuildCaps
import dev.flowclone.data.CloudSettings
import dev.flowclone.data.SttSettings
import dev.flowclone.data.CleanupSettings
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

    /**
     * D36: the shell always sends the whole cleanup order (an empty one is the router's pre-D36
     * behaviour), which the pre-D36 fixtures predate: compared without it, checked on its own.
     */
    private fun withoutCleanup(j: JSONObject) = JSONObject(j.toString()).apply {
        remove("allow_cloud_cleanup"); remove("cleanup_order"); remove("cleanup_off")
    }

    private fun withoutCleanup(fixture: String) = withoutCleanup(JSONObject(fixture)).toString()

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
            assertSameJson(fixture(file), withoutCleanup(s.toRouterConfigJson()))
            assertEquals(file, "[\"local\",\"server\",\"cloud\"]", s.toRouterConfigJson().getJSONArray("cleanup_order").toString())
            assertEquals(file, "[\"cloud\"]", s.toRouterConfigJson().getJSONArray("cleanup_off").toString())
            assertFalse(file, s.toRouterConfigJson().getBoolean("allow_cloud_cleanup"))
            // …and parse back to the same settings (the UI-only badge flag is not part of RouterConfig).
            assertEquals(file, s, DevSettings.fromRouterConfigJson(fixture(file)))
        }
    }

    /** D28: the cloud fields serialise to exactly `router_config_cloud.json` (schema 2; keys are never in it). */
    @Test fun devSettingsSerialiseToTheCloudFixture() {
        val s = DevSettings.defaults(BuildCaps.ALL).copy(
            host = "100.64.0.10", token = "dev-token-0123456789abcdef", allowCloud = true, allowCloudCleanup = true,
            routeOrder = listOf(DevSettings.Route.CLOUD, DevSettings.Route.SERVER, DevSettings.Route.CLIENT),
            // D36: the fixture's pre-D36 `cloud, local` gains the server right after the phone.
            cleanupOrder = listOf(DevSettings.CleanupRoute.CLOUD, DevSettings.CleanupRoute.LOCAL, DevSettings.CleanupRoute.SERVER),
            cloud = CloudSettings(
                stt = SttSettings().withConsent(1792540800000), cleanup = CleanupSettings().withConsent(1792540900000),
                monthlyBudgetUsd = 5.0,
            ),
        )
        assertSameJson(withoutCleanup(fixture("router_config_cloud.json")), withoutCleanup(s.toRouterConfigJson()))
        val j = s.toRouterConfigJson()
        assertEquals("[\"cloud\",\"local\",\"server\"]", j.getJSONArray("cleanup_order").toString())
        assertEquals("[]", j.getJSONArray("cleanup_off").toString()); assertTrue(j.getBoolean("allow_cloud_cleanup"))
        assertEquals(s, DevSettings.fromRouterConfigJson(fixture("router_config_cloud.json")))
        assertTrue(s.cloud.stt.consentOk); assertTrue(s.cloud.cleanup.consentOk)
        assertTrue(s.cloudCleanupRouted)
        // prefer still follows the first of server / client, for routers that ignore `order`.
        assertEquals(DevSettings.Prefer.SERVER, s.prefer)
        // A local build never sends cloud fields at all (C2).
        val local = JSONObject(s.routerConfigFor(BuildCaps.LOCAL))
        for (k in listOf("cloud", "order", "allow_cloud", "allow_cloud_cleanup", "cleanup_order", "cleanup_off")) assertFalse(k, local.has(k))
        // A remote build has no phone engine: the server and the cloud only.
        assertEquals("[\"cloud\",\"server\"]", JSONObject(s.routerConfigFor(BuildCaps.REMOTE)).getJSONArray("cleanup_order").toString())
    }

    @Test fun cloudFixturesDecode() {
        assertEquals(CloudCapabilities(true, listOf("openrouter")), RouterJson.decodeCloudCapabilities(fixture("cloud_capabilities.json")))
        val h = RouterJson.decodeHealth(fixture("health_cloud_ok.json"))
        assertTrue(h.ok); assertEquals("cloud", h.engine); assertEquals("openrouter", h.provider)
        assertEquals("And so, my fellow Americans,", h.transcript); assertEquals(820L, h.latencyMs); assertEquals(0.000225, h.costUsd!!, 1e-12)
        assertEquals("OK · heard \"And so, my fellow Americans,\" · 820 ms · $0.0002 · openai/whisper-large-v3-turbo", DevText.probe(h))
        val st = RouterJson.decodeRouteStatus(fixture("route_status_cloud.json"))
        assertEquals(CircuitStatus(open = true, reason = "auth_failed", untilConfigChange = true), st.cloudCircuit)
        assertEquals("auth_failed", st.last!!.fallback)
        assertNull(RouterJson.decodeRouteStatus(fixture("route_status.json")).cloudCircuit)
        assertNull(RouterJson.decodeRouteStatus(fixture("route_status.json")).cleanupCircuit)
        val c = CoreJson.decodeCommit(fixture("commit_result_cloud.json"))
        val r = c.route!!
        assertEquals("cloud", r.engine); assertEquals("openrouter", r.provider); assertEquals("openai/whisper-large-v3-turbo", r.model)
        assertEquals(1000L, r.billedAudioMs); assertEquals(0.000011, r.costUsd!!, 1e-12); assertEquals(false, r.costEstimated)
        // D28: who cleaned it.
        assertEquals("cloud", r.llmEngine); assertEquals("meta-llama/llama-3.3-70b-instruct", r.llmModel); assertEquals(0.0007, r.llmCostUsd!!, 1e-12)
        // D30: what produced the text.
        assertEquals("cloud", r.cleanupRoute); assertNull(r.cloudFallback)
        assertEquals("C", DevText.badge(r))
        assertEquals("C", DevText.bubbleBadge(r, showEngineBadge = false))
        assertNull(DevText.bubbleBadge(CoreJson.decodeCommit(fixture("commit_result_remote.json")).route, showEngineBadge = false))
        val line = DevText.lastDictation(c)
        assertTrue(line, line.startsWith("cloud · OpenRouter · openai/whisper-large-v3-turbo · $0.0000"))
        assertTrue(line, line.endsWith("AI cleanup: OpenRouter · llama-3.3-70b-instruct · $0.0007"))
        val u = dev.flowclone.data.CloudUsage.decode(fixture("cloud_usage.json"))
        assertEquals("2026-10", u.month)
        assertEquals(dev.flowclone.data.CloudUsage.ProviderUsage("openrouter", 42, 312000, 0.0234, false, "stt"), u.providers[0])
        assertEquals("cleanup", u.providers[1].purpose)
    }

    /** D28 `nativeCleanupStatus` (`cleanup_status.json`) and its status line. */
    @Test fun cleanupStatusFixture() {
        val c = RouterJson.decodeCleanupStatus(fixture("cleanup_status.json"))
        assertTrue(c.supported && c.enabled && c.consent)
        assertEquals(listOf("cloud", "local", "server"), c.order)
        // D36: the server's AI and its cleanup breaker.
        assertEquals(true, c.serverLlm); assertEquals(CircuitStatus(), c.serverCircuit)
        assertNull(RouterJson.decodeCleanupStatus("{}").serverLlm)
        assertNull(c.blocked); assertEquals(CircuitStatus(), c.circuit)
        assertEquals("meta-llama/llama-3.3-70b-instruct", c.model); assertEquals(2500L, c.timeoutMs)
        assertEquals(CleanUsage("cloud", "meta-llama/llama-3.3-70b-instruct", 0.0007, false, "gen-0000000000-fixture", 310, 640), c.last)
        assertEquals(0.014, c.monthCostUsd, 1e-12)
        assertEquals("On · meta-llama/llama-3.3-70b-instruct · last 640 ms first token 310 ms $0.0007 · this month $0.01", DevText.cleanupStatus(c))
        assertEquals("On · m · skipped: no zero-retention provider serves this model: pick one marked ZDR · this month $0",
            DevText.cleanupStatus(CleanupStatus(supported = true, enabled = true, model = "m", blocked = "no_private_route")))
        assertEquals("Not in this build", DevText.cleanupStatus(RouterJson.decodeCleanupStatus("{}")))
    }

    @Test fun keyInfoLine() {
        val k = RouterJson.decodeKeyInfo("""{"label":"sk-or-v1-abc...xyz","limit":10.0,"limit_remaining":7.5,"usage_monthly":1.25,"is_free_tier":false,"warn_no_limit":false}""")
        assertEquals("Key OK · sk-or-v1-abc...xyz · credit left $7.50 of $10.00 · this month $1.25", DevText.keyInfo(k))
        val nolimit = RouterJson.decodeKeyInfo("""{"label":"x","limit":null,"limit_remaining":null,"usage_monthly":0,"warn_no_limit":true}""")
        assertTrue(DevText.keyInfo(nolimit).endsWith("OpenRouter recommends setting one."))
        assertEquals("auth_failed: key rejected", DevText.keyInfo(RouterJson.decodeKeyInfo("""{"error":"auth_failed"}""")))
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
        assertEquals(listOf(PasteSegment.Text("ask not what your country can do. ")), remote.pastePlan())
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
            CorePoll("ask not what", false, 1200, 300, SessionRoute("cloud", provider = "openrouter", model = "openai/whisper-large-v3-turbo")),
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

    @Test fun lastDictationShowsTheLlmOutcomeAsANormalResult() {
        val skipped = CommitResult(CommitStatus.RAW_TRANSCRIPT, fallbackReason = "llm_skipped_clean", route = RouteInfo("local"))
        assertTrue(DevText.lastDictation(skipped).endsWith("RAW_TRANSCRIPT · AI not needed (already clean)"))
        assertTrue(DevText.lastDictation(skipped.copy(status = CommitStatus.FORMATTED, fallbackReason = null)).endsWith("FORMATTED · AI cleaned"))
        assertTrue(DevText.lastDictation(skipped.copy(fallbackReason = null)).endsWith("RAW_TRANSCRIPT"))
    }

    @Test fun d24LoadingAndRouteDiagFixtures() {
        val p = RouterJson.decodePoll(fixture("poll_loading.json"))
        assertTrue(p.loading)
        assertEquals("connect_failed", p.route!!.fallback)
        val i = RouterJson.decodeStartInfo(fixture("start_info_loading.json"))
        assertTrue(i.loading)
        assertEquals("local", i.route!!.engine)
        assertFalse(RouterJson.decodeStartInfo(fixture("start_info_remote.json")).loading)
        val d = RouterJson.decodeRouteStatus(fixture("route_status_diag.json")).lastDiag!!
        assertEquals(RouteDiag(connectMs = 2503, localCold = true, localLoadMs = 1840), d)
        assertNull(RouterJson.decodeRouteStatus(fixture("route_status.json")).lastDiag)
    }

    /** 2026-10-27: why AI cleanup fell back reaches the route (debug log only). */
    @Test fun routeCarriesTheFallbackDetail() {
        // A pre-D59 server may still send rejected_text: ignored.
        val r = CoreJson.decodeRoute(JSONObject("""{"engine":"local","cleanup_route":"local","cloud_fallback":"llm_dropped",
            "rejected_text":"The demo is on Thursday.","guard_detail":"dropped: the demo is on thursday at 3"}"""))
        assertEquals("llm_dropped", r.cloudFallback)
        assertEquals("dropped: the demo is on thursday at 3", r.fallbackDetail)
        val none = CoreJson.decodeRoute(JSONObject("""{"engine":"local"}"""))
        assertNull(none.fallbackDetail)
    }

    @Test fun decodesCustomModelsAndTheCleanupProvider() {
        assertEquals(CustomModels(listOf("Llama-3.1-8B", "gpt-4o-mini")), RouterJson.decodeCustomModels("""{"models":["Llama-3.1-8B","gpt-4o-mini"]}"""))
        assertEquals(CustomModels(error = "auth_failed", message = "key rejected"), RouterJson.decodeCustomModels("""{"error":"auth_failed","message":"key rejected"}"""))
        assertEquals("error", RouterJson.decodeCustomModels("not json").error)
        assertEquals("error", RouterJson.decodeCustomModels("{}").error)
        val st = RouterJson.decodeCleanupStatus("""{"supported":true,"provider":"custom","host":"10.0.0.5"}""")
        assertEquals("custom", st.provider); assertEquals("10.0.0.5", st.host)
        val or = RouterJson.decodeCleanupStatus("{}")
        assertEquals("openrouter", or.provider); assertEquals("", or.host)
    }
}
