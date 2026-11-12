package dev.murmur.data

import dev.murmur.core.RouteInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** D28: the one OpenRouter config (schema 2), its two consents, the key at rest and the wording. */
class CloudSettingsTest {
    private val now = 1_792_540_800_000L

    @org.junit.Before fun fakeRouterAddress() = FakeCustomAddress.install()

    @Test fun defaultsAreThePrivateOnes() {
        val c = CloudSettings()
        assertTrue(c.privacy.zdr); assertEquals("deny", c.privacy.dataCollection)
        assertEquals(OpenRouter.DEFAULT_STT_MODEL, c.stt.model); assertEquals("openai/whisper-large-v3-turbo", c.stt.model)
        assertEquals(OpenRouter.DEFAULT_CLEANUP_MODEL, c.cleanup.model)
        assertEquals(2_500L, c.cleanup.timeoutMs); assertEquals("latency", c.cleanup.sort)
        assertFalse(c.cleanup.sendSurroundingText || c.cleanup.sendScreenWords || c.cleanup.sendAppName)
        assertFalse(c.stt.consentOk); assertFalse(c.cleanup.consentOk)
        assertNull(c.monthlyBudgetUsd)
        assertEquals(2, c.toJson().getInt("schema"))
    }

    @Test fun consentsAreSeparateAndNamedForOpenRouter() {
        val c = CloudSettings()
        val stt = c.copy(stt = c.stt.withConsent(now))
        assertTrue(stt.stt.consentOk); assertFalse("STT consent never counts for cleanup", stt.cleanup.consentOk)
        assertEquals("openrouter.ai", stt.stt.consentHost)
        val both = stt.copy(cleanup = stt.cleanup.withConsent(now + 1))
        assertTrue(both.cleanup.consentOk)
        assertFalse(both.copy(stt = both.stt.withoutConsent()).stt.consentOk)
        // A consent recorded for another host (a hand-edited prefs file) is no consent.
        assertFalse(c.stt.copy(consentHost = "example.com", consentAtMs = now).consentOk)
    }

    @Test fun jsonRoundTripAndLegacyReadsAsDefault() {
        val c = CloudSettings(
            privacy = CloudPrivacy(zdr = false, dataCollection = "allow"),
            stt = SttSettings(model = "elevenlabs/scribe-v2", language = "de", sendDictionary = false).withConsent(now),
            cleanup = CleanupSettings(model = "meta-llama/llama-3.1-8b-instruct", timeoutMs = 4000, sendAppName = true).withConsent(now),
            monthlyBudgetUsd = 2.5,
        )
        assertEquals(c, CloudSettings.fromJson(JSONObject(c.toJson().toString())))
        val legacy = JSONObject("""{"active":"groq-1","providers":[{"id":"groq-1","adapter":"openai_compat"}],"monthly_budget_usd":5}""")
        assertTrue(CloudSettings.isLegacy(legacy))
        assertEquals(CloudSettings(), CloudSettings.fromJson(legacy))
        assertTrue("no schema = 1", CloudSettings.isLegacy(JSONObject("""{"monthly_budget_usd":5}""")))
        assertFalse(CloudSettings.isLegacy(c.toJson()))
        // openRouterJson is the OpenRouterConfig nativeProbeCloud takes: no host, no key.
        val or = c.openRouterJson()
        assertTrue(or.has("stt") && or.has("cleanup") && or.has("privacy"))
        assertFalse(or.toString().contains("base_url"))
    }

    @Test fun modelIdRuleMatchesTheRouter() {
        for (ok in listOf("openai/whisper-large-v3-turbo", "meta-llama/llama-3.3-70b-instruct:nitro", "x/y.z~1_2-3", "a")) assertTrue(ok, OpenRouter.validModelId(ok))
        for (bad in listOf("", "Openai/Whisper", "a b", "x/y:Nitro", "x/y:", "x".repeat(129), "https://openrouter.ai/x")) assertFalse(bad, OpenRouter.validModelId(bad))
        assertNull(OpenRouter.modelIdError(" openai/whisper-large-v3 "))
        assertEquals("Model ids are lower case", OpenRouter.modelIdError("OpenAI/x"))
    }

    @Test fun inputs() {
        assertEquals(2.5, CloudInput.budget("$2.50").value!!, 0.0)
        assertNull(CloudInput.budget(" ").value); assertNull(CloudInput.budget(" ").error)
        assertTrue(CloudInput.budget("-1").error != null)
        assertEquals("5", CloudInput.budgetText(5.0)); assertEquals("2.5", CloudInput.budgetText(2.5)); assertEquals("", CloudInput.budgetText(null))
        assertEquals(800L, CloudInput.timeout("800").value); assertTrue(CloudInput.timeout("799").error != null); assertTrue(CloudInput.timeout("8001").error != null)
        assertEquals("pt-br", CloudInput.language(" PT-BR ").value); assertTrue(CloudInput.language("english").error != null)
    }

    @Test fun consentDialogsSayWhatLeaves() {
        val stt = CloudText.sttConsentBody(SttSettings())
        assertTrue(stt, stt.contains("audio") && stt.contains("openrouter.ai") && stt.contains("dictionary words"))
        assertFalse(CloudText.sttConsentBody(SttSettings(sendDictionary = false)).contains("dictionary"))
        val cl = CloudText.cleanupConsentBody(CleanupSettings())
        // Spec §2.5 wording.
        assertTrue(cl, cl.startsWith("After each dictation, the text (and your dictionary words) is sent to OpenRouter (openrouter.ai)"))
        assertTrue(cl.contains("Your voice is not sent unless Cloud speech-to-text is also on"))
        assertFalse("minimal scope by default", cl.contains("around the cursor") || cl.contains("on your screen") || cl.contains("app's name"))
        val all = CloudText.cleanupSends(CleanupSettings(sendSurroundingText = true, sendScreenWords = true, sendAppName = true))
        assertTrue(all, all.contains("the text around the cursor") && all.contains("words on your screen") && all.endsWith("and the app's name"))
    }

    @Test fun usageIsSplitByPurpose() {
        val u = CloudUsage.decode(javaClass.classLoader!!.getResource("cloud_usage.json")!!.readText())
        assertEquals(0.0234, u.openRouterCostFor(CloudUsage.PURPOSE_STT), 1e-9)
        assertEquals(0.014, u.openRouterCostFor(CloudUsage.PURPOSE_CLEANUP), 1e-9)
        assertTrue(u.openRouterEstimated(CloudUsage.PURPOSE_CLEANUP)); assertFalse(u.openRouterEstimated(CloudUsage.PURPOSE_STT))
        val lines = CloudText.usageLines(u, CloudSettings(monthlyBudgetUsd = 5.0))
        assertEquals("Speech \$0.02 · Cleanup ≈ \$0.01 · this month (2026-10)", lines[0])
        assertEquals("42 speech requests (5.2 min) · 20 cleanup requests", lines[1])
        assertEquals("Budget \$5.00/month: \$0.04 used", lines[2])
        // A pre-D28 entry has no purpose: speech.
        assertEquals(CloudUsage.PURPOSE_STT, CloudUsage.decode("""{"month":"m","providers":[{"provider_id":"x","cost_usd":1}]}""").providers.single().purpose)
        assertEquals(u, CloudUsage.decode(u.toJson().toString()))
    }

    @Test fun customServerCostIsShownApartFromTheBudget() {
        val u = CloudUsage("2026-11", listOf(
            CloudUsage.ProviderUsage(OpenRouter.ID, requests = 3, costUsd = 0.5, purpose = CloudUsage.PURPOSE_STT),
            CloudUsage.ProviderUsage(CustomServer.ID, requests = 9, costUsd = 4.0, purpose = CloudUsage.PURPOSE_CLEANUP),
        ))
        assertEquals(0.5, u.budgetCostUsd, 1e-9); assertEquals(4.0, u.customCostUsd, 1e-9)
        val lines = CloudText.usageLines(u, CloudSettings(monthlyBudgetUsd = 1.0))
        assertEquals("Speech \$0.50 · Cleanup \$0 · this month (2026-11)", lines[0])
        assertEquals("3 speech requests (0.0 min) · 0 cleanup requests", lines[1])
        assertEquals("Budget \$1.00/month: \$0.50 used", lines[2])
        assertEquals("Your proxy: 9 cleanup requests · reported \$4.00 (not in the budget)", lines[3])
        // The page header: OpenRouter only, against the budget.
        assertEquals(listOf("\$0.50" to "this month of \$1.00", "\$0.50" to "speech", "\$0" to "cleanup"),
            CloudText.usageStats(u, CloudSettings(monthlyBudgetUsd = 1.0)))
        val st = dev.murmur.core.RouterJson.decodeCleanupStatus(
            """{"supported":true,"enabled":true,"provider":"custom","host":"10.0.0.5","model":"m","month_cost_usd":4.0,"custom_month_cost_usd":4.0}""")
        assertEquals(4.0, st.customMonthCostUsd, 1e-9)
        val line = dev.murmur.core.DevText.cleanupStatus(st)
        assertEquals("On · LiteLLM (10.0.0.5) · m · \$4.00 on your server this month (not in budget)", line)
        // Both providers this month: OpenRouter's part and the server's, never added together.
        val both = dev.murmur.core.DevText.cleanupStatus(st.copy(monthCostUsd = 4.5))
        assertEquals("On · LiteLLM (10.0.0.5) · m · OpenRouter this month \$0.50 · \$4.00 on your server this month (not in budget)", both)
        val or = dev.murmur.core.DevText.cleanupStatus(st.copy(provider = "openrouter", host = "", monthCostUsd = 4.5))
        assertEquals("On · m · this month \$0.50 · \$4.00 on your server this month (not in budget)", or)
    }

    @Test fun customServerRequestsCountWithoutACost() {
        // A server that sends no cost header still shows its requests; OpenRouter's cleanup stays its own.
        val u = CloudUsage("2026-11", listOf(
            CloudUsage.ProviderUsage(OpenRouter.ID, requests = 2, costUsd = 0.25, costEstimated = true, purpose = CloudUsage.PURPOSE_CLEANUP),
            CloudUsage.ProviderUsage(CustomServer.ID, requests = 1, purpose = CloudUsage.PURPOSE_CLEANUP),
        ))
        val lines = CloudText.usageLines(u, CloudSettings())
        assertEquals("Speech \$0 · Cleanup ≈ \$0.25 · this month (2026-11)", lines[0])
        assertEquals("0 speech requests (0.0 min) · 2 cleanup requests", lines[1])
        assertEquals("Your proxy: 1 cleanup request · no cost reported (not in the budget)", lines.last())
        assertEquals(listOf("\$0.25" to "this month", "\$0" to "speech", "≈\$0.25" to "cleanup"), CloudText.usageStats(u, CloudSettings()))
        assertTrue("no custom line without custom usage", CloudText.usageLines(CloudUsage(), CloudSettings()).none { it.startsWith(CustomServer.USAGE_LABEL) })
    }

    @Test fun routeWording() {
        val r = RouteInfo("cloud", provider = "openrouter", model = "openai/whisper-large-v3-turbo", costUsd = 0.000011, llmEngine = "cloud",
            llmModel = "meta-llama/llama-3.3-70b-instruct", llmCostUsd = 0.0007)
        assertEquals("cloud · OpenRouter · openai/whisper-large-v3-turbo · \$0.0000", CloudText.engineRow(r))
        assertEquals("AI cleanup: OpenRouter · llama-3.3-70b-instruct · \$0.0007", CloudText.cleanupRow(r))
        assertNull(CloudText.cleanupRow(RouteInfo("local")))
        // D52: named by the provider that cleaned.
        assertEquals("AI cleanup: LiteLLM (10.0.0.5) · llama-3.3-70b-instruct · \$0.0007", CloudText.cleanupRow(r, "LiteLLM (10.0.0.5)"))
        // D30: AI cleanup tried, the fallback pipeline produced the text.
        assertEquals("AI cleanup: on-device · qwen2.5-1.5b · OpenRouter not used (llm_timeout)",
            CloudText.cleanupRow(RouteInfo("local", llmEngine = "local", llmModel = "qwen2.5-1.5b", cleanupRoute = "local", cloudFallback = "llm_timeout")))
        assertEquals("AI cleanup: fixed rules · OpenRouter not used (llm_empty) · \$0.0016",
            CloudText.cleanupRow(RouteInfo("local", llmEngine = "cloud", llmModel = "meta-llama/llama-3.3-70b-instruct", llmCostUsd = 0.0016,
                cleanupRoute = "rules", cloudFallback = "llm_empty")))
        assertEquals("AI cleanup: OpenRouter · llama-3.3-70b-instruct · \$0.0016",
            CloudText.cleanupRow(RouteInfo("local", llmEngine = "cloud", llmModel = "meta-llama/llama-3.3-70b-instruct", llmCostUsd = 0.0016, cleanupRoute = "cloud")))
        assertEquals("When your server is unreachable, audio goes to OpenRouter.",
            CloudText.orderWarning(listOf(DevSettings.Route.SERVER, DevSettings.Route.CLOUD)))
        assertNull(CloudText.orderWarning(listOf(DevSettings.Route.CLOUD, DevSettings.Route.SERVER)))
    }

    // ---- the key at rest (C3) ----

    private val key = "sk-or-LEAKCHECK-android-0123456789abcd"

    @Test fun oneKeySealedAndNeverInSettings() {
        val kv = MemKV()
        val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        val store = DevStore(kv, BuildCaps.ALL, SecretVault("murmur_dev_token", fake))
        val s = OpenRouterPage.enableCleanup(OpenRouterPage.enableStt(DevSettings.defaults(BuildCaps.ALL), now), now)
            .let { it.copy(cloud = it.cloud.copy(monthlyBudgetUsd = 3.0)) }
        store.save(s)
        keys.set(OpenRouter.ID, " $key ")
        assertEquals(key, keys.get(OpenRouter.ID))
        assertEquals("••••abcd", keys.masked(OpenRouter.ID))
        assertEquals(s, store.load())
        assertFalse(kv.m.values.any { it.toString().contains("LEAKCHECK") })
        assertFalse(s.toRouterConfigJson().toString().contains("LEAKCHECK"))
        assertFalse(s.toString().contains("LEAKCHECK"))
        assertEquals(setOf("murmur_cloud_openrouter"), fake.keys.keys.filter { it.startsWith("murmur_cloud_") }.toSet())
        keys.set(OpenRouter.ID, "")
        assertNull(keys.get(OpenRouter.ID)); assertFalse("murmur_cloud_openrouter" in fake.keys)
    }

    @Test fun moveReSealsUnderTheNewVaultAndDestroysTheOld() {
        val kv = MemKV(); val fake = FakeVaultKeys()
        val keys = CloudKeyStore(kv) { id -> SecretVault(SecretVault.cloudAlias(id), fake) }
        keys.set("openrouter-1", key)
        keys.move("openrouter-1", OpenRouter.ID)
        assertEquals(key, keys.get(OpenRouter.ID))
        assertFalse(keys.has("openrouter-1")); assertFalse("murmur_cloud_openrouter-1" in fake.keys)
        keys.move("openrouter-1", OpenRouter.ID) // idempotent
        assertEquals(key, keys.get(OpenRouter.ID))
    }

    @Test fun storeKeepsCleanupFieldsAndALocalBuildMasksThem() {
        val kv = MemKV(); val fake = FakeVaultKeys()
        val s = OpenRouterPage.enableCleanup(DevSettings.defaults(BuildCaps.ALL), now)
            .copy(cleanupOrder = listOf(DevSettings.CleanupRoute.CLOUD, DevSettings.CleanupRoute.SERVER, DevSettings.CleanupRoute.LOCAL))
            .withCleanup(DevSettings.CleanupRoute.SERVER, false)
        DevStore(kv, BuildCaps.ALL, SecretVault("t", fake)).save(s)
        val back = DevStore(kv, BuildCaps.ALL, SecretVault("t", fake)).load()
        assertEquals(s.cleanupOrder, back.cleanupOrder); assertEquals(setOf(DevSettings.CleanupRoute.SERVER), back.cleanupOff)
        assertTrue(back.allowCloudCleanup)
        val local = DevStore(kv, BuildCaps.LOCAL, SecretVault("t", fake)).load()
        assertFalse(local.allowCloudCleanup)
        // …and a local build's router JSON carries no cloud field at all (C2).
        val j = JSONObject(local.routerConfigFor(BuildCaps.LOCAL))
        for (k in listOf("cloud", "allow_cloud", "allow_cloud_cleanup", "cleanup_order", "order")) assertFalse(k, j.has(k))
    }

    // ---- D52 custom cleanup server ----

    private val custom = CustomCleanupSettings(baseUrl = "http://10.0.0.5:4000/v1", model = "Qwen/Qwen2.5-7B-Instruct")

    @Test fun customDefaultsAreOmittedFromJson() {
        val j = CloudSettings().toJson()
        assertFalse(j.has("cleanup_provider")); assertFalse(j.has("custom_cleanup"))
        assertEquals(CloudSettings(), CloudSettings.fromJson(j))
        assertEquals(CleanupProvider.OPENROUTER, CloudSettings.fromJson(JSONObject("""{"schema":2}""")).cleanupProvider)
        assertEquals(4_000L, CustomCleanupSettings().timeoutMs); assertEquals(1_500L, CustomCleanupSettings().connectTimeoutMs)
        // An entry kept while OpenRouter is selected is written (the provider is not).
        val kept = CloudSettings(customCleanup = custom).toJson()
        assertFalse(kept.has("cleanup_provider")); assertEquals("Qwen/Qwen2.5-7B-Instruct", kept.getJSONObject("custom_cleanup").getString("model"))
    }

    @Test fun customRoundTripsWithTheRouterNames() {
        val c = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom.copy(timeoutMs = 9_000).withConsent(now))
        val j = c.toJson()
        assertEquals("custom", j.getString("cleanup_provider"))
        val cc = j.getJSONObject("custom_cleanup")
        assertEquals("http://10.0.0.5:4000/v1", cc.getString("base_url"))
        assertEquals(9_000L, cc.getLong("timeout_ms")); assertEquals(1_500L, cc.getLong("connect_timeout_ms"))
        assertEquals("10.0.0.5", cc.getString("consent_host")); assertEquals(now, cc.getLong("consent_at_ms"))
        assertEquals(c, CloudSettings.fromJson(JSONObject(j.toString())))
        // Unknown provider strings read as OpenRouter.
        assertEquals(CleanupProvider.OPENROUTER, CloudSettings.fromJson(JSONObject(j.toString()).put("cleanup_provider", "other")).cleanupProvider)
    }

    @Test fun customConsentIsBoundToTheTypedHost() {
        val c = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom)
        assertFalse(c.cleanupConsentOk)
        val or = c.copy(cleanup = c.cleanup.withConsent(now))
        assertFalse("OpenRouter consent is not custom consent", or.cleanupConsentOk)
        val ok = c.copy(customCleanup = c.customCleanup.withConsent(now))
        assertTrue(ok.cleanupConsentOk); assertFalse("custom consent is not OpenRouter's", ok.cleanup.consentOk)
        assertTrue(ok.copy(customCleanup = ok.customCleanup.copy(baseUrl = "http://10.0.0.5:8000/api/v1")).cleanupConsentOk)
        assertFalse("another host voids it", ok.copy(customCleanup = ok.customCleanup.copy(baseUrl = "http://10.0.0.6:4000/v1")).cleanupConsentOk)
        assertTrue("host compared case-insensitively", CustomCleanupSettings("http://LLM.Lan:4000", "m", consentHost = "llm.lan", consentAtMs = 1).consentOk)
        assertFalse(CustomCleanupSettings("not a url", "m", consentHost = "", consentAtMs = 1).consentOk)
        assertEquals(CustomServer.ID, ok.cleanupKeyId); assertEquals(OpenRouter.ID, CloudSettings().cleanupKeyId)
        assertEquals("Qwen/Qwen2.5-7B-Instruct", ok.cleanupModel); assertEquals(4_000L, ok.cleanupTimeoutMs)
    }

    @Test fun addressComesFromTheRouterReader() {
        // The page parses no URLs: whatever the router's reader says is the URL, host and model check.
        val seen = mutableListOf<Pair<String, String>>()
        CustomServer.check = CustomAddressCheck { u, m -> seen += u to m; CustomAddress(url = "http://[2001:db8::1]:4000/v1", host = "2001:db8::1", modelOk = m == "ok") }
        try {
            assertEquals("http://[2001:db8::1]:4000/v1", CloudInput.baseUrl("http://[2001:DB8::0:1]:4000").value)
            assertEquals("2001:db8::1", CustomCleanupSettings("anything").host)
            assertEquals("2001:db8::1", CustomCleanupSettings("anything").withConsent(now).consentHost)
            assertTrue(CustomServer.validModelId("ok")); assertFalse(CustomServer.validModelId("two words"))
            assertTrue(seen.contains("http://[2001:DB8::0:1]:4000" to ""))
            CustomServer.check = CustomAddressCheck { _, _ -> CustomAddress(error = "bad_base_url", message = "base URL must not contain ? or #") }
            assertEquals("Base URL must not contain ? or #", CloudInput.baseUrl("http://x/v1?q").error)
            assertEquals("", CustomServer.host("http://x/v1?q"))
            assertTrue(CloudInput.baseUrl(" ").error!!.isNotEmpty())
        } finally { FakeCustomAddress.install() }
    }

    @Test fun oneAddressLookupServesReadyHostAndHttps() {
        // A page refresh reads ready, host, https and consent of one settings value: one native call.
        var calls = 0
        CustomServer.check = CustomAddressCheck { u, m -> calls++; FakeCustomAddress.check(u, m) }
        try {
            val c = CustomCleanupSettings("http://10.0.0.5:4000", " m ", consentHost = "10.0.0.5", consentAtMs = 1)
            assertTrue(c.ready); assertEquals("10.0.0.5", c.host); assertFalse(c.https); assertTrue(c.consentOk)
            assertTrue(CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = c).customNotSetUp.not())
            assertEquals(1, calls)
            // Another value (another address) is read afresh.
            assertTrue(c.copy(baseUrl = "https://llm.example").https)
            assertEquals(2, calls)
        } finally { FakeCustomAddress.install() }
    }

    @Test fun aLibraryWithoutTheAddressCallReadsAsNotSetUp() {
        // No native library in JVM tests: the real reader says "unusable", never a guess.
        CustomServer.check = CustomAddressCheck { u, m -> dev.murmur.core.NativeMurmurCore.customCleanupAddress(u, m) }
        try {
            assertNull(CustomServer.normalizeBaseUrl("http://10.0.0.5:4000"))
            val c = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom.withConsent(now))
            assertFalse(c.cleanupConsentOk); assertTrue(c.customNotSetUp)
            assertEquals(CustomServer.NOT_SET_UP, c.cleanupProviderName)
        } finally { FakeCustomAddress.install() }
    }

    @Test fun decodesTheAddressJson() {
        val ok = dev.murmur.core.RouterJson.decodeCustomAddress("""{"url":"http://h:4000/v1","host":"h","model_ok":true}""")
        assertEquals(CustomAddress("http://h:4000/v1", "h", true), ok); assertFalse(ok.https)
        val bad = dev.murmur.core.RouterJson.decodeCustomAddress("""{"error":"bad_base_url","message":"no","model_ok":false}""")
        assertNull(bad.url); assertEquals("", bad.host); assertEquals("bad_base_url", bad.error)
        assertEquals("error", dev.murmur.core.RouterJson.decodeCustomAddress("nope").error)
    }

    @Test fun modelsCallsPassOnlyATypedKey() {
        assertNull(CustomModelsKey.typedOnly(null)); assertNull(CustomModelsKey.typedOnly("   "))
        assertEquals("sk-typed", CustomModelsKey.typedOnly("  sk-typed "))
    }

    @Test fun customModelIdsAreFreeText() {
        assertEquals("Qwen/Qwen2.5-7B", CloudInput.customModel("  Qwen/Qwen2.5-7B ").value)
        assertTrue(CloudInput.customModel(" ").error!!.isNotEmpty())
        assertEquals(30_000L, CloudInput.customTimeout("30000").value)
        assertEquals(800L, CloudInput.customTimeout("800").value)
        assertTrue(CloudInput.customTimeout("799").error!!.contains("800-30000"))
        assertTrue(CloudInput.customTimeout("30001").error != null)
    }

    @Test fun customReadinessAndRouterJson() {
        assertFalse(CustomCleanupSettings().ready)
        assertFalse(custom.copy(model = "").ready); assertFalse(custom.copy(timeoutMs = 40_000).ready)
        assertTrue(custom.ready)
        val ready = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom)
        assertEquals(ready.toJson().toString(), ready.routerJson().toString())
        // An unusable server would make the router refuse the whole config: sent as OpenRouter without cleanup consent.
        val broken = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, cleanup = CleanupSettings().withConsent(now))
        val j = broken.routerJson()
        assertFalse(j.has("cleanup_provider"))
        assertEquals(0L, j.getJSONObject("openrouter").getJSONObject("cleanup").getLong("consent_at_ms"))
    }

    @Test fun customWordingNamesTheHost() {
        val c = CloudSettings(cleanupProvider = CleanupProvider.CUSTOM, customCleanup = custom)
        val (title, body, button) = CloudText.cleanupConsent(c)
        assertEquals("Send dictated text to 10.0.0.5?", title)
        assertTrue(body, body.contains("10.0.0.5") && body.contains(CloudText.HTTP_WARNING) && body.contains("AI cleanup only"))
        assertFalse(body, body.contains("openrouter.ai"))
        assertEquals(CloudText.CUSTOM_CONSENT_BUTTON, button)
        val tls = CloudText.cleanupConsent(c.copy(customCleanup = custom.copy(baseUrl = "https://10.0.0.5/v1"))).second
        assertFalse(tls.contains(CloudText.HTTP_WARNING))
        assertEquals(CloudText.CLEANUP_CONSENT_TITLE, CloudText.cleanupConsent(CloudSettings()).first)
        assertEquals("LiteLLM (10.0.0.5)", c.cleanupProviderName)
        assertEquals(CustomServer.NOT_SET_UP, CloudSettings(cleanupProvider = CleanupProvider.CUSTOM).cleanupProviderName)
        assertEquals("OpenRouter", CloudSettings().cleanupProviderName)
        assertEquals("Connected: 2 models", CloudText.customModelsLine(dev.murmur.core.CustomModels(listOf("a", "b"))))
        assertEquals("Failed: key rejected (Authentication Error)", CloudText.customModelsLine(dev.murmur.core.CustomModels(error = "auth_failed", message = "Authentication Error")))
        assertEquals("Failed: server not reachable", CloudText.customModelsLine(dev.murmur.core.CustomModels(error = "connect_failed")))
    }
}
