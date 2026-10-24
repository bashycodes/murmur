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

        val cl = OpenRouterPage.enableCleanup(base, now, BuildCaps.ALL)
        assertTrue(cl.allowCloudCleanup); assertTrue(cl.cloud.cleanup.consentOk)
        assertEquals(listOf(CleanupRoute.CLOUD, CleanupRoute.LOCAL), cl.cleanupOrder)
        assertFalse("cleanup never turns STT on", cl.allowCloud); assertFalse(cl.cloud.stt.consentOk)
        assertTrue(cl.cloudCleanupRouted)

        // Off keeps the consent: on again asks nothing.
        val off = OpenRouterPage.disableCleanup(cl)
        assertFalse(off.allowCloudCleanup); assertFalse(OpenRouterPage.cleanupNeedsConsent(off))
        assertEquals(cl.cloud.cleanup.consentAtMs, OpenRouterPage.enableCleanup(off, now + 5, BuildCaps.ALL).cloud.cleanup.consentAtMs)
        assertFalse(OpenRouterPage.disableStt(stt).allowCloud)
        assertEquals(stt.routeOrder, OpenRouterPage.disableStt(stt).routeOrder)
    }

    @Test fun cleanupOrder() {
        // A build without the on-device LLM (remote): cloud only.
        assertEquals(listOf(CleanupRoute.CLOUD), OpenRouterPage.enableCleanup(base, now, BuildCaps.REMOTE).cleanupOrder)
        val cl = OpenRouterPage.enableCleanup(base, now, BuildCaps.ALL)
        val phoneFirst = OpenRouterPage.withCleanupOrder(cl, cloudFirst = false)
        assertEquals(listOf(CleanupRoute.LOCAL, CleanupRoute.CLOUD), phoneFirst.cleanupOrder)
        assertFalse(OpenRouterPage.cleanupCloudFirst(phoneFirst))
        assertEquals("On-device, then OpenRouter (llama-3.3-70b-instruct)", OpenRouterPage.cleanupEngineLine(phoneFirst))
        assertEquals("OpenRouter (llama-3.3-70b-instruct), then on-device", OpenRouterPage.cleanupEngineLine(cl))
        // A chosen order survives off → on.
        assertEquals(phoneFirst.cleanupOrder, OpenRouterPage.enableCleanup(OpenRouterPage.disableCleanup(phoneFirst), now, BuildCaps.ALL).cleanupOrder)
        assertNull(OpenRouterPage.cleanupEngineLine(base))
    }

    @Test fun routerJsonCarriesBothUses() {
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(base, now), now, BuildCaps.ALL)
        val j = JSONObject(s.routerConfigFor(BuildCaps.ALL))
        assertTrue(j.getBoolean("allow_cloud")); assertTrue(j.getBoolean("allow_cloud_cleanup"))
        assertEquals("[\"cloud\",\"local\"]", j.getJSONArray("cleanup_order").toString())
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
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(base, now), now, BuildCaps.ALL)
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
}
