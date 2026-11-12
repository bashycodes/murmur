package dev.murmur.data

import dev.murmur.core.CleanupLevel
import dev.murmur.core.HealthReport
import dev.murmur.data.DevSettings.CleanupRoute
import dev.murmur.data.DevSettings.CleanupRoute.CLOUD
import dev.murmur.data.DevSettings.CleanupRoute.LOCAL
import dev.murmur.data.DevSettings.CleanupRoute.SERVER
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** D36: the 3-entry cleanup order {local, server, cloud}: migration, prefs, router JSON, the AI cleanup page's list. */
class CleanupOrderTest {
    @org.junit.Before fun fakeRouterAddress() = FakeCustomAddress.install()

    private val now = 1_792_540_800_000L

    private fun store(kv: MemKV, caps: BuildCaps = BuildCaps.ALL) = DevStore(kv, caps, SecretVault("t", FakeVaultKeys()))

    @Test fun defaultIsPhoneThenServerThenCloudWithTheCloudOff() {
        val d = DevSettings.defaults(BuildCaps.ALL)
        assertEquals(listOf(LOCAL, SERVER, CLOUD), d.cleanupOrder)
        assertTrue(d.cleanupOn(LOCAL)); assertTrue(d.cleanupOn(SERVER)); assertFalse(d.cleanupOn(CLOUD))
        assertEquals(d, store(MemKV()).load())
    }

    @Test fun preD36OrdersKeepCloudVersusPhoneAndGetTheServerAfterThePhone() {
        fun full(vararg r: CleanupRoute) = DevSettings.fullCleanupOrder(r.toList())
        assertEquals(listOf(LOCAL, SERVER, CLOUD), full())
        assertEquals(listOf(CLOUD, LOCAL, SERVER), full(CLOUD, LOCAL))
        assertEquals(listOf(LOCAL, SERVER, CLOUD), full(LOCAL, CLOUD))
        assertEquals(listOf(CLOUD, LOCAL, SERVER), full(CLOUD))
        assertEquals(listOf(LOCAL, SERVER, CLOUD), full(LOCAL))
        // A D36 order is kept as it is.
        assertEquals(listOf(SERVER, CLOUD, LOCAL), full(SERVER, CLOUD, LOCAL))
    }

    @Test fun migrationKeepsPhoneAndServerOnAndTheCloudOnlyIfItWasUsed() {
        // Cloud allowed and in the order: on.
        DevSettings.cleanupFrom(listOf(CLOUD, LOCAL), emptyList(), allowCloudCleanup = true).let {
            assertEquals(listOf(CLOUD, LOCAL, SERVER), it.order); assertEquals(emptySet<CleanupRoute>(), it.off); assertTrue(it.cloudOn)
        }
        // Cloud never allowed: in the list, but off.
        DevSettings.cleanupFrom(listOf(LOCAL, CLOUD), emptyList(), allowCloudCleanup = false).let {
            assertEquals(listOf(LOCAL, SERVER, CLOUD), it.order); assertFalse(it.cloudOn)
        }
        // Allowed but not in the (empty) pre-D36 order: the router never used it, so off.
        assertFalse(DevSettings.cleanupFrom(emptyList(), emptyList(), allowCloudCleanup = true).cloudOn)
        // A stray pre-D36 `cleanup_off` is ignored: the phone and the server start on.
        assertEquals(emptySet<CleanupRoute>(), DevSettings.cleanupFrom(listOf(LOCAL), listOf(LOCAL), false).off)
        // D36 values are taken as stored.
        DevSettings.cleanupFrom(listOf(SERVER, LOCAL, CLOUD), listOf(LOCAL, CLOUD), allowCloudCleanup = true).let {
            assertEquals(listOf(SERVER, LOCAL, CLOUD), it.order); assertEquals(setOf(LOCAL), it.off); assertFalse(it.cloudOn)
        }
    }

    @Test fun oldPrefsMigrateOnLoadAndSaveTheFullList() {
        val kv = MemKV()
        kv.edit { putString(DevStore.CLEANUP_ORDER, "cloud,local"); putBoolean(DevStore.ALLOW_CLOUD_CLEANUP, true) }
        val s = store(kv).load()
        assertEquals(listOf(CLOUD, LOCAL, SERVER), s.cleanupOrder)
        assertTrue(s.allowCloudCleanup); assertTrue(s.cleanupOn(LOCAL)); assertTrue(s.cleanupOn(SERVER))
        store(kv).save(s.withCleanup(SERVER, false))
        assertEquals("cloud,local,server", kv.getString(DevStore.CLEANUP_ORDER))
        assertEquals("server", kv.getString(DevStore.CLEANUP_OFF))
        val back = store(kv).load()
        assertEquals(listOf(CLOUD, LOCAL, SERVER), back.cleanupOrder); assertFalse(back.cleanupOn(SERVER)); assertTrue(back.allowCloudCleanup)

        val off = MemKV()
        off.edit { putString(DevStore.CLEANUP_ORDER, "local,cloud"); putBoolean(DevStore.ALLOW_CLOUD_CLEANUP, false) }
        store(off).load().let { assertEquals(listOf(LOCAL, SERVER, CLOUD), it.cleanupOrder); assertFalse(it.allowCloudCleanup) }
    }

    @Test fun saveCloudWritesOnlyWhatItIsGiven() {
        val kv = MemKV()
        val st = store(kv)
        st.saveCloud(cloud = CloudSettings(), cleanupOff = setOf(LOCAL))
        assertEquals(setOf(LOCAL), st.load().cleanupOff); assertEquals(listOf(LOCAL, SERVER, CLOUD), st.load().cleanupOrder)
        st.saveCloud(cloud = CloudSettings(), cleanupOrder = listOf(SERVER, LOCAL, CLOUD))
        assertEquals(setOf(LOCAL), st.load().cleanupOff); assertEquals(listOf(SERVER, LOCAL, CLOUD), st.load().cleanupOrder)
        assertEquals("local,cloud", kv.getString(DevStore.CLEANUP_OFF))
    }

    @Test fun routerJsonAlwaysCarriesTheWholeOrderWithTheOffEntries() {
        val s = CloudPage.enableCleanup(DevSettings.defaults(BuildCaps.ALL), now)
            .copy(cleanupOrder = listOf(SERVER, CLOUD, LOCAL)).withCleanup(LOCAL, false)
        val j = JSONObject(s.routerConfigFor(BuildCaps.ALL))
        assertEquals("[\"server\",\"cloud\",\"local\"]", j.getJSONArray("cleanup_order").toString())
        assertEquals("[\"local\"]", j.getJSONArray("cleanup_off").toString())
        assertTrue(j.getBoolean("allow_cloud_cleanup"))
        // The cloud switched off: in `cleanup_off`, and `allow_cloud_cleanup` false.
        val noCloud = JSONObject(CloudPage.disableCleanup(s).routerConfigFor(BuildCaps.ALL))
        assertEquals("[\"cloud\",\"local\"]", noCloud.getJSONArray("cleanup_off").toString())
        assertFalse(noCloud.getBoolean("allow_cloud_cleanup"))
        // Everything off is still the whole list (not the empty, pre-D36 order).
        val allOff = JSONObject(CloudPage.disableCleanup(s).withCleanup(SERVER, false).routerConfigFor(BuildCaps.ALL))
        assertEquals(3, allOff.getJSONArray("cleanup_order").length()); assertEquals(3, allOff.getJSONArray("cleanup_off").length())
        // …and parses back to the same settings.
        assertEquals(s, DevSettings.fromRouterConfigJson(s.toRouterConfigJson().toString()))
        assertEquals(CloudPage.disableCleanup(s), DevSettings.fromRouterConfigJson(CloudPage.disableCleanup(s).toRouterConfigJson().toString()))
    }

    @Test fun anEngineOffThePhoneRunsTheLlmStepWithThePhoneAiOff() {
        val caps = BuildCaps.ALL
        val d = DevSettings.defaults(caps)
        assertFalse("no server set up", d.effective(caps).offPhoneCleanup)
        val server = d.copy(host = "100.64.0.10")
        assertTrue(server.effective(caps).offPhoneCleanup)
        assertFalse(server.withCleanup(SERVER, false).effective(caps).offPhoneCleanup)
        assertFalse("a local build has no server", server.effective(BuildCaps.LOCAL).offPhoneCleanup)
        assertTrue(CloudPage.enableCleanup(d, now).effective(caps).offPhoneCleanup)
        val st = Settings(llmEnabled = false)
        assertEquals(CleanupLevel.LIGHT, st.toSessionConfig(emptyList(), cloudCleanup = server.effective(caps).offPhoneCleanup).cleanupLevel)
    }

    @Test fun pageListsThisBuildsEnginesAndMovesThemPastHiddenOnes() {
        val d = DevSettings.defaults(BuildCaps.ALL)
        assertEquals(listOf(LOCAL, SERVER, CLOUD), CleanupEngines.engines(d, BuildCaps.ALL))
        assertEquals(listOf(SERVER, CLOUD), CleanupEngines.engines(d, BuildCaps.REMOTE))
        assertEquals(listOf(LOCAL), CleanupEngines.engines(d, BuildCaps.LOCAL))
        assertEquals(listOf(SERVER, LOCAL, CLOUD), CleanupEngines.reorder(d, BuildCaps.ALL, SERVER, -1))
        assertEquals(listOf(LOCAL, SERVER, CLOUD), CleanupEngines.reorder(d, BuildCaps.ALL, LOCAL, -1))
        // Remote: the phone keeps its stored place.
        assertEquals(listOf(LOCAL, CLOUD, SERVER), CleanupEngines.reorder(d, BuildCaps.REMOTE, CLOUD, -1))
        // The phone's switch is also the on-device AI.
        assertFalse(CleanupEngines.on(d, Settings(llmEnabled = false), LOCAL))
        assertTrue(CleanupEngines.on(d, Settings(llmEnabled = true), LOCAL))
        assertFalse(CleanupEngines.on(d.withCleanup(LOCAL, false), Settings(llmEnabled = true), LOCAL))
        assertTrue(CleanupEngines.on(d, Settings(), SERVER)); assertFalse(CleanupEngines.on(d, Settings(), CLOUD))
    }

    @Test fun rowStatusLines() {
        val caps = BuildCaps.ALL
        val m = "qwen2.5-1.5b-instruct-q4_k_m.gguf"
        assertEquals("No model imported", CleanupEngines.phoneLine(true, null, null))
        assertEquals("qwen2.5-1.5b · loaded", CleanupEngines.phoneLine(true, m, true))
        assertEquals("qwen2.5-1.5b · loading…", CleanupEngines.phoneLine(true, m, null))
        assertEquals("qwen2.5-1.5b · failed to load", CleanupEngines.phoneLine(true, m, false))
        assertEquals("qwen2.5-1.5b · off, tap to use it", CleanupEngines.phoneLine(false, m, true))
        val d = DevSettings.defaults(caps)
        val set = d.copy(host = "100.64.0.10")
        assertEquals("Not set up", CleanupEngines.serverLine(d, caps, true))
        assertEquals("Not in this build", CleanupEngines.serverLine(set, BuildCaps.LOCAL, true))
        assertEquals("Ready · has an AI model", CleanupEngines.serverLine(set, caps, true))
        assertEquals("Connected · no AI model on the server", CleanupEngines.serverLine(set, caps, false))
        assertEquals("Not checked yet", CleanupEngines.serverLine(set, caps, null))
        assertEquals("Not reachable", CleanupEngines.serverLine(set, caps, true, circuitOpen = true))
        assertEquals("OpenRouter · Sends text · asks first", CleanupEngines.cloudLine(d, hasKey = false))
        val on = CloudPage.enableCleanup(d, now)
        assertEquals("OpenRouter · Sends text · llama-3.3-70b-instruct · no key", CleanupEngines.cloudLine(on, hasKey = false))
        assertEquals("OpenRouter · Sends text · llama-3.3-70b-instruct", CleanupEngines.cloudLine(on, hasKey = true))
    }

    @Test fun serverRowFallsBackToTheHealthProbeBeforeTheCleanupConnectionsHello() {
        // Test connection passed, no dictation yet (cleanup_status has no hello): not "not connected".
        val caps = BuildCaps.ALL
        val set = DevSettings.defaults(caps).copy(host = "100.64.0.10")
        fun line(h: HealthReport?, checking: Boolean = false, serverLlm: Boolean? = null, circuit: Boolean = false) =
            CleanupEngines.serverLine(set, caps, serverLlm, circuit, h, checking)
        assertEquals("Ready · has an AI model", line(HealthReport(ok = true, llm = true)))
        assertEquals("Connected · no AI model on the server", line(HealthReport(ok = true, llm = false)))
        assertEquals("Connected", line(HealthReport(ok = true)))
        assertEquals("Not reachable", line(HealthReport(ok = false, error = "connect_failed")))
        assertEquals("Checking…", line(null, checking = true))
        assertEquals("Not checked yet", line(null))
        // The cleanup connection's own answer and an open circuit win over an older probe.
        assertEquals("Connected · no AI model on the server", line(HealthReport(ok = true, llm = true), serverLlm = false))
        assertEquals("Not reachable", line(HealthReport(ok = true, llm = true), circuit = true))
        // A failed probe wins over an old hello: the router keeps "has a model" with no expiry, so a
        // server that went down after one dictation must not stay "Ready".
        assertEquals("Not reachable", line(HealthReport(ok = false, error = "connect_failed"), serverLlm = true))
        assertEquals("Not reachable", line(HealthReport(ok = false, error = "connect_failed"), serverLlm = false))
        // Not set up stays that, whatever was probed.
        assertEquals("Not set up", CleanupEngines.serverLine(DevSettings.defaults(caps), caps, null, h = HealthReport(ok = true)))
    }

    @Test fun theFirstCleanupModelSwitchesThePhoneEngineOn() {
        val d = DevSettings.defaults(BuildCaps.ALL)
        // Fresh install: the phone first in the order, but the on-device AI off.
        val off = Settings(llmEnabled = false)
        assertFalse(CleanupEngines.on(d, off, LOCAL))
        val (s, cleanupOff) = CleanupEngines.phoneOnForFirstModel(hadModel = false, off, d)!!
        assertTrue(CleanupEngines.on(d.copy(cleanupOff = cleanupOff), s, LOCAL))
        // The row was switched off too: both come on.
        val rowOff = d.withCleanup(LOCAL, false)
        val (s2, off2) = CleanupEngines.phoneOnForFirstModel(hadModel = false, off, rowOff)!!
        assertTrue(CleanupEngines.on(rowOff.copy(cleanupOff = off2), s2, LOCAL))
        // The server's switch is left as it was.
        assertFalse(CleanupEngines.phoneOnForFirstModel(false, off, d.withCleanup(SERVER, false))!!.second.let { SERVER !in it })
        // Already on, or a model was there before (the user's off stays off): nothing to change.
        assertEquals(null, CleanupEngines.phoneOnForFirstModel(false, Settings(llmEnabled = true), d))
        assertEquals(null, CleanupEngines.phoneOnForFirstModel(hadModel = true, off, d))
    }

    @Test fun anUpgradeWithAiCleanupOffKeepsItOffButAFreshInstallHasTheServerOn() {
        // (pre-D36 prefs, phone AI was on, server on after the upgrade)
        val cases = listOf(
            Triple(mapOf("allow_server" to "true"), false, false),
            Triple(mapOf(DevStore.CLEANUP_ORDER to "cloud,local"), false, false),
            Triple(mapOf(DevStore.CLEANUP_ORDER to "local", DevStore.ALLOW_CLOUD_CLEANUP to "true"), false, false),
            Triple(mapOf("allow_server" to "true"), true, true),
            // Cloud cleanup was on: some AI cleaned, so the server joins after the phone.
            Triple(mapOf(DevStore.CLEANUP_ORDER to "cloud,local", DevStore.ALLOW_CLOUD_CLEANUP to "true"), false, true),
        )
        cases.forEachIndexed { i, (old, phoneAi, serverOn) ->
            val kv = MemKV()
            kv.edit {
                old.forEach { (k, v) -> if (v == "true") putBoolean(k, true) else putString(k, v) }
            }
            store(kv).migrateCleanupD36(phoneAiWasOn = phoneAi)
            val s = store(kv).load()
            assertEquals("case $i", serverOn, s.cleanupOn(SERVER))
            assertEquals("case $i", SERVER, s.cleanupOrder[s.cleanupOrder.indexOf(LOCAL) + 1])
            val router = JSONObject(s.routerConfigFor(BuildCaps.ALL))
            val off = router.getJSONArray("cleanup_off").let { a -> (0 until a.length()).map { a.getString(it) } }
            assertEquals("case $i", !serverOn, "server" in off)
            // Once only: turning the phone AI on later doesn't change it.
            store(kv).migrateCleanupD36(phoneAiWasOn = true)
            assertEquals("case $i", serverOn, store(kv).load().cleanupOn(SERVER))
        }
        // Fresh install (nothing stored), phone AI off by default: the server is on, and stays on.
        val kv = MemKV()
        store(kv).migrateCleanupD36(phoneAiWasOn = false)
        assertTrue(store(kv).load().cleanupOn(SERVER))
        store(kv).load(); store(kv).migrateCleanupD36(phoneAiWasOn = false)
        assertTrue(store(kv).load().cleanupOn(SERVER))
    }

    @Test fun cloudEngineIsCloudAndItsLineNamesTheProvider() {
        val d = DevSettings.defaults(BuildCaps.ALL)
        assertEquals("Cloud", CleanupEngines.name(CLOUD))
        assertEquals("Clean up in the cloud", CleanupEngines.useLabel(CLOUD))
        val cc = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "gpt-4o-mini")
        val c = d.copy(cloud = d.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = cc), allowCloudCleanup = true)
        assertEquals("This phone", CleanupEngines.name(LOCAL))
        assertEquals("LiteLLM (10.0.0.5) · Sends text · asks first · unencrypted (http)", CleanupEngines.cloudLine(c, hasKey = false))
        val ok = c.copy(cloud = c.cloud.copy(customCleanup = cc.copy(baseUrl = "https://10.0.0.5/v1").withConsent(1)))
        assertEquals("LiteLLM (10.0.0.5) · Sends text · gpt-4o-mini", CleanupEngines.cloudLine(ok, hasKey = false))
        assertEquals("OpenRouter · Sends text · asks first · no key", CleanupEngines.cloudLine(d.copy(allowCloudCleanup = true), hasKey = false))
    }

    @Test fun anUnusableLiteLlmSaysCloudCleanupIsOff() {
        // Custom selected, then its model cleared: the router gets OpenRouter without cleanup
        // consent; the order row must say so, never "OpenRouter".
        val d = DevSettings.defaults(BuildCaps.ALL)
        val cc = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "", consentHost = "10.0.0.5", consentAtMs = 1)
        val c = d.copy(cloud = d.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = cc), allowCloudCleanup = true)
        assertTrue(c.cloud.customNotSetUp)
        assertEquals(LiteLLM.NOT_SET_UP, CleanupEngines.cloudLine(c, hasKey = true))
        assertEquals(LiteLLM.NOT_SET_UP, c.cloud.cleanupProviderName)
        assertFalse(c.cloud.routerJson().has("cleanup_provider"))
        assertEquals(0L, c.cloud.routerJson().getJSONObject("openrouter").getJSONObject("cleanup").getLong("consent_at_ms"))
    }
}
