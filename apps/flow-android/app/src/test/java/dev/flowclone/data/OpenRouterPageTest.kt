package dev.flowclone.data

import dev.flowclone.data.DevSettings.CleanupRoute
import dev.flowclone.data.DevSettings.Route
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D28 page logic: two independent opt-ins with their own consents, order, models, privacy. */
class OpenRouterPageTest {
    private val base = DevSettings.defaults(BuildCaps.ALL)
    private val now = 1_792_540_800_000L

    @Test fun sttAndCleanupAreIndependent() {
        val stt = OpenRouterPage.enableStt(base, now)
        assertTrue(stt.allowCloud); assertTrue(stt.cloud.stt.consentOk)
        assertEquals(listOf(Route.CLOUD, Route.SERVER, Route.CLIENT), stt.routeOrder)
        assertFalse("STT never turns cleanup on", stt.allowCloudCleanup)
        assertFalse(stt.cloud.cleanup.consentOk)
        assertTrue(OpenRouterPage.cleanupNeedsConsent(stt))

        val cl = OpenRouterPage.enableCleanup(base, now)
        assertTrue(cl.allowCloudCleanup); assertTrue(cl.cloud.cleanup.consentOk)
        // D36: switched on where it stands in the order (default: last).
        assertEquals(listOf(CleanupRoute.LOCAL, CleanupRoute.SERVER, CleanupRoute.CLOUD), cl.cleanupOrder)
        assertFalse("cleanup never turns STT on", cl.allowCloud); assertFalse(cl.cloud.stt.consentOk)
        assertTrue(cl.cloudCleanupRouted)

        // Off keeps the consent: on again asks nothing.
        val off = OpenRouterPage.disableCleanup(cl)
        assertFalse(off.allowCloudCleanup); assertFalse(OpenRouterPage.cleanupNeedsConsent(off))
        assertEquals(cl.cloud.cleanup.consentAtMs, OpenRouterPage.enableCleanup(off, now + 5).cloud.cleanup.consentAtMs)
        assertFalse(OpenRouterPage.disableStt(stt).allowCloud)
        assertEquals(stt.routeOrder, OpenRouterPage.disableStt(stt).routeOrder)
    }

    @Test fun cleanupOrder() {
        val cl = OpenRouterPage.enableCleanup(base, now)
        val cloudFirst = cl.copy(cleanupOrder = listOf(CleanupRoute.CLOUD, CleanupRoute.LOCAL, CleanupRoute.SERVER))
        assertEquals("On-device, then OpenRouter (llama-3.3-70b-instruct)", OpenRouterPage.cleanupEngineLine(cl))
        assertEquals("OpenRouter (llama-3.3-70b-instruct), then on-device", OpenRouterPage.cleanupEngineLine(cloudFirst))
        // A configured server is named; a switched-off engine is not.
        val withServer = cl.copy(host = "100.64.0.10")
        assertEquals("On-device, then your server, then OpenRouter (llama-3.3-70b-instruct)", OpenRouterPage.cleanupEngineLine(withServer))
        assertEquals("Your server, then OpenRouter (llama-3.3-70b-instruct)", OpenRouterPage.cleanupEngineLine(withServer.withCleanup(CleanupRoute.LOCAL, false)))
        // A chosen order survives off → on.
        assertEquals(cloudFirst.cleanupOrder, OpenRouterPage.enableCleanup(OpenRouterPage.disableCleanup(cloudFirst), now).cleanupOrder)
        assertNull(OpenRouterPage.cleanupEngineLine(base))
    }

    @Test fun routerJsonCarriesBothUses() {
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(base, now), now)
        val j = JSONObject(s.routerConfigFor(BuildCaps.ALL))
        assertTrue(j.getBoolean("allow_cloud")); assertTrue(j.getBoolean("allow_cloud_cleanup"))
        assertEquals("[\"local\",\"server\",\"cloud\"]", j.getJSONArray("cleanup_order").toString())
        assertEquals("[]", j.getJSONArray("cleanup_off").toString())
        assertEquals(2, j.getJSONObject("cloud").getInt("schema"))
        assertEquals("openrouter.ai", j.getJSONObject("cloud").getJSONObject("openrouter").getJSONObject("cleanup").getString("consent_host"))
    }

    @Test fun modelsAndPrivacy() {
        val s = OpenRouterPage.withModel(base, CloudCatalog.Use.STT, " google/gemini-3.5-flash-lite ", "chat_audio")
        assertEquals("google/gemini-3.5-flash-lite", s.cloud.stt.model); assertEquals("chat_audio", s.cloud.stt.sttPath)
        val c = OpenRouterPage.withModel(base, CloudCatalog.Use.CLEANUP, "meta-llama/llama-3.1-8b-instruct:nitro")
        assertEquals("meta-llama/llama-3.1-8b-instruct:nitro", OpenRouterPage.model(c, CloudCatalog.Use.CLEANUP))
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, OpenRouterPage.model(c, CloudCatalog.Use.STT))
        assertTrue(runCatching { OpenRouterPage.withModel(base, CloudCatalog.Use.STT, "Bad Id") }.isFailure)
        val p = OpenRouterPage.withPrivacy(base, zdr = false)
        assertFalse(p.cloud.privacy.zdr); assertTrue("deny stays unless turned off too", p.cloud.privacy.denyDataCollection)
        assertEquals("allow", OpenRouterPage.withPrivacy(p, denyCollection = false).cloud.privacy.dataCollection)
    }

    @Test fun summaries() {
        assertEquals("Speech off · Cleanup off", OpenRouterPage.settingsSummary(base, hasKey = false))
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(base, now), now)
        assertEquals("Speech on · Cleanup on (llama-3.3-70b-instruct) · no key", OpenRouterPage.settingsSummary(s, hasKey = false))
        assertEquals("Speech on · Cleanup on (llama-3.3-70b-instruct)", OpenRouterPage.settingsSummary(s, hasKey = true))
        assertEquals("Speech on · Cleanup on · budget \$5.00",
            DevSummary.cloud(s.copy(cloud = s.cloud.copy(monthlyBudgetUsd = 5.0)), BuildCaps.ALL, hasKey = true))
        assertEquals("Speech off · Cleanup off · no key", DevSummary.cloud(base, BuildCaps.ALL, hasKey = false))
        assertEquals("Not in this build", DevSummary.cloud(base, BuildCaps.LOCAL, hasKey = false))
        assertFalse(OpenRouterPage.mayRefreshInBackground(base)); assertTrue(OpenRouterPage.mayRefreshInBackground(s))
    }

    @Test fun cloudCleanupRunsTheLlmStepEvenWithTheLocalAiOff() {
        val st = Settings()
        assertEquals(dev.flowclone.core.CleanupLevel.NONE, st.copy(llmEnabled = false).toSessionConfig(emptyList()).cleanupLevel)
        val cfg = st.copy(llmEnabled = false).toSessionConfig(emptyList(), cloudCleanup = true)
        assertEquals(dev.flowclone.core.CleanupLevel.LIGHT, cfg.cleanupLevel); assertFalse(cfg.rawMode)
        assertEquals(dev.flowclone.core.CleanupLevel.HIGH,
            st.copy(llmEnabled = false, cleanup = dev.flowclone.core.CleanupLevel.HIGH).toSessionConfig(emptyList(), cloudCleanup = true).cleanupLevel)
    }

    // ---- D52 custom cleanup server ----

    private val server = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "gpt-4o-mini")

    @Test fun customProviderIsSelectableOnlyOnceReady() {
        assertNull(OpenRouterPage.withProvider(base, CleanupProvider.CUSTOM))
        assertEquals("Set the server address first", OpenRouterPage.customMissing(base.cloud.customCleanup))
        val url = OpenRouterPage.withCustom(base) { copy(baseUrl = server.baseUrl) }
        assertEquals("Set the model first", OpenRouterPage.customMissing(url.cloud.customCleanup))
        val ready = OpenRouterPage.withCustom(url) { copy(model = server.model) }
        assertNull(OpenRouterPage.customMissing(ready.cloud.customCleanup))
        val custom = OpenRouterPage.withProvider(ready, CleanupProvider.CUSTOM)!!
        assertTrue(custom.cloud.customProvider)
        assertFalse(OpenRouterPage.withProvider(custom, CleanupProvider.OPENROUTER)!!.cloud.customProvider)
    }

    @Test fun customConsentIsItsOwnAndFollowsTheHost() {
        val s = base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server))
        assertTrue(OpenRouterPage.cleanupNeedsConsent(s))
        val on = OpenRouterPage.enableCleanup(s, now)
        assertTrue(on.allowCloudCleanup); assertTrue(on.cloud.cleanupConsentOk)
        assertEquals("10.0.0.5", on.cloud.customCleanup.consentHost)
        assertFalse("OpenRouter's consent is untouched", on.cloud.cleanup.consentOk)
        assertTrue(on.offPhoneCleanup)
        val moved = OpenRouterPage.withCustom(on) { copy(baseUrl = "http://10.0.0.9:4000") }
        assertTrue(OpenRouterPage.cleanupNeedsConsent(moved)); assertFalse(moved.offPhoneCleanup)
        assertEquals("Given on D for 10.0.0.5", OpenRouterPage.cleanupConsentLine(on.cloud) { "D" })
        assertEquals("Given on D for openrouter.ai", OpenRouterPage.cleanupConsentLine(OpenRouterPage.enableCleanup(base, now).cloud) { "D" })
    }

    @Test fun customSummariesNameTheServerAndNeedNoKey() {
        val s = OpenRouterPage.enableCleanup(base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server)), now)
        assertEquals("Speech off · Cleanup on (gpt-4o-mini on 10.0.0.5)", OpenRouterPage.settingsSummary(s, hasKey = false))
        assertTrue(OpenRouterPage.cleanupEngineLine(s)!!.contains("Custom server (10.0.0.5)"))
        val or = OpenRouterPage.enableCleanup(base, now)
        assertTrue(OpenRouterPage.settingsSummary(or, hasKey = false).endsWith("· no key"))
    }
}
