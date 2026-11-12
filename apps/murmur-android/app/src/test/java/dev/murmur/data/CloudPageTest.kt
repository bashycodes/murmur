package dev.murmur.data

import dev.murmur.data.DevSettings.CleanupRoute
import dev.murmur.data.DevSettings.Route
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D28 page logic: two independent opt-ins with their own consents, order, models, privacy. */
class CloudPageTest {
    @org.junit.Before fun fakeRouterAddress() = FakeCustomAddress.install()

    private val base = DevSettings.defaults(BuildCaps.ALL)
    private val now = 1_792_540_800_000L
    /** The AI cleanup summary without the rules ("Light · …"), null when no AI engine runs. */
    private fun line(d: DevSettings, phoneAi: Boolean) =
        SettingsSummary.cleanupRow(Settings(llmEnabled = phoneAi, spokenCleanup = false), d, BuildCaps.ALL).takeUnless { it == "Off" }

    @Test fun sttAndCleanupAreIndependent() {
        val stt = CloudPage.enableStt(base, now)
        assertTrue(stt.allowCloud); assertTrue(stt.cloud.stt.consentOk)
        assertEquals(listOf(Route.CLOUD, Route.SERVER, Route.CLIENT), stt.routeOrder)
        assertFalse("STT never turns cleanup on", stt.allowCloudCleanup)
        assertFalse(stt.cloud.cleanup.consentOk)
        assertTrue(CloudPage.cleanupNeedsConsent(stt))

        val cl = CloudPage.enableCleanup(base, now)
        assertTrue(cl.allowCloudCleanup); assertTrue(cl.cloud.cleanup.consentOk)
        // D36: switched on where it stands in the order (default: last).
        assertEquals(listOf(CleanupRoute.LOCAL, CleanupRoute.SERVER, CleanupRoute.CLOUD), cl.cleanupOrder)
        assertFalse("cleanup never turns STT on", cl.allowCloud); assertFalse(cl.cloud.stt.consentOk)
        assertTrue(cl.cloudCleanupRouted)

        // Off keeps the consent: on again asks nothing.
        val off = CloudPage.disableCleanup(cl)
        assertFalse(off.allowCloudCleanup); assertFalse(CloudPage.cleanupNeedsConsent(off))
        assertEquals(cl.cloud.cleanup.consentAtMs, CloudPage.enableCleanup(off, now + 5).cloud.cleanup.consentAtMs)
        assertFalse(CloudPage.disableStt(stt).allowCloud)
        assertEquals(stt.routeOrder, CloudPage.disableStt(stt).routeOrder)
    }

    @Test fun cleanupOrder() {
        val cl = CloudPage.enableCleanup(base, now)
        val cloudFirst = cl.copy(cleanupOrder = listOf(CleanupRoute.CLOUD, CleanupRoute.LOCAL, CleanupRoute.SERVER))
        assertEquals("Light · This phone, then cloud", line(cl, phoneAi = true))
        assertEquals("Light · Cloud, then this phone", line(cloudFirst, phoneAi = true))
        // A configured server is named; a switched-off engine is not.
        val withServer = cl.copy(host = "100.64.0.10")
        assertEquals("Light · This phone, then your server, then cloud", line(withServer, phoneAi = true))
        assertEquals("Light · Your server, then cloud", line(withServer.withCleanup(CleanupRoute.LOCAL, false), phoneAi = true))
        // A chosen order survives off → on.
        assertEquals(cloudFirst.cleanupOrder, CloudPage.enableCleanup(CloudPage.disableCleanup(cloudFirst), now).cleanupOrder)
        assertNull(line(base, phoneAi = false))
    }

    @Test fun routerJsonCarriesBothUses() {
        val s = CloudPage.enableCleanup(CloudPage.enableStt(base, now), now)
        val j = JSONObject(s.routerConfigFor(BuildCaps.ALL))
        assertTrue(j.getBoolean("allow_cloud")); assertTrue(j.getBoolean("allow_cloud_cleanup"))
        assertEquals("[\"local\",\"server\",\"cloud\"]", j.getJSONArray("cleanup_order").toString())
        assertEquals("[]", j.getJSONArray("cleanup_off").toString())
        assertEquals(2, j.getJSONObject("cloud").getInt("schema"))
        assertEquals("openrouter.ai", j.getJSONObject("cloud").getJSONObject("openrouter").getJSONObject("cleanup").getString("consent_host"))
    }

    @Test fun modelsAndPrivacy() {
        val s = CloudPage.withModel(base, CloudCatalog.Use.STT, " google/gemini-3.5-flash-lite ", "chat_audio")
        assertEquals("google/gemini-3.5-flash-lite", s.cloud.stt.model); assertEquals("chat_audio", s.cloud.stt.sttPath)
        val c = CloudPage.withModel(base, CloudCatalog.Use.CLEANUP, "meta-llama/llama-3.1-8b-instruct:nitro")
        assertEquals("meta-llama/llama-3.1-8b-instruct:nitro", CloudPage.model(c, CloudCatalog.Use.CLEANUP))
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, CloudPage.model(c, CloudCatalog.Use.STT))
        assertTrue(runCatching { CloudPage.withModel(base, CloudCatalog.Use.STT, "Bad Id") }.isFailure)
        val p = CloudPage.withPrivacy(base, zdr = false)
        assertFalse(p.cloud.privacy.zdr); assertTrue("deny stays unless turned off too", p.cloud.privacy.denyDataCollection)
        assertEquals("allow", CloudPage.withPrivacy(p, denyCollection = false).cloud.privacy.dataCollection)
    }

    @Test fun summaries() {
        assertEquals("Speech off · Cleanup off", CloudPage.settingsSummary(base, hasKey = false))
        val s = CloudPage.enableCleanup(CloudPage.enableStt(base, now), now)
        assertEquals("Speech on · Cleanup on (llama-3.3-70b-instruct) · no key", CloudPage.settingsSummary(s, hasKey = false))
        assertEquals("Speech on · Cleanup on (llama-3.3-70b-instruct)", CloudPage.settingsSummary(s, hasKey = true))
        assertEquals("Speech on · Cleanup on · budget \$5.00",
            DevSummary.cloud(s.copy(cloud = s.cloud.copy(monthlyBudgetUsd = 5.0)), BuildCaps.ALL, hasKey = true))
        assertEquals("Speech off · Cleanup off · no key", DevSummary.cloud(base, BuildCaps.ALL, hasKey = false))
        assertEquals("Not in this build", DevSummary.cloud(base, BuildCaps.LOCAL, hasKey = false))
        assertFalse(CloudPage.mayRefreshInBackground(base)); assertTrue(CloudPage.mayRefreshInBackground(s))
    }

    @Test fun cloudCleanupRunsTheLlmStepEvenWithTheLocalAiOff() {
        val st = Settings()
        assertEquals(dev.murmur.core.CleanupLevel.NONE, st.copy(llmEnabled = false).toSessionConfig(emptyList()).cleanupLevel)
        val cfg = st.copy(llmEnabled = false).toSessionConfig(emptyList(), cloudCleanup = true)
        assertEquals(dev.murmur.core.CleanupLevel.LIGHT, cfg.cleanupLevel); assertFalse(cfg.rawMode)
        assertEquals(dev.murmur.core.CleanupLevel.HIGH,
            st.copy(llmEnabled = false, cleanup = dev.murmur.core.CleanupLevel.HIGH).toSessionConfig(emptyList(), cloudCleanup = true).cleanupLevel)
    }

    // ---- D52 custom cleanup server ----

    private val server = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "gpt-4o-mini")

    @Test fun liteLlmIsSavedWhenPickedAndSaysWhatItStillNeeds() {
        // D78: the switch shows what dictation uses; an unready proxy sends nothing (routerJson).
        val picked = CloudPage.withProvider(base, CleanupProvider.CUSTOM)
        assertTrue(picked.cloud.customProvider); assertTrue(picked.cloud.customNotSetUp)
        assertFalse(picked.cloud.routerJson().has("cleanup_provider"))
        assertEquals("Set the proxy address first", CloudPage.customMissing(picked.cloud.customCleanup))
        val url = CloudPage.withCustom(picked) { copy(baseUrl = server.baseUrl) }
        assertEquals("Set the model first", CloudPage.customMissing(url.cloud.customCleanup))
        val ready = CloudPage.withCustom(url) { copy(model = server.model) }
        assertNull(CloudPage.customMissing(ready.cloud.customCleanup))
        assertFalse(ready.cloud.customNotSetUp)
        assertEquals("custom", ready.cloud.routerJson().getString("cleanup_provider"))
        val back = CloudPage.withProvider(ready, CleanupProvider.OPENROUTER)
        assertFalse(back.cloud.customProvider)
        assertEquals("the proxy's fields are kept", server.baseUrl, back.cloud.customCleanup.baseUrl)
    }

    // ---- D78 one Cloud provider at a time ----

    @Test fun pickingLiteLlmTurnsCloudSpeechOffAndKeepsItsConsent() {
        val both = CloudPage.enableStt(base, now)
        assertTrue(CloudPage.sttAvailable(both))
        val ll = CloudPage.withProvider(both, CleanupProvider.CUSTOM)
        assertFalse(ll.allowCloud); assertFalse(CloudPage.sttAvailable(ll))
        assertTrue("OpenRouter's speech consent is kept", ll.cloud.stt.consentOk)
        assertEquals("the route order is kept", both.routeOrder, ll.routeOrder)
        // The speech switch refuses: enableStt changes nothing while LiteLLM is the provider.
        assertEquals(ll, CloudPage.enableStt(ll, now + 1))
        // Back to OpenRouter: speech stays off until turned on again, then asks nothing.
        val or = CloudPage.withProvider(ll, CleanupProvider.OPENROUTER)
        assertFalse(or.allowCloud); assertTrue(CloudPage.enableStt(or, now + 2).allowCloud)
        assertFalse(CloudPage.sttNeedsConsent(or))
    }

    @Test fun aSavedMixNeverReachesTheRouterOrTheStore() {
        val mix = base.copy(allowCloud = true, routeOrder = DevSettings.withCloudFirst(base.routeOrder),
            cloud = CloudPage.enableStt(base, now).cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server))
        assertEquals(mix.copy(allowCloud = false), CloudPage.oneProvider(mix))
        assertFalse("the router gets no cloud speech", mix.effective(BuildCaps.ALL).allowCloud)
        assertFalse(mix.effective(BuildCaps.ALL).cloudRouted)
        // The store never writes the mix, whichever save is used.
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("murmur_dev_token", FakeVaultKeys()))
        store.save(mix)
        assertFalse(store.load().allowCloud)
        store.saveCloud(allowCloud = true, cloud = mix.cloud)
        assertFalse(store.load().allowCloud)
        store.saveCloud(allowCloud = true, cloud = CloudSettings())
        assertTrue("OpenRouter speech is allowed again", store.load().allowCloud)
    }

    @Test fun aMixSavedBeforeD78IsFixedOnLoadWithOneNotice() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("murmur_dev_token", FakeVaultKeys()))
        val cloud = CloudPage.enableStt(base, now).cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server)
        // What an older app left: both switches on (written raw, as the old store did).
        kv.edit { putBoolean("allow_cloud", true); putString("cloud", cloud.toJson().toString()) }
        assertTrue(store.load().allowCloud)
        assertTrue(CloudMigration.fixOneProvider(store))
        val s = store.load()
        assertFalse(s.allowCloud); assertTrue(s.cloud.customProvider); assertTrue(s.cloud.stt.consentOk)
        assertEquals(CloudText.LITELLM_STT_OFF_NOTICE, store.cloudNotice)
        // Once: nothing to fix next time, the notice stays until dismissed.
        store.cloudNotice = null
        assertFalse(CloudMigration.fixOneProvider(store))
        assertNull(store.cloudNotice)
        // OpenRouter with speech on is left alone.
        store.saveCloud(allowCloud = true, cloud = cloud.copy(cleanupProvider = CleanupProvider.OPENROUTER))
        assertFalse(CloudMigration.fixOneProvider(store)); assertTrue(store.load().allowCloud)
    }

    @Test fun theOneProviderNoticeKeepsANoticeNotYetDismissed() {
        val kv = MemKV()
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("murmur_dev_token", FakeVaultKeys()))
        val cloud = CloudPage.enableStt(base, now).cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server)
        kv.edit { putBoolean("allow_cloud", true); putString("cloud", cloud.toJson().toString()) }
        // A D28 migration notice written just before, on the same start.
        store.addCloudNotice("Synthetic migration notice.")
        assertTrue(CloudMigration.fixOneProvider(store))
        assertEquals("Synthetic migration notice.\n\n" + CloudText.LITELLM_STT_OFF_NOTICE, store.cloudNotice)
        // The router's notice later is added too, and a repeat is not doubled.
        store.addCloudNotice("Synthetic router notice.")
        store.addCloudNotice(CloudText.LITELLM_STT_OFF_NOTICE)
        assertEquals(
            listOf("Synthetic migration notice.", CloudText.LITELLM_STT_OFF_NOTICE, "Synthetic router notice."),
            store.cloudNotice!!.split(DevStore.NOTICE_SEP),
        )
    }

    @Test fun theOneProviderRuleLivesOnTheSettings() {
        val ll = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM)
        assertTrue(base.cloud.sttAvailable); assertFalse(ll.sttAvailable)
        assertEquals(true, base.cloud.sttAllowed(true)); assertNull(base.cloud.sttAllowed(null))
        assertEquals(false, ll.sttAllowed(true)); assertEquals(false, ll.sttAllowed(null))
        val mix = CloudPage.enableStt(base, now).copy(cloud = ll)
        assertEquals(mix.oneCloudProvider(), CloudPage.oneProvider(mix))
        assertFalse(mix.effective(BuildCaps.ALL).allowCloud)
    }

    @Test fun customConsentIsItsOwnAndFollowsTheHost() {
        val s = base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server))
        assertTrue(CloudPage.cleanupNeedsConsent(s))
        val on = CloudPage.enableCleanup(s, now)
        assertTrue(on.allowCloudCleanup); assertTrue(on.cloud.cleanupConsentOk)
        assertEquals("10.0.0.5", on.cloud.customCleanup.consentHost)
        assertFalse("OpenRouter's consent is untouched", on.cloud.cleanup.consentOk)
        assertTrue(on.offPhoneCleanup)
        val moved = CloudPage.withCustom(on) { copy(baseUrl = "http://10.0.0.9:4000") }
        assertTrue(CloudPage.cleanupNeedsConsent(moved)); assertFalse(moved.offPhoneCleanup)
        assertEquals("Given on D for 10.0.0.5", CloudPage.cleanupConsentLine(on.cloud) { "D" })
        assertEquals("Given on D for openrouter.ai", CloudPage.cleanupConsentLine(CloudPage.enableCleanup(base, now).cloud) { "D" })
    }

    @Test fun customSummariesNameTheServerAndNeedNoKey() {
        val s = CloudPage.enableCleanup(base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server)), now)
        assertEquals("Speech off · Cleanup on (gpt-4o-mini on 10.0.0.5)", CloudPage.settingsSummary(s, hasKey = false))
        assertEquals("Light · Cloud", line(s, phoneAi = false))
        val or = CloudPage.enableCleanup(base, now)
        assertTrue(CloudPage.settingsSummary(or, hasKey = false).endsWith("· no key"))
    }

    @Test fun anUnusableCustomServerIsShownAsOffNotAsOpenRouter() {
        val on = CloudPage.enableCleanup(base.copy(cloud = base.cloud.copy(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = server)), now)
        val broken = CloudPage.withCustom(on) { copy(model = "") }
        assertTrue(broken.cloud.customNotSetUp)
        assertEquals("Speech off · Cleanup off (LiteLLM not set up)", CloudPage.settingsSummary(broken, hasKey = true))
        assertNull(line(broken, phoneAi = false))
        assertFalse(CloudPage.settingsSummary(broken, hasKey = true).contains("OpenRouter"))
    }
}
